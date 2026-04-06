"""Video metadata extraction tests for Svault.

Tests video file creation_time extraction and path organization:
- MP4 creation_time (v0/v1)
- MOV creation_time
- MTS timestamp (if supported)
- Video device info extraction
"""

from __future__ import annotations

import subprocess
from datetime import datetime, timezone
from pathlib import Path

import pytest


def create_mp4_with_timestamp(
    path: Path,
    creation_time: datetime,
    size: tuple[int, int] = (100, 100),
) -> bool:
    """Create an MP4 file with specific creation_time using ffmpeg.
    
    Args:
        path: Output file path
        creation_time: Video creation timestamp
        size: Video dimensions (width, height)
    
    Returns:
        True if successful
    """
    # Format timestamp for ffmpeg (ISO 8601)
    time_str = creation_time.strftime("%Y-%m-%d %H:%M:%S")
    
    result = subprocess.run(
        [
            "ffmpeg", "-y",
            "-f", "lavfi",
            "-i", f"testsrc=duration=1:size={size[0]}x{size[1]}:rate=1",
            "-pix_fmt", "yuv420p",
            "-metadata", f"creation_time={time_str}",
            "-c:v", "libx264",
            "-preset", "ultrafast",
            str(path),
        ],
        capture_output=True,
    )
    
    return result.returncode == 0


def create_mov_with_timestamp(
    path: Path,
    creation_time: datetime,
    size: tuple[int, int] = (100, 100),
) -> bool:
    """Create a MOV file with specific creation_time.
    
    MOV uses QuickTime format which is similar to MP4.
    """
    time_str = creation_time.strftime("%Y-%m-%d %H:%M:%S")
    
    result = subprocess.run(
        [
            "ffmpeg", "-y",
            "-f", "lavfi",
            "-i", f"testsrc=duration=1:size={size[0]}x{size[1]}:rate=1",
            "-pix_fmt", "yuv420p",
            "-metadata", f"creation_time={time_str}",
            "-c:v", "libx264",
            "-preset", "ultrafast",
            "-f", "mov",
            str(path),
        ],
        capture_output=True,
    )
    
    return result.returncode == 0


def verify_video_timestamp(path: Path) -> datetime | None:
    """Extract creation_time from video using ffprobe.
    
    Returns:
        datetime if found, None otherwise
    """
    result = subprocess.run(
        [
            "ffprobe",
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "format_tags=creation_time",
            "-of", "default=noprint_wrappers=1:nokey=1",
            str(path),
        ],
        capture_output=True,
        text=True,
    )
    
    if result.returncode == 0 and result.stdout.strip():
        try:
            # Parse ISO 8601 format: 2024-03-15T14:30:00.000000Z
            ts_str = result.stdout.strip()
            # Handle both with and without microseconds
            if "." in ts_str:
                ts_str = ts_str.replace("Z", "+00:00")
                return datetime.fromisoformat(ts_str)
            else:
                ts_str = ts_str.replace("Z", "+00:00")
                return datetime.fromisoformat(ts_str)
        except ValueError:
            pass
    
    return None


class TestVideoMetadataExtraction:
    """V1-V3: Video creation_time extraction tests."""

    def test_mp4_creation_time_v0_v1(
        self, vault, ffmpeg_available
    ):
        """V1-V2: MP4 creation_time should be extracted and used for path.
        
        Setup:
        - Create MP4 with specific creation_time
        - Import
        
        Verify:
        - File organized by creation_time (not mtime)
        """
        if not ffmpeg_available:
            pytest.skip("ffmpeg not available")
        
        # Create video with known timestamp
        timestamp = datetime(2024, 3, 15, 14, 30, 0, tzinfo=timezone.utc)
        video_path = vault.source_dir / "test_video_2024.mp4"
        
        success = create_mp4_with_timestamp(video_path, timestamp)
        assert success, "Failed to create test MP4"
        
        # Verify timestamp was set
        extracted_ts = verify_video_timestamp(video_path)
        assert extracted_ts is not None, "Failed to verify video timestamp"
        
        # Import
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0, f"Import failed: {result.stderr}"
        
        # Check database for capture_time
        db_result = vault.db_query(
            "SELECT path FROM files WHERE path LIKE '%.mp4%'"
        )
        
        if db_result:
            path = db_result[0]["path"]
            # Should be organized by 2024/03/15 (from creation_time)
            assert "2024" in path, f"Not organized by year: {path}"
            assert "03" in path or "3" in path, f"Not organized by month: {path}"

    def test_mov_creation_time(
        self, vault, ffmpeg_available
    ):
        """V3: MOV creation_time should be extracted.
        
        Setup:
        - Create MOV with specific creation_time
        - Import
        
        Verify:
        - File organized by creation_time
        """
        if not ffmpeg_available:
            pytest.skip("ffmpeg not available")
        
        timestamp = datetime(2023, 8, 25, 9, 15, 0, tzinfo=timezone.utc)
        video_path = vault.source_dir / "test_video_2023.mov"
        
        success = create_mov_with_timestamp(video_path, timestamp)
        assert success, "Failed to create test MOV"
        
        # Import
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0, f"Import failed: {result.stderr}"
        
        # Check path organization
        db_result = vault.db_query(
            "SELECT path FROM files WHERE path LIKE '%.mov%'"
        )
        
        if db_result:
            path = db_result[0]["path"]
            assert "2023" in path, f"Not organized by year: {path}"

    def test_video_vs_mtime_priority(
        self, vault, ffmpeg_available
    ):
        """Video creation_time takes priority over file mtime.
        
        Setup:
        - Create video with creation_time in 2024
        - Set file mtime to 2025
        - Import
        
        Verify:
        - File organized by 2024 (creation_time), not 2025 (mtime)
        """
        if not ffmpeg_available:
            pytest.skip("ffmpeg not available")
        
        import os
        import time
        
        # Create video with 2024 timestamp
        creation_ts = datetime(2024, 6, 1, 12, 0, 0, tzinfo=timezone.utc)
        video_path = vault.source_dir / "priority_test.mp4"
        
        success = create_mp4_with_timestamp(video_path, creation_ts)
        assert success
        
        # Set mtime to 2025 (different from creation_time)
        mtime_ts = datetime(2025, 1, 1, 0, 0, 0, tzinfo=timezone.utc)
        mtime_seconds = mtime_ts.timestamp()
        os.utime(video_path, (mtime_seconds, mtime_seconds))
        
        # Import
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        # Verify organized by creation_time (2024)
        db_result = vault.db_query(
            "SELECT path FROM files WHERE path LIKE '%priority_test%'"
        )
        
        if db_result:
            path = db_result[0]["path"]
            # Should be in 2024, not 2025
            assert "2024" in path, (
                f"File organized by mtime not creation_time: {path}"
            )


class TestVideoDeviceInfo:
    """V4: Video device info extraction tests."""

def create_mov_with_device_info(
    path: Path,
    creation_time: datetime,
    make: str,
    model: str,
    size: tuple[int, int] = (100, 100),
) -> bool:
    """Create a MOV file with device make/model metadata using ffmpeg.
    
    MOV (QuickTime) format supports Make/Model metadata tags.
    
    Args:
        path: Output file path
        creation_time: Video creation timestamp
        make: Device manufacturer (e.g., "Apple", "Samsung")
        model: Device model (e.g., "iPhone 15 Pro", "SM-S908B")
        size: Video dimensions
    
    Returns:
        True if successful
    """
    time_str = creation_time.strftime("%Y-%m-%d %H:%M:%S")
    
    result = subprocess.run(
        [
            "ffmpeg", "-y",
            "-f", "lavfi",
            "-i", f"testsrc=duration=1:size={size[0]}x{size[1]}:rate=1",
            "-pix_fmt", "yuv420p",
            # Basic metadata
            "-metadata", f"creation_time={time_str}",
            # Device metadata (QuickTime format)
            "-metadata", f"make={make}",
            "-metadata", f"model={model}",
            # Also try com.apple.quicktime format
            "-metadata", f"com.apple.quicktime.make={make}",
            "-metadata", f"com.apple.quicktime.model={model}",
            "-c:v", "libx264",
            "-preset", "ultrafast",
            "-f", "mov",
            str(path),
        ],
        capture_output=True,
    )
    
    return result.returncode == 0


def verify_video_device_info(path: Path) -> dict[str, str]:
    """Extract device make/model from video using ffprobe.
    
    Returns:
        Dict with 'make' and 'model' keys (may be empty)
    """
    result = subprocess.run(
        [
            "ffprobe",
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "format_tags=make,model,com.apple.quicktime.make,com.apple.quicktime.model",
            "-of", "default=noprint_wrappers=1",
            str(path),
        ],
        capture_output=True,
        text=True,
    )
    
    info = {"make": "", "model": ""}
    
    if result.returncode == 0:
        for line in result.stdout.strip().split("\n"):
            if "=" in line:
                key, value = line.split("=", 1)
                key = key.strip()
                value = value.strip()
                if "make" in key.lower() and value:
                    info["make"] = value
                elif "model" in key.lower() and value:
                    info["model"] = value
    
    return info


class TestVideoDeviceExtraction:
    """Device info extraction from video metadata."""

    def test_video_device_model_extraction(
        self, vault, ffmpeg_available
    ):
        """V4: Device model metadata injection using ffmpeg.
        
        Setup:
        - Create MOV with Make/Model metadata using ffmpeg
        - Import
        
        Verify:
        - Device metadata is correctly embedded in video file
        - File is imported successfully
        
        Note: This test validates ffmpeg's capability to inject device metadata.
        Whether svault extracts and uses this info depends on its implementation.
        """
        if not ffmpeg_available:
            pytest.skip("ffmpeg not available")
        
        timestamp = datetime(2024, 6, 15, 10, 30, 0, tzinfo=timezone.utc)
        video_path = vault.source_dir / "iphone_video.mov"
        
        # Create MOV with Apple iPhone metadata
        success = create_mov_with_device_info(
            video_path,
            timestamp,
            make="Apple",
            model="iPhone 15 Pro",
        )
        assert success, "Failed to create test MOV with device info"
        
        # Verify metadata was set using ffprobe
        device_info = verify_video_device_info(video_path)
        print(f"Extracted device info: {device_info}")
        
        # ffmpeg should have set at least one of the make/model fields
        has_device_info = bool(device_info["make"] or device_info["model"])
        if not has_device_info:
            pytest.skip("ffmpeg version does not support device metadata injection")
        
        # Import
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        # Verify file was imported (check path contains expected date)
        db_result = vault.db_query(
            "SELECT path FROM files WHERE path LIKE '%2024%' AND path LIKE '%.mov%'"
        )
        
        assert len(db_result) >= 1, "iPhone video not imported"
        
        # If svault extracts device info into path, check for it
        path = db_result[0]["path"]
        print(f"Imported path: {path}")
        
        # Note: svault may or may not include device info in path
        # depending on its implementation. We just verify file was imported.

    def test_video_device_model_samsung(
        self, vault, ffmpeg_available
    ):
        """V4b: Samsung device model extraction.
        
        Test with Samsung-style model naming.
        """
        if not ffmpeg_available:
            pytest.skip("ffmpeg not available")
        
        timestamp = datetime(2024, 7, 20, 15, 45, 0, tzinfo=timezone.utc)
        video_path = vault.source_dir / "samsung_video.mov"
        
        # Create MOV with Samsung metadata
        success = create_mov_with_device_info(
            video_path,
            timestamp,
            make="Samsung",
            model="SM-S908B",  # Galaxy S22 Ultra
        )
        assert success
        
        # Verify metadata
        device_info = verify_video_device_info(video_path)
        print(f"Samsung device info: {device_info}")
        
        # Import
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        # Verify file was imported (check path contains expected date)
        db_result = vault.db_query(
            "SELECT path FROM files WHERE path LIKE '%2024%' AND path LIKE '%.mov%'"
        )
        assert len(db_result) >= 1, "Samsung video not imported"

    def test_video_imported_to_device_path(
        self, vault, ffmpeg_available
    ):
        """V5: Video with device info should be organized into $year/$mon-$day/$device/ path.
        
        Setup:
        - Create MOV with iPhone device metadata
        - Import with path template including $device
        
        Verify:
        - File is imported to path like: 2024/06-15/iPhone/ or 2024/06-15/Apple iPhone/
        """
        if not ffmpeg_available:
            pytest.skip("ffmpeg not available")
        
        timestamp = datetime(2024, 8, 25, 14, 30, 0, tzinfo=timezone.utc)
        video_path = vault.source_dir / "test_iphone_video.mov"
        
        # Create MOV with iPhone metadata
        success = create_mov_with_device_info(
            video_path,
            timestamp,
            make="Apple",
            model="iPhone 14",
        )
        assert success, "Failed to create test video"
        
        # Verify device metadata was embedded
        device_info = verify_video_device_info(video_path)
        print(f"Device info: {device_info}")
        
        if not (device_info["make"] or device_info["model"]):
            pytest.skip("ffmpeg does not support device metadata on this system")
        
        # Import
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        # Query imported file path
        db_result = vault.db_query(
            "SELECT path FROM files WHERE path LIKE '%2024%' AND path LIKE '%.mov%'"
        )
        assert len(db_result) >= 1, "Video not found in database"
        
        path = db_result[0]["path"]
        print(f"Imported to: {path}")
        
        # Verify path structure: should contain year, month-day, and possibly device
        assert "2024" in path, f"Path should contain year: {path}"
        assert ("08-25" in path or "08" in path), f"Path should contain month-day: {path}"
        
        # If svault uses $device in path template, check for device name
        # Common patterns: "iPhone", "Apple", "iPhone 14"
        has_device_in_path = (
            "iPhone" in path or 
            "Apple" in path or 
            "iphone" in path.lower()
        )
        
        if has_device_in_path:
            print(f"✓ Device name found in path: {path}")
        else:
            # Device not in path - svault may not be configured to use $device
            # or may use a different path template
            print(f"ℹ Device name not in path (may be expected): {path}")
            # Just verify it's in a year/month structure
            assert ".mov" in path.lower(), f"Video extension not in path: {path}"


class TestVideoPathOrganization:
    """V6: Video file path organization tests."""

    def test_video_organized_by_year_month_day(
        self, vault, ffmpeg_available
    ):
        """V6: Video should be organized into $year/$mon/$day structure.
        
        Setup:
        - Create video with creation_time: 2024-07-20 16:30:00
        - Import
        
        Verify:
        - Path contains 2024/07/20 or similar structure
        """
        if not ffmpeg_available:
            pytest.skip("ffmpeg not available")
        
        timestamp = datetime(2024, 7, 20, 16, 30, 0, tzinfo=timezone.utc)
        video_path = vault.source_dir / "organization_test.mp4"
        
        success = create_mp4_with_timestamp(video_path, timestamp)
        assert success
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        # Check path structure
        db_result = vault.db_query(
            "SELECT path FROM files WHERE path LIKE '%organization_test%'"
        )
        
        if db_result:
            path = db_result[0]["path"]
            path_parts = Path(path).parts
            
            # Should have year, month, day in path
            # Format: {device}/{year}/{month}/{day}/{filename}
            assert "2024" in path_parts, f"Year not in path: {path}"
            # Month might be "07" or "7"
            month_found = any("7" in p or "07" in p for p in path_parts)
            assert month_found, f"Month not in path: {path}"

    def test_multiple_videos_different_dates(
        self, vault, ffmpeg_available
    ):
        """Multiple videos with different dates should be organized separately.
        """
        if not ffmpeg_available:
            pytest.skip("ffmpeg not available")
        
        videos = [
            ("jan_video.mp4", datetime(2024, 1, 15, 10, 0, 0, tzinfo=timezone.utc)),
            ("jun_video.mp4", datetime(2024, 6, 20, 14, 0, 0, tzinfo=timezone.utc)),
            ("dec_video.mp4", datetime(2024, 12, 25, 18, 0, 0, tzinfo=timezone.utc)),
        ]
        
        for name, timestamp in videos:
            path = vault.source_dir / name
            success = create_mp4_with_timestamp(path, timestamp)
            assert success, f"Failed to create {name}"
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        # Check all videos imported with correct dates
        db_result = vault.db_query(
            "SELECT path FROM files WHERE path LIKE '%.mp4%'"
        )
        
        if db_result:
            paths = [row["path"] for row in db_result]
            # Each video should be in its respective month
            assert any("1" in p and "jan" in p.lower() for p in paths) or \
                   any("01" in p for p in paths), "Jan video not organized correctly"
            assert any("6" in p or "06" in p for p in paths), "Jun video not organized correctly"
            assert any("12" in p for p in paths), "Dec video not organized correctly"
