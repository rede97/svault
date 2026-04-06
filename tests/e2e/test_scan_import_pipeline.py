"""Tests for scan + filter + import pipeline workflow.

中文说明：
测试 "扫描 → 过滤 → 导入" 的完整工作流程：
1. 扫描源目录中的媒体文件
2. 使用各种条件过滤（文件名、日期、大小等）
3. 将过滤后的文件导入到 vault

此测试验证整个流程是否能够正常工作。
"""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

import pytest

from conftest import (
    VaultEnv,
    assert_file_imported,
    create_minimal_jpeg,
    create_minimal_mp4,
    create_minimal_raw,
    EXIFTOOL_AVAILABLE,
)


class TestScanFilterImportPipeline:
    """Test the complete scan -> filter -> import workflow."""

    def test_scan_filter_by_extension_import(self, vault: VaultEnv) -> None:
        """Scan files, filter by extension, then import matching files.
        
        Workflow:
        1. Create mixed files (jpg, png, mp4, dng)
        2. Use find + grep to filter only jpg files
        3. Copy filtered files to staging dir
        4. Import staging dir to vault
        5. Verify only jpg files were imported
        """
        # Create mixed file types
        files = {
            "photo1.jpg": "jpg_content_1",
            "photo2.jpg": "jpg_content_2",
            "screenshot.png": "png_content",
            "video.mp4": "mp4_content",
            "raw.dng": "dng_content",
        }
        
        for filename, content in files.items():
            filepath = vault.source_dir / filename
            if filename.endswith(".jpg"):
                create_minimal_jpeg(filepath, content)
            elif filename.endswith(".png"):
                # Create a minimal file that looks like PNG
                filepath.write_bytes(b"\x89PNG\r\n\x1a\n" + content.encode())
            elif filename.endswith(".mp4"):
                create_minimal_mp4(filepath)
            elif filename.endswith(".dng"):
                create_minimal_raw(filepath)
        
        # Simulate: find + filter by extension (jpg only)
        # In real usage, this could be: find . -name "*.jpg" | ...
        staging_dir = vault.output_dir / "staging"
        staging_dir.mkdir()
        
        # Filter: only copy .jpg files to staging
        for filepath in vault.source_dir.glob("*.jpg"):
            import shutil
            shutil.copy2(filepath, staging_dir / filepath.name)
        
        # Import the filtered staging directory
        result = vault.import_dir(staging_dir)
        assert result.returncode == 0
        
        data = json.loads(result.stdout)
        assert data["imported"] == 2, f"Expected 2 JPG files imported, got {data}"
        
        # Verify database contains only jpg files
        rows = vault.db_files()
        assert len(rows) == 2
        paths = [Path(r["path"]).name for r in rows]
        assert "photo1.jpg" in paths
        assert "photo2.jpg" in paths

    def test_scan_filter_by_date_pattern_import(self, vault: VaultEnv) -> None:
        """Scan files, filter by date in filename, then import.
        
        Common scenario: Camera files named IMG_20240501_123456.jpg
        User wants to import only photos from a specific date.
        """
        if not EXIFTOOL_AVAILABLE:
            pytest.skip("exiftool not available")
        
        # Create files with date-stamped names and EXIF
        date_files = [
            ("IMG_20240501_101010.jpg", "2024:05:01 10:10:10"),
            ("IMG_20240501_102030.jpg", "2024:05:01 10:20:30"),
            ("IMG_20240515_080000.jpg", "2024:05:15 08:00:00"),
            ("IMG_20240601_120000.jpg", "2024:06:01 12:00:00"),
        ]
        
        for filename, exif_date in date_files:
            filepath = vault.source_dir / filename
            create_minimal_jpeg(filepath, f"content_{filename}")
            # Add EXIF date
            subprocess.run([
                "exiftool", "-overwrite_original",
                f"-DateTimeOriginal={exif_date}",
                str(filepath)
            ], check=True, capture_output=True)
        
        # Filter: only files from 2024-05-01 (by filename pattern)
        staging_dir = vault.output_dir / "staging_may1"
        staging_dir.mkdir()
        
        for filepath in vault.source_dir.glob("IMG_20240501_*.jpg"):
            import shutil
            shutil.copy2(filepath, staging_dir / filepath.name)
        
        result = vault.import_dir(staging_dir)
        assert result.returncode == 0
        
        data = json.loads(result.stdout)
        assert data["imported"] == 2, f"Expected 2 files from May 1, got {data}"
        
        # Verify
        rows = vault.db_files()
        assert len(rows) == 2

    def test_scan_filter_by_file_size_import(self, vault: VaultEnv) -> None:
        """Scan files, filter by size (e.g., skip small thumbnails), then import."""
        # Create files of different sizes
        # Large photo (simulated)
        large_file = vault.source_dir / "large_photo.jpg"
        create_minimal_jpeg(large_file, "x" * 1000)  # Larger content
        # Append more data to make it bigger
        with open(large_file, "ab") as f:
            f.write(b"A" * 10000)
        
        # Small thumbnail (simulated)
        small_file = vault.source_dir / "thumb.jpg"
        create_minimal_jpeg(small_file, "small")
        
        # Filter: only files > 5KB
        size_threshold = 5 * 1024
        staging_dir = vault.output_dir / "staging_large"
        staging_dir.mkdir()
        
        import shutil
        for filepath in vault.source_dir.glob("*.jpg"):
            if filepath.stat().st_size > size_threshold:
                shutil.copy2(filepath, staging_dir / filepath.name)
        
        result = vault.import_dir(staging_dir)
        assert result.returncode == 0
        
        data = json.loads(result.stdout)
        assert data["imported"] == 1, f"Expected 1 large file, got {data}"
        
        # Verify only large file was imported
        rows = vault.db_files()
        assert len(rows) == 1
        assert "large_photo" in rows[0]["path"]

    def test_scan_filter_by_mtime_recent_import(self, vault: VaultEnv) -> None:
        """Scan files, filter by modification time (only recent), then import.
        
        Common scenario: Import only photos taken in the last 7 days.
        """
        now = time.time()
        day_secs = 24 * 3600
        
        # Create files with different mtimes
        recent_file = vault.source_dir / "recent.jpg"
        create_minimal_jpeg(recent_file, "recent")
        os.utime(recent_file, (now - 1 * day_secs, now - 1 * day_secs))
        
        old_file = vault.source_dir / "old.jpg"
        create_minimal_jpeg(old_file, "old")
        os.utime(old_file, (now - 30 * day_secs, now - 30 * day_secs))
        
        very_old_file = vault.source_dir / "very_old.jpg"
        create_minimal_jpeg(very_old_file, "very_old")
        os.utime(very_old_file, (now - 100 * day_secs, now - 100 * day_secs))
        
        # Filter: only files modified within last 7 days
        cutoff_time = now - 7 * day_secs
        staging_dir = vault.output_dir / "staging_recent"
        staging_dir.mkdir()
        
        import shutil
        for filepath in vault.source_dir.glob("*.jpg"):
            if filepath.stat().st_mtime > cutoff_time:
                shutil.copy2(filepath, staging_dir / filepath.name)
        
        result = vault.import_dir(staging_dir)
        assert result.returncode == 0
        
        data = json.loads(result.stdout)
        assert data["imported"] == 1, f"Expected 1 recent file, got {data}"
        
        rows = vault.db_files()
        assert len(rows) == 1
        assert "recent" in rows[0]["path"]

    def test_scan_filter_by_camera_model_import(self, vault: VaultEnv) -> None:
        """Scan files, filter by camera model in EXIF, then import.
        
        Scenario: User has photos from multiple cameras but only wants
        to import photos from their main camera.
        """
        if not EXIFTOOL_AVAILABLE:
            pytest.skip("exiftool not available")
        
        # Create files with different camera models
        camera_files = [
            ("iphone_photo.jpg", "Apple", "iPhone 15"),
            ("canon_photo.jpg", "Canon", "EOS R5"),
            ("sony_photo.jpg", "Sony", "A7IV"),
        ]
        
        for filename, make, model in camera_files:
            filepath = vault.source_dir / filename
            create_minimal_jpeg(filepath, f"content_{filename}")
            subprocess.run([
                "exiftool", "-overwrite_original",
                f"-Make={make}",
                f"-Model={model}",
                "-DateTimeOriginal=2024:05:01 12:00:00",
                str(filepath)
            ], check=True, capture_output=True)
        
        # Filter: only Canon photos (by checking EXIF)
        # In real usage, this could use exiftool in a pipeline
        staging_dir = vault.output_dir / "staging_canon"
        staging_dir.mkdir()
        
        import shutil
        for filepath in vault.source_dir.glob("*.jpg"):
            # Check EXIF Make using exiftool
            result = subprocess.run(
                ["exiftool", "-Make", "-s3", str(filepath)],
                capture_output=True, text=True
            )
            if "Canon" in result.stdout:
                shutil.copy2(filepath, staging_dir / filepath.name)
        
        result = vault.import_dir(staging_dir)
        assert result.returncode == 0
        
        data = json.loads(result.stdout)
        assert data["imported"] == 1, f"Expected 1 Canon file, got {data}"
        
        rows = vault.db_files()
        assert len(rows) == 1
        assert "Canon" in rows[0]["path"]

    def test_scan_filter_exclude_pattern_import(self, vault: VaultEnv) -> None:
        """Scan files, exclude by pattern (e.g., skip edited/cropped files), then import.
        
        Pattern: Exclude files with "_edited", "_crop", "_filter" in name.
        """
        files = [
            "IMG_001.jpg",          # Original
            "IMG_001_edited.jpg",   # Edited version - should exclude
            "IMG_002.jpg",          # Original
            "IMG_002_crop.jpg",     # Cropped version - should exclude
            "IMG_003_filter.jpg",   # Filtered - should exclude
        ]
        
        for filename in files:
            filepath = vault.source_dir / filename
            create_minimal_jpeg(filepath, f"content_{filename}")
        
        # Filter: exclude files with _edited, _crop, _filter
        exclude_patterns = ["_edited", "_crop", "_filter"]
        staging_dir = vault.output_dir / "staging_originals"
        staging_dir.mkdir()
        
        import shutil
        for filepath in vault.source_dir.glob("*.jpg"):
            if not any(pattern in filepath.name for pattern in exclude_patterns):
                shutil.copy2(filepath, staging_dir / filepath.name)
        
        result = vault.import_dir(staging_dir)
        assert result.returncode == 0
        
        data = json.loads(result.stdout)
        assert data["imported"] == 2, f"Expected 2 original files, got {data}"
        
        rows = vault.db_files()
        assert len(rows) == 2


class TestImportShowDup:
    """Test import --show-dup flag for duplicate file visibility."""

    def test_import_show_dup_shows_duplicate_files(self, vault: VaultEnv) -> None:
        """Test that --show-dup shows duplicate files during scanning."""
        # Create and import first batch
        create_minimal_jpeg(vault.source_dir / "photo1.jpg", "content_1")
        create_minimal_jpeg(vault.source_dir / "photo2.jpg", "content_2")
        result1 = vault.import_dir(vault.source_dir)
        assert result1.returncode == 0
        
        # Create identical files in new source (all duplicates)
        new_source = vault.output_dir / "new_source"
        new_source.mkdir()
        create_minimal_jpeg(new_source / "photo1.jpg", "content_1")  # dup
        create_minimal_jpeg(new_source / "photo2.jpg", "content_2")  # dup
        
        # Import with --show-dup
        result2 = vault.run("import", "--yes", "--show-dup", str(new_source))
        assert result2.returncode == 0
        
        # Should show "Duplicate" label during scanning
        combined = result2.stdout + result2.stderr
        assert "Duplicate" in combined
        assert "photo1.jpg" in combined
        assert "photo2.jpg" in combined

    def test_import_without_show_dup_hides_duplicates(self, vault: VaultEnv) -> None:
        """Test that without --show-dup, duplicate files are not shown during scanning."""
        # Create and import first batch
        create_minimal_jpeg(vault.source_dir / "photo.jpg", "content")
        result1 = vault.import_dir(vault.source_dir)
        assert result1.returncode == 0
        
        # Create identical file in new source
        new_source = vault.output_dir / "new_source"
        new_source.mkdir()
        create_minimal_jpeg(new_source / "photo.jpg", "content")
        
        # Import without --show-dup
        result2 = vault.run("import", "--yes", str(new_source))
        assert result2.returncode == 0
        
        # Should show summary count but not "Duplicate" label for individual files
        combined = result2.stdout + result2.stderr
        assert "Likely duplicate:" in combined
        # The word "Duplicate" should not appear in scanning section (only in summary)
        lines = combined.split('\n')
        scanning_section = False
        found_duplicate_label = False
        for line in lines:
            if "Scanning" in line:
                scanning_section = True
            elif "Pre-flight:" in line:
                scanning_section = False
            elif scanning_section and "Duplicate" in line and "photo.jpg" in line:
                found_duplicate_label = True
        assert not found_duplicate_label


class TestScanImportDirectoryStructure:
    """Test scanning nested directory structures and filtering."""

    def test_scan_nested_filter_by_depth_import(self, vault: VaultEnv) -> None:
        """Scan nested dirs, filter by depth, then import."""
        # Create nested structure
        structure = [
            ("root.jpg", ""),
            ("level1/l1.jpg", "level1"),
            ("level1/level2/l2.jpg", "level1/level2"),
            ("level1/level2/level3/l3.jpg", "level1/level2/level3"),
        ]
        
        for filename, subdir in structure:
            dir_path = vault.source_dir / subdir if subdir else vault.source_dir
            dir_path.mkdir(parents=True, exist_ok=True)
            create_minimal_jpeg(dir_path / Path(filename).name, f"content_{filename}")
        
        # Filter: only files at depth 0-1 (not deeper)
        staging_dir = vault.output_dir / "staging_shallow"
        staging_dir.mkdir()
        
        import shutil
        for filepath in vault.source_dir.rglob("*.jpg"):
            # Calculate depth relative to source_dir
            rel_parts = filepath.relative_to(vault.source_dir).parts
            depth = len(rel_parts) - 1  # 0 for root, 1 for level1, etc.
            
            if depth <= 1:
                dest_dir = staging_dir / Path(*rel_parts[:-1]) if len(rel_parts) > 1 else staging_dir
                dest_dir.mkdir(parents=True, exist_ok=True)
                shutil.copy2(filepath, dest_dir / filepath.name)
        
        result = vault.import_dir(staging_dir)
        assert result.returncode == 0
        
        data = json.loads(result.stdout)
        assert data["imported"] == 2, f"Expected 2 files (depth 0-1), got {data}"


class TestScanImportBatchProcessing:
    """Test batch processing of scanned and filtered files."""

    def test_scan_filter_large_batch_import(self, vault: VaultEnv) -> None:
        """Test filtering a large batch of files and importing."""
        # Create 50 files
        for i in range(50):
            filename = f"photo_{i:03d}.jpg"
            create_minimal_jpeg(vault.source_dir / filename, f"content_{i}")
        
        # Filter: only even-numbered files
        staging_dir = vault.output_dir / "staging_even"
        staging_dir.mkdir()
        
        import shutil
        for filepath in vault.source_dir.glob("photo_*.jpg"):
            num = int(filepath.stem.split("_")[1])
            if num % 2 == 0:
                shutil.copy2(filepath, staging_dir / filepath.name)
        
        result = vault.import_dir(staging_dir)
        assert result.returncode == 0
        
        data = json.loads(result.stdout)
        assert data["imported"] == 25, f"Expected 25 even files, got {data}"
        
        rows = vault.db_files()
        assert len(rows) == 25

    def test_scan_filter_mixed_content_import(self, vault: VaultEnv) -> None:
        """Test scanning mixed content types, filtering specific types, then import."""
        # Create mixed content
        content_types = [
            ("photo.jpg", "jpeg"),
            ("video.mp4", "mp4"),
            ("raw.dng", "raw"),
            ("thumb.jpg", "jpeg"),  # Small jpg
            ("clip.mov", "mov"),
        ]
        
        for filename, ftype in content_types:
            filepath = vault.source_dir / filename
            if ftype == "jpeg":
                create_minimal_jpeg(filepath, f"content_{filename}")
            elif ftype == "mp4":
                create_minimal_mp4(filepath)
            elif ftype == "raw":
                create_minimal_raw(filepath)
            elif ftype == "mov":
                # Minimal MOV
                filepath.write_bytes(b"\x00\x00\x00\x14ftypqt  \x00\x00\x00\x00qt  ")
        
        # Filter: only images (jpg, dng), skip videos
        # Note: DNG is also an image format (RAW), so we expect 3 files:
        # photo.jpg, thumb.jpg, and raw.dng
        image_exts = {".jpg", ".jpeg", ".dng", ".cr2", ".cr3", ".nef"}
        staging_dir = vault.output_dir / "staging_images"
        staging_dir.mkdir()
        
        import shutil
        for filepath in vault.source_dir.iterdir():
            if filepath.suffix.lower() in image_exts:
                shutil.copy2(filepath, staging_dir / filepath.name)
        
        result = vault.import_dir(staging_dir)
        assert result.returncode == 0
        
        data = json.loads(result.stdout)
        # Expect 3 images: 2 JPG + 1 DNG (RAW is also an image)
        assert data["imported"] == 3, f"Expected 3 image files (2 JPG + 1 DNG), got {data}"
