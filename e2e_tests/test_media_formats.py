"""E2E tests for various media format support.

Tests importing different image and video formats into the vault.

中文场景说明：
- 图片格式：JPEG, PNG, TIFF, HEIC, RAW/DNG
- 视频格式：MP4, MOV
- 不同格式的媒体信息提取和路径组织

必要性：
- 确保所有支持的格式都能正确导入
- 验证格式特定的元数据提取（EXIF、创建时间等）
- 测试不同扩展名的文件过滤

Note: Video format tests require ffmpeg for proper file generation.
Tests will be skipped if ffmpeg is not installed.
"""

from __future__ import annotations

import shutil

import pytest

from conftest import VaultEnv, create_media_file, FFMPEG_AVAILABLE

# Mark video tests that require ffmpeg
video_only = pytest.mark.skipif(
    not FFMPEG_AVAILABLE,
    reason="ffmpeg not installed - required for proper video file generation"
)


class TestImageFormats:
    """Tests for various image format support."""
    
    @pytest.fixture(autouse=True)
    def setup_extensions(self, vault: VaultEnv) -> None:
        """Configure vault to accept all test image formats."""
        config_path = vault.vault_dir / "svault.toml"
        config_path.write_text("""
[global]

[import]
path_template = "$year/$mon-$day/$device/$filename"
allowed_extensions = ["jpg", "jpeg", "png", "tiff", "tif", "heic", "heif", "dng", "mp4", "mov"]
""")
    
    @pytest.mark.parametrize("ext", ["jpg", "jpeg", "JPG", "JPEG"])
    def test_import_jpeg_various_extensions(self, vault: VaultEnv, ext: str) -> None:
        """JPEG files with various case extensions should be imported."""
        create_media_file(vault.source_dir / f"test.{ext}", "jpeg", "content1")
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 1
        assert rows[0]["status"] == "imported"
    
    def test_import_png(self, vault: VaultEnv) -> None:
        """PNG files should be imported successfully."""
        create_media_file(vault.source_dir / "test.png", "png", "png_content")
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 1
        assert rows[0]["status"] == "imported"
    
    def test_import_tiff(self, vault: VaultEnv) -> None:
        """TIFF files should be imported successfully."""
        create_media_file(vault.source_dir / "test.tiff", "tiff", "tiff_content")
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 1
    
    def test_import_tif_alternate_extension(self, vault: VaultEnv) -> None:
        """TIF (alternate TIFF extension) should be imported."""
        create_media_file(vault.source_dir / "test.tif", "tif", "tif_content")
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 1
    
    def test_import_heic(self, vault: VaultEnv) -> None:
        """HEIC files should be imported successfully."""
        create_media_file(vault.source_dir / "test.heic", "heic", "heic_content")
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 1
    
    def test_import_dng_raw(self, vault: VaultEnv) -> None:
        """DNG (RAW) files should be imported successfully."""
        create_media_file(vault.source_dir / "test.dng", "dng", "dng_content")
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 1
    
    def test_import_mixed_image_formats(self, vault: VaultEnv) -> None:
        """Mixed image formats in same import should all be imported."""
        formats = [
            ("photo.jpg", "jpeg"),
            ("screenshot.png", "png"),
            ("scan.tiff", "tiff"),
            ("iphone.heic", "heic"),
            ("camera.dng", "dng"),
        ]
        
        for filename, fmt in formats:
            create_media_file(vault.source_dir / filename, fmt, f"content_{filename}")
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 5
        
        # Verify all statuses
        for row in rows:
            assert row["status"] == "imported"


class TestVideoFormats:
    """Tests for video format support.
    
    Note: These tests require ffmpeg to create proper video files.
    Without ffmpeg, minimal structures are created which may not
    be fully valid but should still be importable.
    """
    
    @pytest.fixture(autouse=True)
    def setup_extensions(self, vault: VaultEnv) -> None:
        """Configure vault to accept video formats."""
        config_path = vault.vault_dir / "svault.toml"
        config_path.write_text("""
[global]

[import]
path_template = "$year/$mon-$day/$device/$filename"
allowed_extensions = ["mp4", "mov"]
""")
    
    @video_only
    def test_import_mp4(self, vault: VaultEnv) -> None:
        """MP4 video files should be imported successfully."""
        create_media_file(vault.source_dir / "video.mp4", "mp4", "mp4_content")
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 1
        assert rows[0]["status"] == "imported"
    
    @video_only
    def test_import_mov(self, vault: VaultEnv) -> None:
        """MOV (QuickTime) files should be imported successfully."""
        create_media_file(vault.source_dir / "video.mov", "mov", "mov_content")
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 1
    
    @video_only
    def test_import_mixed_video_formats(self, vault: VaultEnv) -> None:
        """Mixed video formats should all be imported."""
        create_media_file(vault.source_dir / "clip1.mp4", "mp4", "content1")
        create_media_file(vault.source_dir / "clip2.mov", "mov", "content2")
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 2


class TestMixedMediaImport:
    """Tests for mixed image and video imports."""
    
    @pytest.fixture(autouse=True)
    def setup_extensions(self, vault: VaultEnv) -> None:
        """Configure vault to accept all media formats."""
        config_path = vault.vault_dir / "svault.toml"
        config_path.write_text("""
[global]

[import]
path_template = "$year/$mon-$day/$device/$filename"
allowed_extensions = ["jpg", "png", "dng", "mp4", "mov"]
""")
    
    @video_only
    def test_import_photos_and_videos_together(self, vault: VaultEnv) -> None:
        """Photos and videos should be imported together."""
        # Photos
        create_media_file(vault.source_dir / "photo1.jpg", "jpeg", "photo1_content_abc123")
        create_media_file(vault.source_dir / "photo2.png", "png", "photo2_content_def456")
        create_media_file(vault.source_dir / "raw.dng", "dng", "raw1_content_ghi789")
        
        # Videos
        create_media_file(vault.source_dir / "video1.mp4", "mp4", "video1_content_jkl012")
        create_media_file(vault.source_dir / "video2.mov", "mov", "video2_content_mno345")
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 5
    
    def test_duplicate_detection_across_formats(self, vault: VaultEnv) -> None:
        """Duplicate detection should work across different formats.
        
        Note: Different formats with same content are NOT duplicates
        because they have different binary data.
        """
        # Create images in different formats with distinct content markers
        # The content markers ensure different binary data
        create_media_file(vault.source_dir / "image.jpg", "jpeg", "jpeg_format_unique_content_xyz")
        create_media_file(vault.source_dir / "image.png", "png", "png_format_unique_content_abc")
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        # Both should be imported (different binary content due to format headers)
        assert len(rows) == 2, f"Expected 2 files, got {len(rows)}: {rows}"
        for row in rows:
            assert row["status"] == "imported"


class TestFormatFiltering:
    """Tests for format-based file filtering."""
    
    def test_unsupported_extensions_filtered(self, vault: VaultEnv) -> None:
        """Files with unsupported extensions should be filtered out."""
        # Supported
        create_media_file(vault.source_dir / "photo.jpg", "jpeg", "supported")
        
        # Unsupported (not in default allowed_extensions)
        (vault.source_dir / "document.txt").write_text("text content")
        (vault.source_dir / "data.json").write_text('{"key": "value"}')
        (vault.source_dir / "script.py").write_text("print('hello')")
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        # Only jpg should be imported
        assert len(rows) == 1
        assert rows[0]["path"].endswith(".jpg")
    
    def test_custom_extensions_filter(self, vault: VaultEnv) -> None:
        """Custom allowed_extensions should filter correctly."""
        # Create config with only PNG allowed
        config_path = vault.vault_dir / "svault.toml"
        config_path.write_text("""
[global]

[import]
path_template = "$year/$filename"
allowed_extensions = ["png"]
""")
        
        # Multiple formats
        create_media_file(vault.source_dir / "photo.jpg", "jpeg", "jpg_content")
        create_media_file(vault.source_dir / "screenshot.png", "png", "png_content")
        create_media_file(vault.source_dir / "scan.tiff", "tiff", "tiff_content")
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        # Only PNG should be imported
        assert len(rows) == 1
        assert rows[0]["path"].endswith(".png")


class TestFormatSpecificPathTemplates:
    """Tests for format-specific path organization."""
    
    def test_videos_organized_by_extension_in_path(self, vault: VaultEnv) -> None:
        """Videos should be organized in vault paths like photos."""
        create_media_file(vault.source_dir / "vacation.mp4", "mp4", "vacation_video")
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        # Video should exist in vault storage
        vault_files = vault.get_vault_files()
        video_files = [f for f in vault_files if f.suffix.lower() == ".mp4"]
        assert len(video_files) == 1
    
    def test_raw_files_organized_correctly(self, vault: VaultEnv) -> None:
        """RAW files should be organized in vault with correct paths."""
        create_media_file(vault.source_dir / "raw.dng", "dng", "raw_data")
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        vault_files = vault.get_vault_files()
        dng_files = [f for f in vault_files if f.suffix.lower() == ".dng"]
        assert len(dng_files) == 1
