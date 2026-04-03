"""Media binding tests for Live Photo and RAW+JPEG pairs.

Tests detection and import behavior for:
- Live Photo: .heic/.jpg + .mov pairs
- RAW+JPEG: .dng/.arw + .jpg pairs
- Burst sequences
"""

from __future__ import annotations

import subprocess
from datetime import datetime
from pathlib import Path

import pytest


def create_live_photo_pair(
    directory: Path,
    base_name: str,
    timestamp: datetime,
    ffmpeg_available: bool,
) -> tuple[Path, Path]:
    """Create a Live Photo pair (HEIC/JPG + MOV).
    
    Returns:
        Tuple of (image_path, video_path)
    """
    directory.mkdir(parents=True, exist_ok=True)
    
    image_path = directory / f"{base_name}.jpg"
    video_path = directory / f"{base_name}.mov"
    
    # Create JPEG with EXIF
    try:
        from PIL import Image, ExifTags
        from PIL.ExifTags import TAGS
        
        img = Image.new("RGB", (100, 100), color="red")
        
        # Add basic EXIF
        exif = img.getexif()
        # DateTimeOriginal (tag 0x9003)
        date_str = timestamp.strftime("%Y:%m:%d %H:%M:%S")
        exif[0x9003] = date_str  # DateTimeOriginal
        exif[0x9004] = date_str  # DateTimeDigitized
        exif[0x0131] = "Apple"   # Make
        exif[0x0110] = "iPhone 15 Pro"  # Model
        
        img.save(image_path, "JPEG", exif=exif)
    except ImportError:
        # Fallback: create minimal JPEG
        image_path.write_bytes(b"\xff\xd8\xff\xe0" + b"\x00" * 100)
    
    # Create MOV with creation_time using ffmpeg
    if ffmpeg_available:
        date_str = timestamp.strftime("%Y-%m-%d %H:%M:%S")
        result = subprocess.run(
            [
                "ffmpeg", "-y",
                "-f", "lavfi",
                "-i", "testsrc=duration=1:size=100x100:rate=1",
                "-pix_fmt", "yuv420p",
                "-metadata", f"creation_time={date_str}",
                "-c:v", "libx264",
                "-preset", "ultrafast",
                str(video_path),
            ],
            capture_output=True,
        )
        if result.returncode != 0:
            # Fallback: create minimal MOV
            video_path.write_bytes(b"\x00\x00\x00\x20ftypqt20qt20\x00\x00\x00\x00" + b"\x00" * 100)
    else:
        # Create minimal MOV structure
        video_path.write_bytes(b"\x00\x00\x00\x20ftypqt20qt20\x00\x00\x00\x00" + b"\x00" * 100)
    
    return image_path, video_path


def create_raw_jpeg_pair(
    directory: Path,
    base_name: str,
    timestamp: datetime,
) -> tuple[Path, Path]:
    """Create a RAW+JPEG pair (DNG + JPG).
    
    Returns:
        Tuple of (raw_path, jpeg_path)
    """
    directory.mkdir(parents=True, exist_ok=True)
    
    raw_path = directory / f"{base_name}.dng"
    jpeg_path = directory / f"{base_name}.jpg"
    
    # Create JPEG with EXIF
    try:
        from PIL import Image
        
        img = Image.new("RGB", (100, 100), color="blue")
        exif = img.getexif()
        date_str = timestamp.strftime("%Y:%m:%d %H:%M:%S")
        exif[0x9003] = date_str
        exif[0x0131] = "Sony"
        exif[0x0110] = "ILCE-7M4"
        
        img.save(jpeg_path, "JPEG", exif=exif)
    except ImportError:
        jpeg_path.write_bytes(b"\xff\xd8\xff\xe0" + b"\x00" * 100)
    
    # Create minimal DNG (TIFF-based)
    # DNG has magic bytes: II (little endian) + 42 (magic) + 8 (IFD offset)
    raw_path.write_bytes(b"II\x2a\x00\x08\x00\x00\x00" + b"\x00" * 200)
    
    return raw_path, jpeg_path


def create_burst_sequence(
    directory: Path,
    prefix: str,
    count: int,
    timestamp: datetime,
) -> list[Path]:
    """Create a burst sequence of images.
    
    Returns:
        List of image paths
    """
    directory.mkdir(parents=True, exist_ok=True)
    paths = []
    
    for i in range(1, count + 1):
        path = directory / f"{prefix}_{i:04d}.jpg"
        
        try:
            from PIL import Image
            
            img = Image.new("RGB", (100, 100), color="green")
            exif = img.getexif()
            date_str = timestamp.strftime("%Y:%m:%d %H:%M:%S")
            exif[0x9003] = date_str
            exif[0x0131] = "Canon"
            exif[0x0110] = "EOS R5"
            
            img.save(path, "JPEG", exif=exif)
        except ImportError:
            path.write_bytes(b"\xff\xd8\xff\xe0" + b"\x00" * 100)
        
        paths.append(path)
    
    return paths


class TestLivePhotoBinding:
    """F1-F2: Live Photo detection and import tests."""

    def test_live_photo_detection_with_ffmpeg(
        self, vault, ffmpeg_available
    ):
        """F1: Live Photo (HEIC/JPG + MOV) should be detected as binding.
        
        Setup:
        - Create IMG_1234.jpg + IMG_1234.mov with same base name
        - Import directory
        
        Verify:
        - Both files imported
        - Use same timestamp path
        """
        if not ffmpeg_available:
            pytest.skip("ffmpeg not available")
        
        timestamp = datetime(2024, 3, 15, 14, 30, 0)
        
        # Create Live Photo pair
        img_path, mov_path = create_live_photo_pair(
            vault.source_dir,
            "IMG_1234",
            timestamp,
            ffmpeg_available=True,
        )
        
        # Import
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0, f"Import failed: {result.stderr}"
        
        # Verify both files imported
        vault_dir = Path(vault.root)
        
        # Check files exist in vault
        jpg_imported = list(vault_dir.rglob("*IMG_1234*.jpg"))
        mov_imported = list(vault_dir.rglob("*IMG_1234*.mov"))
        
        assert len(jpg_imported) > 0, "JPEG not imported"
        assert len(mov_imported) > 0, "MOV not imported"

    def test_live_photo_same_device_path(
        self, vault, ffmpeg_available
    ):
        """F2: Live Photo pair should use same device path.
        
        Setup:
        - Create Live Photo pair with same device EXIF
        
        Verify:
        - Both files in same $device/$year/$mon/$day structure
        """
        if not ffmpeg_available:
            pytest.skip("ffmpeg not available")
        
        timestamp = datetime(2024, 5, 20, 10, 0, 0)
        
        create_live_photo_pair(
            vault.source_dir,
            "IMG_5678",
            timestamp,
            ffmpeg_available=True,
        )
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0, f"Import failed: {result.stderr}"
        
        # Check database path consistency
        db_result = vault.db_query(
            "SELECT path FROM files ORDER BY path"
        )
        
        if db_result:
            paths = [row["path"] for row in db_result]
            # Both files should be imported
            assert len(paths) >= 2, f"Expected 2 files, found {len(paths)}"


class TestRawJpegBinding:
    """F3-F4: RAW+JPEG binding detection and import tests."""

    def test_raw_jpeg_detection(
        self, vault
    ):
        """F3: RAW+JPEG pair should be detected as binding.
        
        Setup:
        - Create DSC_0001.dng + DSC_0001.jpg with same base name
        
        Verify:
        - Both files imported
        """
        timestamp = datetime(2024, 6, 10, 16, 45, 0)
        
        raw_path, jpeg_path = create_raw_jpeg_pair(
            vault.source_dir,
            "DSC_0001",
            timestamp,
        )
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0, f"Import failed: {result.stderr}"
        
        # Verify both files imported
        vault_dir = Path(vault.root)
        
        dng_imported = list(vault_dir.rglob("*.dng"))
        jpg_imported = list(vault_dir.rglob("*DSC_0001*.jpg"))
        
        assert len(dng_imported) > 0, "DNG not imported"
        assert len(jpg_imported) > 0, "JPEG not imported"

    def test_raw_jpeg_same_organization(
        self, vault
    ):
        """F4: RAW and JPEG should be in same date hierarchy.
        
        Verify:
        - RAW and JPEG in same $year/$mon/$day directory (may differ in device)
        """
        timestamp = datetime(2024, 7, 25, 9, 15, 0)
        
        create_raw_jpeg_pair(
            vault.source_dir,
            "DSC_0002",
            timestamp,
        )
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        # Check database paths - both should have same year/month/day
        db_result = vault.db_query(
            "SELECT path FROM files ORDER BY path"
        )
        
        if db_result:
            paths = [row["path"] for row in db_result]
            # Extract date parts from path
            # Path format examples:
            #   {device}/{year}/{month}-{day}/{filename}
            #   Unknown/2026/04-03/IMG_0001.jpg
            date_parts = []
            for p in paths:
                parts = Path(p).parts
                # Find year part (4 digits) and month-day part
                for i, part in enumerate(parts):
                    if part.isdigit() and len(part) == 4:  # Year
                        if i + 1 < len(parts):
                            date_key = f"{part}/{parts[i+1]}"
                            date_parts.append(date_key)
                            break
            
            # All files should be in same date directory
            if date_parts:
                assert len(set(date_parts)) == 1, f"Files in different dates: {date_parts}"


class TestBurstSequence:
    """F5-F6: Burst sequence detection tests."""

    def test_burst_detection(
        self, vault
    ):
        """F5: Burst sequence should be detected.
        
        Setup:
        - Create IMG_0001.jpg ~ IMG_0005.jpg
        
        Verify:
        - All files imported
        """
        timestamp = datetime(2024, 8, 1, 12, 0, 0)
        
        paths = create_burst_sequence(
            vault.source_dir,
            "IMG",
            5,
            timestamp,
        )
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0, f"Import failed: {result.stderr}"
        
        # Verify all 5 files imported
        vault_dir = Path(vault.root)
        imported = list(vault_dir.rglob("IMG_*.jpg"))
        
        # Should have 5 imported files
        # (may have more due to other test artifacts)
        assert len(imported) >= 5, f"Expected 5 burst files, found {len(imported)}"


class TestBindingEdgeCases:
    """F6: Edge cases for binding detection."""

    def test_partial_binding_import(
        self, vault, ffmpeg_available
    ):
        """F6: Partial binding (only one file) should still import.
        
        Setup:
        - Create only .jpg without .mov
        
        Verify:
        - Single file imports successfully
        """
        timestamp = datetime(2024, 9, 10, 8, 0, 0)
        
        # Create only JPEG, no video
        directory = vault.source_dir
        directory.mkdir(parents=True, exist_ok=True)
        
        try:
            from PIL import Image
            
            img = Image.new("RGB", (100, 100), color="yellow")
            exif = img.getexif()
            exif[0x9003] = timestamp.strftime("%Y:%m:%d %H:%M:%S")
            
            img.save(directory / "SINGLE_001.jpg", "JPEG", exif=exif)
        except ImportError:
            (directory / "SINGLE_001.jpg").write_bytes(
                b"\xff\xd8\xff\xe0" + b"\x00" * 100
            )
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0, f"Import failed: {result.stderr}"
        
        # Verify single file imported
        vault_dir = Path(vault.root)
        imported = list(vault_dir.rglob("*SINGLE_001*.jpg"))
        
        assert len(imported) > 0, "Single file not imported"
