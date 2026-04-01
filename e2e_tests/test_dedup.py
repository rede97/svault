"""Deduplication tests.

Tests the three-layer deduplication system:
1. CRC32C (fast, check first/last 64KB)
2. XXH3-128 (strong hash of full file)
3. SHA-256 (cryptographic verification)

中文场景说明：
- 重复导入：用户多次导入同一批照片（如从相机和云备份分别导入）
- 重命名后导入：用户重命名文件后再次导入
- 跨目录重复：同一文件存在于多个子目录中

必要性：
- 节省存储空间（避免保存多份相同内容）
- 保持数据库整洁（无重复记录）
- 快速检测（CRC32C 避免计算完整哈希）
"""

from __future__ import annotations

import shutil
from pathlib import Path

import pytest

from conftest import VaultEnv, assert_file_duplicate, assert_file_imported


@pytest.mark.dedup
class TestDeduplication:
    """Test deduplication at various levels."""
    
    def test_same_file_imported_twice(self, vault: VaultEnv) -> None:
        """Importing same file twice should detect duplicates."""
        # Create and import first time
        test_file = vault.source_dir / "test.jpg"
        header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
        test_file.write_bytes(header + b'unique_content_12345')
        
        vault.import_dir(vault.source_dir)
        assert_file_imported(vault, "test.jpg")
        
        # Import same directory again
        result = vault.import_dir(vault.source_dir)
        
        # Should still only have one file in DB
        files = vault.db_files()
        assert len(files) == 1
        assert files[0]["status"] == "imported"
    
    def test_renamed_file_detected_as_duplicate(self, vault: VaultEnv) -> None:
        """File with different name but same content should be duplicate."""
        # Create original
        original = vault.source_dir / "original.jpg"
        header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
        content = header + b'same_content_across_files'
        original.write_bytes(content)
        
        # Import original
        vault.import_dir(vault.source_dir)
        assert_file_imported(vault, "original.jpg")
        
        # Create renamed copy
        renamed = vault.source_dir / "renamed.jpg"
        shutil.copy2(original, renamed)
        original.unlink()  # Remove original from source
        
        # Import renamed
        vault.import_dir(vault.source_dir)
        
        # Should be detected as duplicate
        assert_file_duplicate(vault, "renamed.jpg")
    
    def test_different_content_same_name(self, vault: VaultEnv) -> None:
        """Files with same name but different content are conflicts, not duplicates."""
        # Create two files with same name in different subdirs
        for subdir in ["cam1", "cam2"]:
            (vault.source_dir / subdir).mkdir(exist_ok=True)
            f = vault.source_dir / subdir / "IMG_0001.jpg"
            header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
            # Different content for each
            f.write_bytes(header + f"content_from_{subdir}".encode())
        
        vault.import_dir(vault.source_dir)
        
        # Both should be imported (one renamed)
        files = vault.db_files()
        assert len(files) == 2
        
        # Both should have status "imported"
        for f in files:
            assert f["status"] == "imported"


class TestBatchDeduplication:
    """Test deduplication within a single batch."""
    
    def test_multiple_duplicates_in_same_batch(self, vault: VaultEnv) -> None:
        """Multiple copies of same file in one import batch."""
        # Create original and 5 duplicates in same batch
        header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
        content = header + b'batch_duplicate_test'
        
        for i in range(6):
            f = vault.source_dir / f"file_{i}.jpg"
            f.write_bytes(content)
        
        vault.import_dir(vault.source_dir)
        
        # Only one should be imported
        files = vault.db_files()
        assert len(files) == 1
        assert files[0]["status"] == "imported"
    
    def test_cross_directory_duplicates(self, vault: VaultEnv) -> None:
        """Duplicates scattered across subdirectories."""
        header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
        content = header + b'cross_dir_test'
        
        # Create same file in multiple subdirs
        for subdir in ["day1", "day2", "day3", "backup"]:
            (vault.source_dir / subdir).mkdir(exist_ok=True)
            f = vault.source_dir / subdir / "photo.jpg"
            f.write_bytes(content)
        
        vault.import_dir(vault.source_dir)
        
        # Only one should be imported
        files = vault.db_files()
        assert len(files) == 1
