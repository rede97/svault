"""Basic import functionality tests.

This module tests svault's core import functionality for normal use cases.

中文场景说明：
- 标准 EXIF 导入：用户从 iPhone/相机导入带完整元数据的照片（90%场景）
- 无设备信息：某些经过编辑或老照片丢失设备信息
- 无 EXIF：截图、扫描件等没有拍摄元数据的文件
- Samsung 设备：测试 Android 设备的特殊处理
- 重复检测：用户多次导入同一批照片，避免存储浪费

Coverage of old test_rules.json scenarios:
- s1_normal_apple: test_import_with_exif_date_and_device
- s2_no_device: test_import_no_device
- s3_no_exif: test_import_no_exif_uses_mtime
- s4_duplicate*: test_exact_duplicate_not_imported, test_multiple_duplicates
- s5_samsung: test_import_samsung_device
- s6_make_in_model: test_import_avoids_redundant_make
"""

from __future__ import annotations

import time
from pathlib import Path

import pytest

from conftest import (
    VaultEnv,
    assert_file_duplicate,
    assert_file_imported,
    assert_path_contains,
    copy_fixture,
    create_minimal_jpeg,
)


class TestNormalImport:
    """Test normal import scenarios with various EXIF conditions."""
    
    def test_import_with_exif_date_and_device(self, vault: VaultEnv, source_factory: callable) -> None:
        """Import file with EXIF date and Apple device info.
        
        Expected: File imported to $year/$mon-$day/$device/$filename
        """
        source_factory(
            "apple_test.jpg",
            exif_date="2024:05:01 10:30:00",
            exif_make="Apple",
            exif_model="iPhone 15",
        )
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        row = assert_file_imported(vault, "apple_test.jpg")
        assert_path_contains(
            row["path"],
            "2024",
            "05-01",
            "Apple iPhone 15",
            "apple_test.jpg",
        )
        assert row["crc32c_val"] is not None
    
    def test_import_no_device(self, vault: VaultEnv, source_factory: callable) -> None:
        """Import file with EXIF date but no Make/Model.
        
        Expected: device=Unknown
        """
        source_factory(
            "no_device.jpg",
            exif_date="2024:05:01 18:00:00",
        )
        
        vault.import_dir(vault.source_dir)
        
        row = assert_file_imported(vault, "no_device.jpg")
        assert_path_contains(row["path"], "2024", "05-01", "Unknown")
    
    def test_import_no_exif_uses_mtime(self, vault: VaultEnv, source_factory: callable) -> None:
        """Import file without EXIF - should use mtime fallback.
        
        Expected: Path derived from file modification time
        """
        # Set mtime to 2024-03-15
        target_ts = time.mktime(time.strptime("2024:03:15 08:00:00", "%Y:%m:%d %H:%M:%S"))
        source_factory("no_exif.jpg", mtime=target_ts)
        
        vault.import_dir(vault.source_dir)
        
        row = assert_file_imported(vault, "no_exif.jpg")
        assert_path_contains(row["path"], "2024", "03-15", "Unknown")
    
    def test_import_samsung_device(self, vault: VaultEnv, source_factory: callable) -> None:
        """Import Samsung device photo.
        
        Expected: Model already starts with 'Samsung', no duplication
        """
        source_factory(
            "samsung.jpg",
            exif_date="2024:05:02 14:20:00",
            exif_make="Samsung",
            exif_model="Galaxy S24",
        )
        
        vault.import_dir(vault.source_dir)
        
        row = assert_file_imported(vault, "samsung.jpg")
        assert_path_contains(row["path"], "Samsung")
        # Should not have "Samsung Samsung"
        assert "Samsung Samsung" not in row["path"]
    
    def test_import_avoids_redundant_make(self, vault: VaultEnv, source_factory: callable) -> None:
        """Model starting with Make should not duplicate Make name.
        
        Expected: "Apple iPhone 14" not "Apple Apple iPhone 14"
        """
        source_factory(
            "apple_redundant.jpg",
            exif_date="2024:05:02 09:00:00",
            exif_make="Apple",
            exif_model="Apple iPhone 14",
        )
        
        vault.import_dir(vault.source_dir)
        
        row = assert_file_imported(vault, "apple_redundant.jpg")
        assert "Apple iPhone 14" in row["path"]
        assert "Apple Apple" not in row["path"]


class TestDuplicateDetection:
    """Test duplicate file detection based on content hash."""
    
    def test_exact_duplicate_not_imported(self, vault: VaultEnv, source_factory: callable) -> None:
        """Exact byte-for-byte duplicate should not be imported twice.
        
        Scenario:
        1. Import original file
        2. Import duplicate (same content, different name)
        3. Duplicate should be detected and skipped
        """
        # Create original
        source_factory(
            "original.jpg",
            exif_date="2024:05:01 10:00:00",
            exif_make="Test",
            exif_model="Camera",
        )
        
        # First import
        vault.import_dir(vault.source_dir)
        assert_file_imported(vault, "original.jpg")
        
        # Create duplicate with different name
        original = vault.source_dir / "original.jpg"
        duplicate = vault.source_dir / "duplicate.jpg"
        import shutil
        shutil.copy2(original, duplicate)
        
        # Second import
        result = vault.import_dir(vault.source_dir)
        
        # Duplicate should not be in DB
        assert_file_duplicate(vault, "duplicate.jpg")
    
    @pytest.mark.parametrize("dup_count", [1, 3, 6])
    def test_multiple_duplicates(self, vault: VaultEnv, source_factory: callable, dup_count: int) -> None:
        """Test handling of multiple duplicates in batch."""
        # Create original
        source_factory(
            "original.jpg",
            exif_date="2024:05:01 10:00:00",
            exif_make="Test",
            exif_model="Camera",
        )
        
        vault.import_dir(vault.source_dir)
        
        # Create multiple duplicates
        original = vault.source_dir / "original.jpg"
        for i in range(dup_count):
            dup_path = vault.source_dir / f"duplicate_{i}.jpg"
            import shutil
            shutil.copy2(original, dup_path)
        
        # Import duplicates
        vault.import_dir(vault.source_dir)
        
        # None of the duplicates should be in DB
        for i in range(dup_count):
            assert_file_duplicate(vault, f"duplicate_{i}.jpg")
        
        # Only original should be in DB
        files = vault.db_files()
        assert len(files) == 1


class TestExistingFixtures:
    """Tests using pre-generated fixture files."""
    
    def test_fixture_apple_with_exif(self, vault: VaultEnv) -> None:
        """Test with pre-generated apple_with_exif.jpg fixture."""
        copy_fixture(vault, "apple_with_exif.jpg")
        
        vault.import_dir(vault.source_dir)
        
        row = assert_file_imported(vault, "apple_with_exif.jpg")
        assert_path_contains(row["path"], "2024", "05-01", "Apple iPhone 15")
    
    def test_fixture_samsung(self, vault: VaultEnv) -> None:
        """Test with pre-generated samsung_photo.jpg fixture."""
        copy_fixture(vault, "samsung_photo.jpg")
        
        vault.import_dir(vault.source_dir)
        
        row = assert_file_imported(vault, "samsung_photo.jpg")
        assert "Samsung" in row["path"]
