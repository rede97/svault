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

import shutil
import subprocess
import tempfile
from pathlib import Path

import pytest
from hypothesis import given, settings, strategies as st

from conftest import create_minimal_jpeg


def _create_vault_env(tmp_path: Path) -> tuple[Path, Path, Path]:
    """Create minimal vault environment for property tests.
    
    Returns (vault_dir, source_dir, binary_path)
    """
    binary = Path(__file__).parent.parent / "target" / "release" / "svault"
    vault_dir = tmp_path / "vault"
    source_dir = tmp_path / "source"
    
    vault_dir.mkdir()
    source_dir.mkdir()
    
    # Init vault
    subprocess.run(
        [str(binary), "init"],
        cwd=str(vault_dir),
        check=True,
        capture_output=True,
    )
    
    return vault_dir, source_dir, binary


def _import_dir(vault_dir: Path, source_dir: Path, binary: Path) -> None:
    """Run import command."""
    subprocess.run(
        [str(binary), "--output", "json", "import", "--yes", str(source_dir.resolve())],
        cwd=str(vault_dir),
        check=True,
        capture_output=True,
    )


def _db_files(vault_dir: Path) -> list[dict]:
    """Query files from database."""
    import sqlite3
    db_path = vault_dir / ".svault" / "vault.db"
    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    cur = conn.execute(
        "SELECT path, size, mtime, crc32c_val, xxh3_128, sha256, status, imported_at FROM files"
    )
    rows = [dict(r) for r in cur.fetchall()]
    conn.close()
    return rows


# =============================================================================
# Property Tests
# =============================================================================

@pytest.mark.property
class TestHashProperties:
    """Properties about hash computation and deduplication."""
    
    @given(st.binary(min_size=1, max_size=10000))
    @settings(max_examples=20, deadline=30000)
    def test_same_content_same_hash(self, content: bytes) -> None:
        """Same content should always produce same database state."""
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            vault_dir, source_dir, binary = _create_vault_env(tmp_path)
            
            # Create two files with identical content
            header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
            full_content = header + content[:5000]  # Limit size
            
            f1 = source_dir / "file1.jpg"
            f2 = source_dir / "file2.jpg"
            f1.write_bytes(full_content)
            f2.write_bytes(full_content)
            
            _import_dir(vault_dir, source_dir, binary)
            
            files = _db_files(vault_dir)
            # Only one should be in DB (the other is duplicate)
            assert len(files) == 1
            assert files[0]["status"] == "imported"
    
    @given(st.binary(min_size=10, max_size=1000), st.binary(min_size=10, max_size=1000))
    @settings(max_examples=20, deadline=30000)
    def test_different_content_different_entry(self, content1: bytes, content2: bytes) -> None:
        """Different content should result in different DB entries."""
        if content1 == content2:
            return  # Skip if same
        
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            vault_dir, source_dir, binary = _create_vault_env(tmp_path)
            
            header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
            
            f1 = source_dir / "file1.jpg"
            f2 = source_dir / "file2.jpg"
            f1.write_bytes(header + content1)
            f2.write_bytes(header + content2)
            
            _import_dir(vault_dir, source_dir, binary)
            
            files = _db_files(vault_dir)
            # Should have 2 entries (collision rename, not duplicate)
            assert len(files) == 2


@pytest.mark.property
class TestFileCountProperties:
    """Properties about file counts."""
    
    @given(st.integers(min_value=1, max_value=20))
    @settings(max_examples=10, deadline=60000)
    def test_n_unique_files_n_db_rows(self, n: int) -> None:
        """N unique files should result in N DB rows."""
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            vault_dir, source_dir, binary = _create_vault_env(tmp_path)
            
            header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
            
            for i in range(n):
                f = source_dir / f"file_{i}.jpg"
                f.write_bytes(header + f"unique_{i}".encode())
            
            _import_dir(vault_dir, source_dir, binary)
            
            files = _db_files(vault_dir)
            assert len(files) == n


@pytest.mark.property
class TestFilenameProperties:
    """Properties about filename handling."""
    
    @given(st.text(min_size=1, max_size=50, alphabet="abcdefghijklmnopqrstuvwxyz0123456789_-"))
    @settings(max_examples=30, deadline=10000)
    def test_any_valid_filename_can_be_imported(self, filename: str) -> None:
        """Any valid filename should be importable without crashing."""
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            vault_dir, source_dir, binary = _create_vault_env(tmp_path)
            
            # Ensure .jpg extension
            if not filename.endswith('.jpg'):
                filename += '.jpg'
            
            f = source_dir / filename
            create_minimal_jpeg(f, f"content_for_{filename}")
            
            # Should not crash
            result = subprocess.run(
                [str(binary), "import", "--yes", str(source_dir.resolve())],
                cwd=str(vault_dir),
                capture_output=True,
            )
            # 0 = success, 1 = some files failed (both acceptable)
            assert result.returncode in [0, 1]
