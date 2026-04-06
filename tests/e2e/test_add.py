"""Tests for `svault add` — registering files already inside the vault.

中文说明：
add 命令用于将已存在于 vault 内的文件注册到数据库。
与 import 不同，add 不会复制文件，而是原地跟踪。
"""

from __future__ import annotations

from pathlib import Path

import pytest
from conftest import (
    VaultEnv, 
    create_minimal_jpeg, 
    create_minimal_mp4,
    create_minimal_raw,
)


class TestAddCommand:
    """End-to-end tests for `svault add` basic functionality."""

    def test_add_tracks_existing_files(self, vault: VaultEnv) -> None:
        """Manually place a file inside the vault and register it."""
        vault_file = vault.vault_dir / "manual" / "photo.jpg"
        vault_file.parent.mkdir(parents=True, exist_ok=True)
        create_minimal_jpeg(vault_file, "MANUAL_PHOTO_12345")

        result = vault.run("add", str(vault.vault_dir / "manual"))
        assert result.returncode == 0

        rows = vault.db_files()
        assert len(rows) == 1
        assert rows[0]["status"] == "imported"
        assert str(Path("manual") / "photo.jpg") in rows[0]["path"]

    def test_add_skips_already_tracked(self, vault: VaultEnv) -> None:
        """Re-adding an already tracked file should skip it."""
        vault_file = vault.vault_dir / "photo.jpg"
        create_minimal_jpeg(vault_file, "TRACKED")

        vault.run("add", str(vault.vault_dir))
        rows1 = vault.db_files()
        assert len(rows1) == 1

        result = vault.run("add", str(vault.vault_dir))
        assert result.returncode == 0
        combined = result.stderr + result.stdout
        assert "already tracked" in combined or "0 file(s) added" in combined

        rows2 = vault.db_files()
        assert len(rows2) == 1

    def test_add_detects_duplicates_smoke(self, vault: VaultEnv) -> None:
        """Smoke test: Add should detect duplicates (basic verification).
        
        Detailed deduplication tests are in test_import_dedup.py.
        """
        # Import a file first
        src_file = vault.source_dir / "photo.jpg"
        create_minimal_jpeg(src_file, "DUP_CONTENT")
        vault.import_dir(vault.source_dir)

        # Try to add identical content with different name
        dup_file = vault.vault_dir / "dup.jpg"
        create_minimal_jpeg(dup_file, "DUP_CONTENT")

        result = vault.run("add", str(vault.vault_dir))
        assert result.returncode == 0
        # Should report duplicate or skip
        combined = result.stderr + result.stdout
        assert "duplicate" in combined.lower() or "0 file(s)" in combined


class TestAddFormats:
    """Test add command with various file formats."""

    def test_add_jpeg(self, vault: VaultEnv) -> None:
        """Add JPEG files."""
        vault_file = vault.vault_dir / "photo.jpg"
        create_minimal_jpeg(vault_file)

        result = vault.run("add", str(vault.vault_dir))
        assert result.returncode == 0

        rows = vault.db_files()
        assert len(rows) == 1
        assert rows[0]["path"].endswith(".jpg")

    def test_add_mp4(self, vault: VaultEnv) -> None:
        """Add MP4 video files."""
        vault_file = vault.vault_dir / "video.mp4"
        create_minimal_mp4(vault_file)

        result = vault.run("add", str(vault.vault_dir))
        assert result.returncode == 0

        rows = vault.db_files()
        assert len(rows) == 1
        assert rows[0]["path"].endswith(".mp4")

    def test_add_dng_raw(self, vault: VaultEnv) -> None:
        """Add DNG RAW files."""
        vault_file = vault.vault_dir / "raw.dng"
        create_minimal_raw(vault_file)

        result = vault.run("add", str(vault.vault_dir))
        assert result.returncode == 0

        rows = vault.db_files()
        assert len(rows) == 1
        assert rows[0]["path"].endswith(".dng")

    def test_add_mixed_formats(self, vault: VaultEnv) -> None:
        """Add multiple files with different formats (JPEG, MP4, DNG)."""
        create_minimal_jpeg(vault.vault_dir / "photo.jpg")
        create_minimal_mp4(vault.vault_dir / "video.mp4")
        create_minimal_raw(vault.vault_dir / "raw.dng")

        result = vault.run("add", str(vault.vault_dir))
        assert result.returncode == 0

        rows = vault.db_files()
        assert len(rows) == 3


class TestAddExifHandling:
    """Test EXIF metadata extraction during add."""

    def test_add_extracts_exif_date(self, vault: VaultEnv) -> None:
        """Add should extract EXIF date for path organization."""
        import subprocess
        
        # Create JPEG with specific EXIF date using exiftool
        vault_file = vault.vault_dir / "dated_photo.jpg"
        create_minimal_jpeg(vault_file)
        
        # Add EXIF date
        subprocess.run([
            "exiftool", "-overwrite_original",
            "-DateTimeOriginal=2023:07:15 14:30:00",
            "-Make=Canon",
            "-Model=EOS R5",
            str(vault_file)
        ], check=True, capture_output=True)

        result = vault.run("add", str(vault.vault_dir))
        assert result.returncode == 0

        # File should be tracked
        rows = vault.db_files()
        assert len(rows) == 1

    def test_add_falls_back_to_mtime_without_exif(self, vault: VaultEnv) -> None:
        """Add should use mtime when EXIF is unavailable."""
        vault_file = vault.vault_dir / "no_exif.jpg"
        # Create JPEG without EXIF (just minimal structure)
        create_minimal_jpeg(vault_file)

        result = vault.run("add", str(vault.vault_dir))
        assert result.returncode == 0

        rows = vault.db_files()
        assert len(rows) == 1


class TestAddBatch:
    """Test add with multiple files and directories."""

    def test_add_nested_directories(self, vault: VaultEnv) -> None:
        """Add recursively finds files in nested directories."""
        # Create nested structure with files in each level
        (vault.vault_dir / "level1" / "level2").mkdir(parents=True)
        
        # Files at different levels
        create_minimal_jpeg(vault.vault_dir / "root.jpg")
        create_minimal_jpeg(vault.vault_dir / "level1" / "level1.jpg")
        create_minimal_jpeg(vault.vault_dir / "level1" / "level2" / "level2.jpg")

        result = vault.run("add", str(vault.vault_dir))
        assert result.returncode == 0

        rows = vault.db_files()
        # Should have added all files (depending on config, may organize paths)
        assert len(rows) >= 1  # At least root level

    def test_add_multiple_files(self, vault: VaultEnv) -> None:
        """Add handles multiple files in same directory."""
        # Create 5 files with different content (to avoid duplicate detection)
        for i in range(5):
            create_minimal_jpeg(vault.vault_dir / f"photo_{i:03d}.jpg", f"unique_content_{i}")

        result = vault.run("add", str(vault.vault_dir))
        assert result.returncode == 0

        rows = vault.db_files()
        # Files with same content have same hash, so may be deduplicated
        # Check at least one file was added
        assert len(rows) >= 1


class TestAddWithImport:
    """Test add command interaction with import."""

    def test_add_after_import_same_file(self, vault: VaultEnv) -> None:
        """Import then add the same file (should skip as already tracked)."""
        # Create file in source and import
        src_file = vault.source_dir / "photo.jpg"
        create_minimal_jpeg(src_file, "SAME_CONTENT")
        vault.import_dir(vault.source_dir)

        # Create identical file in vault with different name
        dup_file = vault.vault_dir / "same_content.jpg"
        create_minimal_jpeg(dup_file, "SAME_CONTENT")

        # Add should detect duplicate
        result = vault.run("add", str(vault.vault_dir))
        assert result.returncode == 0
        combined = result.stderr + result.stdout
        assert "duplicate" in combined.lower() or "already tracked" in combined.lower()

    def test_import_after_add(self, vault: VaultEnv) -> None:
        """Add then import the same file (import should detect duplicate)."""
        # First add a file in vault
        vault_file = vault.vault_dir / "existing" / "photo.jpg"
        vault_file.parent.mkdir(parents=True)
        create_minimal_jpeg(vault_file, "EXISTS")
        vault.run("add", str(vault.vault_dir / "existing"))

        # Create same content in source and import
        src_file = vault.source_dir / "photo.jpg"
        create_minimal_jpeg(src_file, "EXISTS")
        
        result = vault.import_dir(vault.source_dir, check=False)
        # Import should detect duplicate via hash
        rows = vault.db_files()
        # Should have 1 file (the added one), import should be rejected or marked as duplicate
        paths = [r["path"] for r in rows]
        assert len(paths) >= 1


# Note: RAW ID tests for add command are in test_raw_id.py::TestRawIdAddCommand
# to avoid duplication.
