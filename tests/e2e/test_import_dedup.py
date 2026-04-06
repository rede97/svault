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


    def test_crc_collision_same_prefix_different_content(self, vault: VaultEnv) -> None:
        """CRC collision: Two files with same first 64KB but different content.
        
        JPEG files use CRC strategy Head(64KB), so if two files have identical
        first 64KB but differ afterwards, they will have same CRC but different
        strong hash (XXH3-128).
        
        Expected behavior:
        - Stage B (CRC): Both files have same CRC → marked as "Duplicate"
        - Stage D (Strong Hash): Different XXH3-128 → confirmed as different files
        - Both files imported with conflict resolution (photo.jpg, photo.1.jpg)
        """
        # Create two files with same JPEG header but different content after
        header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
        # Add padding to make files larger than 64KB to ensure collision
        padding = b'\x00' * (65 * 1024)  # 65KB padding
        
        f1 = vault.source_dir / "photo_a.jpg"
        f2 = vault.source_dir / "photo_b.jpg"
        
        # File 1: header + padding + "AAAA..."
        f1.write_bytes(header + padding + b'A' * 1000)
        # File 2: header + padding + "BBBB..." (same CRC header, different content)
        f2.write_bytes(header + padding + b'B' * 1000)
        
        # Both files should have same CRC (first 64KB identical)
        import zlib
        crc1 = zlib.crc32(header + padding[:65536 - len(header)])
        crc2 = zlib.crc32(header + padding[:65536 - len(header)])
        assert crc1 == crc2, "Test setup error: CRC should be identical"
        
        # Import both files
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        # Both files should be imported (not duplicates, different content)
        files = vault.db_files()
        assert len(files) == 2, f"Expected 2 files (different content), got {len(files)}"
        
        # Verify different strong hashes
        hashes = {f["xxh3_128"] for f in files}
        assert len(hashes) == 2, "Files should have different XXH3-128 hashes"


# ========== Tests migrated from test_import.py ==========

class TestDuplicateDetection:
    """Test duplicate file detection based on content hash.
    
    Migrated from test_import.py to consolidate deduplication tests.
    """
    
    def test_exact_duplicate_not_imported(self, vault: VaultEnv, source_factory: callable) -> None:
        """Exact byte-for-byte duplicate should not be imported twice."""
        source_factory(
            "original.jpg",
            exif_date="2024:05:01 10:00:00",
            exif_make="Test",
            exif_model="Camera",
        )
        
        vault.import_dir(vault.source_dir)
        assert_file_imported(vault, "original.jpg")
        
        # Create duplicate with different name
        original = vault.source_dir / "original.jpg"
        duplicate = vault.source_dir / "duplicate.jpg"
        import shutil
        shutil.copy2(original, duplicate)
        
        vault.import_dir(vault.source_dir)
        assert_file_duplicate(vault, "duplicate.jpg")
    
    @pytest.mark.parametrize("dup_count", [1, 3, 6])
    def test_multiple_duplicates(self, vault: VaultEnv, source_factory: callable, dup_count: int) -> None:
        """Test handling of multiple duplicates in batch."""
        source_factory(
            "original.jpg",
            exif_date="2024:05:01 10:00:00",
            exif_make="Test",
            exif_model="Camera",
        )
        
        vault.import_dir(vault.source_dir)
        
        original = vault.source_dir / "original.jpg"
        for i in range(dup_count):
            dup_path = vault.source_dir / f"duplicate_{i}.jpg"
            import shutil
            shutil.copy2(original, dup_path)
        
        vault.import_dir(vault.source_dir)
        
        for i in range(dup_count):
            assert_file_duplicate(vault, f"duplicate_{i}.jpg")
        
        files = vault.db_files()
        assert len(files) == 1
