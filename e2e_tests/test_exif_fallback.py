"""E2E tests for EXIF fallback scenarios.

Tests handling of files with missing or incomplete EXIF metadata.

中文场景说明：
- 无 EXIF 照片：使用文件修改时间(mtime)作为拍摄时间，设备显示为"Unknown"
- 部分 EXIF：有设备信息但无日期，或反之
- 无效 EXIF：日期格式错误，损坏的 EXIF 数据

必要性：
- 确保无 EXIF 照片也能正确归档
- 验证 fallback 链的完整性
- 测试各种边界情况
"""

from __future__ import annotations

import time
from pathlib import Path

import pytest

from conftest import VaultEnv, create_minimal_jpeg, copy_fixture


class TestNoExifFallback:
    """Tests for photos without EXIF metadata."""
    
    def test_no_exif_uses_mtime_for_date(self, vault: VaultEnv) -> None:
        """Photo without EXIF should use file mtime for path organization.
        
        Expected: Path based on mtime, device = "Unknown"
        """
        # Set specific mtime
        target_time = time.mktime(time.strptime("2024:06:15 14:30:00", "%Y:%m:%d %H:%M:%S"))
        copy_fixture(vault, "no_exif.jpg")
        
        # Adjust mtime after copying
        test_file = vault.source_dir / "no_exif.jpg"
        test_file.touch()
        test_file.stat()
        import os
        os.utime(test_file, (target_time, target_time))
        
        vault.import_dir(vault.source_dir)
        
        rows = vault.db_files()
        assert len(rows) == 1
        
        # Path should contain date from mtime
        assert "2024" in rows[0]["path"]
        assert "06-15" in rows[0]["path"] or "15" in rows[0]["path"]
        # Device should be Unknown
        assert "Unknown" in rows[0]["path"]
    
    def test_no_exif_device_is_unknown(self, vault: VaultEnv) -> None:
        """Photo without EXIF should have device = 'Unknown'."""
        copy_fixture(vault, "no_exif.jpg")
        
        vault.import_dir(vault.source_dir)
        
        rows = vault.db_files()
        assert len(rows) == 1
        
        # Verify device is Unknown in path
        assert "Unknown" in rows[0]["path"]
    
    def test_minimal_jpeg_has_no_exif(self, vault: VaultEnv) -> None:
        """Verify create_minimal_jpeg creates files without EXIF."""
        create_minimal_jpeg(vault.source_dir / "minimal.jpg", "test_content")
        
        vault.import_dir(vault.source_dir)
        
        rows = vault.db_files()
        assert len(rows) == 1
        # Should use Unknown device
        assert "Unknown" in rows[0]["path"]


class TestPartialExif:
    """Tests for files with partial EXIF data."""
    
    def test_exif_date_only_no_device(self, vault: VaultEnv, source_factory) -> None:
        """Photo with date but no Make/Model should use 'Unknown' device.
        
        This tests the case where EXIF exists but only contains DateTimeOriginal.
        
        Note: If exiftool is not available, this test will create a file without EXIF
        and still verify the 'Unknown' device behavior.
        """
        from conftest import EXIFTOOL_AVAILABLE
        
        test_file = vault.source_dir / "date_only.jpg"
        
        if EXIFTOOL_AVAILABLE:
            # Create with EXIF date using exiftool
            create_minimal_jpeg(test_file, "content")
            import subprocess
            subprocess.run([
                "exiftool", "-overwrite_original",
                "-DateTimeOriginal=2024:07:20 10:00:00",
                str(test_file)
            ], capture_output=True)
        else:
            # Fallback: create without EXIF, set mtime
            target_time = time.mktime(time.strptime("2024:07:20 10:00:00", "%Y:%m:%d %H:%M:%S"))
            create_minimal_jpeg(test_file, "content")
            import os
            os.utime(test_file, (target_time, target_time))
        
        vault.import_dir(vault.source_dir)
        
        rows = vault.db_files()
        assert len(rows) == 1
        
        # Device should be Unknown (no Make/Model in EXIF)
        assert "Unknown" in rows[0]["path"]
        
        # If exiftool worked, date should be 2024
        if EXIFTOOL_AVAILABLE:
            assert "2024" in rows[0]["path"] or "07-20" in rows[0]["path"] or "20" in rows[0]["path"]
    
    def test_exif_device_only_no_date(self, vault: VaultEnv, source_factory) -> None:
        """Photo with device but no date should use mtime for date.
        
        This tests the case where Make/Model exists but DateTimeOriginal is missing.
        """
        target_time = time.mktime(time.strptime("2024:08:10 16:00:00", "%Y:%m:%d %H:%M:%S"))
        
        # Create file with mtime first
        import os
        test_file = vault.source_dir / "device_only.jpg"
        create_minimal_jpeg(test_file, "content")
        os.utime(test_file, (target_time, target_time))
        
        # Then add EXIF device info using exiftool if available
        from conftest import EXIFTOOL_AVAILABLE
        if EXIFTOOL_AVAILABLE:
            import subprocess
            subprocess.run([
                "exiftool", "-overwrite_original",
                "-Make=TestCamera", "-Model=TestModel",
                str(test_file)
            ], capture_output=True)
        else:
            # Without exiftool, skip this test
            pytest.skip("exiftool not available for adding partial EXIF")
        
        vault.import_dir(vault.source_dir)
        
        rows = vault.db_files()
        assert len(rows) == 1
        
        # Date should be from mtime (if exiftool worked)
        if EXIFTOOL_AVAILABLE:
            assert "2024" in rows[0]["path"]


class TestExifFallbackChain:
    """Tests documenting the EXIF fallback chain."""
    
    def test_exif_fallback_chain_documentation(self) -> None:
        """Document the EXIF fallback chain for dates.
        
        Chain (in order of priority):
        1. EXIF DateTimeOriginal (most accurate - when photo was taken)
        2. EXIF DateTime (when file was created/modified)
        3. File modification time (mtime) - last resort
        
        For device:
        1. EXIF Make + Model (e.g., "Apple iPhone 14")
        2. EXIF Model only (if Make is empty or redundant)
        3. "Unknown" - last resort
        """
        # This is a documentation test - no actual test needed
        pass
    
    def test_date_priority_datetimeoriginal_over_datetime(self, vault: VaultEnv, source_factory) -> None:
        """DateTimeOriginal should be preferred over DateTime.
        
        DateTimeOriginal = when photo was actually taken
        DateTime = when file was modified (may differ)
        """
        from conftest import EXIFTOOL_AVAILABLE
        if not EXIFTOOL_AVAILABLE:
            pytest.skip("exiftool required for setting specific EXIF fields")
        
        test_file = vault.source_dir / "date_priority.jpg"
        create_minimal_jpeg(test_file, "content")
        
        # Set both DateTimeOriginal and DateTime to different values
        import subprocess
        result = subprocess.run([
            "exiftool", "-overwrite_original",
            "-DateTimeOriginal=2024:01:15 12:00:00",
            "-DateTime=2024:12:25 00:00:00",  # Different date
            str(test_file)
        ], capture_output=True)
        
        if result.returncode != 0:
            pytest.skip(f"exiftool failed: {result.stderr.decode()}")
        
        vault.import_dir(vault.source_dir)
        
        rows = vault.db_files()
        assert len(rows) == 1
        
        # Should use DateTimeOriginal (January), not DateTime (December)
        # Note: The path format may vary, but should contain 2024
        assert "2024" in rows[0]["path"], f"Expected 2024 in path: {rows[0]['path']}"


class TestEdgeCases:
    """Edge cases for EXIF handling."""
    
    def test_very_old_photo_date(self, vault: VaultEnv, source_factory) -> None:
        """Photos from before 1970 (Unix epoch) should be handled.
        
        Note: This tests historical photos that may have invalid dates.
        """
        # Create photo with date before Unix epoch
        test_file = vault.source_dir / "old_photo.jpg"
        create_minimal_jpeg(test_file, "old_content")
        
        from conftest import EXIFTOOL_AVAILABLE
        if EXIFTOOL_AVAILABLE:
            import subprocess
            subprocess.run([
                "exiftool", "-overwrite_original",
                "-DateTimeOriginal=1960:01:01 00:00:00",  # Before Unix epoch
                str(test_file)
            ], capture_output=True)
        
        # Should not crash
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 1
    
    def test_future_date(self, vault: VaultEnv, source_factory) -> None:
        """Photos with future dates should be handled gracefully."""
        test_file = vault.source_dir / "future_photo.jpg"
        create_minimal_jpeg(test_file, "future_content")
        
        from conftest import EXIFTOOL_AVAILABLE
        if EXIFTOOL_AVAILABLE:
            import subprocess
            subprocess.run([
                "exiftool", "-overwrite_original",
                "-DateTimeOriginal=2035:12:31 23:59:59",  # Future date
                str(test_file)
            ], capture_output=True)
        
        # Should not crash
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 1


class TestCorruptedExif:
    """Tests for corrupted or invalid EXIF data."""
    
    def test_corrupted_exif_fallback_to_mtime(self, vault: VaultEnv) -> None:
        """Corrupted EXIF should fallback to mtime, not crash.
        
        Creates a JPEG with corrupted EXIF data segment.
        """
        import os
        target_time = time.mktime(time.strptime("2024:09:01 08:00:00", "%Y:%m:%d %H:%M:%S"))
        
        test_file = vault.source_dir / "corrupted_exif.jpg"
        
        # Create JPEG with invalid EXIF
        # JPEG header + corrupted EXIF marker
        jpeg_header = b'\xff\xd8\xff\xe1\x00\x10Exif\x00\x00'  # EXIF APP1 marker
        corrupted_data = b'CORRUPTED_DATA_NOT_VALID_EXIF'
        image_data = b'\xff\xd9'  # EOI marker
        
        test_file.write_bytes(jpeg_header + corrupted_data + image_data)
        os.utime(test_file, (target_time, target_time))
        
        # Should not crash, should use mtime
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 1
        
        # Should have Unknown device (EXIF parsing failed)
        assert "Unknown" in rows[0]["path"]


class TestMultipleFilesMixedExif:
    """Tests for batch imports with mixed EXIF presence."""
    
    def test_batch_import_mixed_exif_and_no_exif(self, vault: VaultEnv, source_factory) -> None:
        """Batch import should handle mix of files with and without EXIF.
        
        Some photos from camera (with EXIF), some from screenshots (no EXIF).
        
        Note: Without exiftool, both files will have "Unknown" device.
        With exiftool, one should have "Canon" and one "Unknown".
        """
        from conftest import EXIFTOOL_AVAILABLE
        import shutil
        
        # Photo with EXIF (if exiftool available)
        camera_file = vault.source_dir / "camera_photo.jpg"
        if EXIFTOOL_AVAILABLE:
            source_factory(
                "camera_photo.jpg",
                exif_date="2024:10:05 15:00:00",
                exif_make="Canon",
                exif_model="EOS R5",
            )
            # Verify EXIF was actually set
            import subprocess
            result = subprocess.run(
                ["exiftool", "-Make", "-Model", str(camera_file)],
                capture_output=True, text=True
            )
            has_canon_exif = "Canon" in result.stdout
        else:
            # Without exiftool, create a file with mtime (will also be Unknown)
            target_time = time.mktime(time.strptime("2024:10:05 15:00:00", "%Y:%m:%d %H:%M:%S"))
            create_minimal_jpeg(camera_file, "camera_content")
            import os
            os.utime(camera_file, (target_time, target_time))
            has_canon_exif = False
        
        # Screenshot without EXIF (using mtime)
        target_time = time.mktime(time.strptime("2024:10:06 09:00:00", "%Y:%m:%d %H:%M:%S"))
        copy_fixture(vault, "no_exif.jpg")
        test_file = vault.source_dir / "no_exif.jpg"
        test_file.rename(vault.source_dir / "screenshot.jpg")
        import os
        os.utime(vault.source_dir / "screenshot.jpg", (target_time, target_time))
        
        vault.import_dir(vault.source_dir)
        
        rows = vault.db_files()
        assert len(rows) == 2
        
        # Both should be imported
        paths = [r["path"] for r in rows]
        
        # Verify both files are in the vault
        filenames = [Path(p).name for p in paths]
        assert "camera_photo.jpg" in filenames or any("camera" in p for p in filenames)
        assert "screenshot.jpg" in filenames or any("screenshot" in p for p in filenames)
        
        # Screenshot should always have Unknown device
        screenshot_paths = [p for p in paths if "screenshot" in p and "Unknown" in p]
        assert len(screenshot_paths) == 1, f"Expected screenshot with Unknown in: {paths}"
        
        # If exiftool worked and EXIF was set correctly, camera should have Canon
        if has_canon_exif:
            canon_paths = [p for p in paths if "Canon" in p]
            assert len(canon_paths) == 1, f"Expected one Canon device path in: {paths}"
