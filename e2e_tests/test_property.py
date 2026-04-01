"""Property-based tests using Hypothesis.

These tests verify invariants and properties that should always hold true,
regardless of input data. They complement the example-based tests by
exploring a wider range of inputs.

中文场景说明：
- 边界值探索：使用 Hypothesis 生成大量随机输入，发现边界情况
- 属性验证：验证 "相同内容必产生相同哈希" 等不变量
- 健壮性测试：各种奇怪的文件名、内容大小是否能正常处理

必要性：
- 发现人工想不到的边界情况
- 验证系统不变量（invariants）
- 补充示例测试的不足（example-based 测试只能覆盖已知场景）
- 作为回归测试，捕获意外行为变化

Note: Property tests are slower and may be skipped in CI with -m "not property"
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest
from hypothesis import HealthCheck, given, settings, strategies as st

from conftest import VaultEnv, create_minimal_jpeg


# =============================================================================
# Strategies
# =============================================================================

def valid_filename_strategy() -> st.SearchStrategy[str]:
    """Generate valid filenames (not empty, no null bytes, reasonable length)."""
    return st.text(
        min_size=1,
        max_size=200,
        alphabet=st.characters(
            whitelist_categories=('L', 'N'),  # Letters and numbers
            whitelist_characters='_-.'
        )
    ).filter(lambda x: x.strip() and not x.startswith('.'))


def exif_date_strategy() -> st.SearchStrategy[str]:
    """Generate valid EXIF date strings."""
    return st.dates(min_value=__import__('datetime').date(2000, 1, 1)).map(
        lambda d: d.strftime("%Y:%m:%d %H:%M:%S")
    )


def device_name_strategy() -> st.SearchStrategy[str]:
    """Generate realistic camera device names."""
    makes = ["Apple", "Sony", "Canon", "Nikon", "Samsung", "Fujifilm", "Panasonic", ""]
    models = ["iPhone 15", "A7IV", "EOS R5", "Z8", "Galaxy S24", "X-T5", "GH6", "Unknown"]
    
    return st.one_of(
        st.sampled_from([f"{m} {mo}" for m in makes if m for mo in models]),
        st.text(min_size=1, max_size=50).filter(lambda x: x.strip())
    )


# =============================================================================
# Property Tests
# =============================================================================

@pytest.mark.property
class TestPathProperties:
    """Properties about vault path generation."""
    
    @given(valid_filename_strategy())
    @settings(
        max_examples=50,
        deadline=10000,
        suppress_health_check=[HealthCheck.function_scoped_fixture],
    )
    def test_any_valid_filename_can_be_imported(self, vault: VaultEnv, filename: str) -> None:
        """Any valid filename should be importable without crashing."""
        # Ensure filename has .jpg extension for our test
        if not filename.endswith('.jpg'):
            filename += '.jpg'
        
        # Sanitize: remove path separators
        filename = filename.replace('/', '_').replace('\\', '_')
        
        f = vault.source_dir / filename
        create_minimal_jpeg(f, f"content_for_{filename}")
        
        # Should not crash
        result = vault.import_dir(vault.source_dir, check=False)
        assert result.returncode in [0, 1]  # 0 = all success, 1 = some failed
    
    @given(exif_date_strategy())
    @settings(
        max_examples=30,
        deadline=10000,
        suppress_health_check=[HealthCheck.function_scoped_fixture],
    )
    def test_exif_date_parsed_correctly(self, vault: VaultEnv, date_str: str) -> None:
        """EXIF dates should be parsed and reflected in path."""
        f = vault.source_dir / "dated.jpg"
        create_minimal_jpeg(f)
        
        # Use exiftool to set date
        import subprocess
        subprocess.run(
            ["exiftool", "-overwrite_original", f"-DateTimeOriginal={date_str}", str(f)],
            check=False,
            capture_output=True,
        )
        
        vault.import_dir(vault.source_dir, check=False)
        
        # If imported, check path contains year
        year = date_str[:4]
        files = vault.db_files()
        if files:
            assert year in files[0].get("path", "") or files[0].get("status") != "imported"


@pytest.mark.property
class TestHashProperties:
    """Properties about hash computation and deduplication."""
    
    @given(st.binary(min_size=1, max_size=1024*1024))  # Up to 1MB
    @settings(
        max_examples=20,
        deadline=30000,
        suppress_health_check=[HealthCheck.function_scoped_fixture],
    )
    def test_same_content_same_hash(self, vault: VaultEnv, content: bytes) -> None:
        """Same content should always produce same database state."""
        # Skip empty content
        if not content:
            return
        
        # Create two files with identical content
        header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
        full_content = header + content[:100000]  # Limit size
        
        f1 = vault.source_dir / "file1.jpg"
        f2 = vault.source_dir / "file2.jpg"
        f1.write_bytes(full_content)
        f2.write_bytes(full_content)
        
        vault.import_dir(vault.source_dir)
        
        # Only one should be in DB (the other is duplicate)
        files = vault.db_files()
        assert len(files) == 1
        assert files[0]["status"] == "imported"
    
    @given(st.binary(min_size=10, max_size=1000), st.binary(min_size=10, max_size=1000))
    @settings(
        max_examples=20,
        deadline=30000,
        suppress_health_check=[HealthCheck.function_scoped_fixture],
    )
    def test_different_content_different_entry(
        self, vault: VaultEnv, content1: bytes, content2: bytes
    ) -> None:
        """Different content should result in different DB entries.
        
        Note: Very unlikely collision is statistically negligible for this test.
        """
        if content1 == content2:
            return  # Skip if Hypothesis happens to generate same content
        
        header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
        
        f1 = vault.source_dir / "file1.jpg"
        f2 = vault.source_dir / "file2.jpg"
        f1.write_bytes(header + content1)
        f2.write_bytes(header + content2)
        
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        # Should have 2 entries (collision rename, not duplicate)
        assert len(files) == 2


@pytest.mark.property
class TestDatabaseProperties:
    """Properties about database state."""
    
    def test_db_row_has_required_fields(self, vault: VaultEnv) -> None:
        """Every imported file should have required database fields."""
        f = vault.source_dir / "test.jpg"
        create_minimal_jpeg(f, "test_content")
        
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        assert len(files) > 0
        
        required_fields = ["path", "size", "mtime", "status", "imported_at"]
        for row in files:
            for field in required_fields:
                assert field in row, f"Missing required field: {field}"
            
            # Status should be valid
            assert row["status"] in ["imported", "duplicate", "failed"]
    
    @given(st.integers(min_value=1, max_value=50))
    @settings(
        max_examples=10,
        deadline=60000,
        suppress_health_check=[HealthCheck.function_scoped_fixture],
    )
    def test_n_unique_files_n_db_rows(self, vault: VaultEnv, n: int) -> None:
        """N unique files should result in N DB rows."""
        header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
        
        for i in range(n):
            f = vault.source_dir / f"file_{i}.jpg"
            f.write_bytes(header + f"unique_{i}".encode())
        
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        assert len(files) == n
