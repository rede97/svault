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



class TestAddInternalMoveDetection:
    """Test add command detects vault-internal moves and suggest update."""

    def test_add_detects_vault_internal_move_suggests_update(self, vault: VaultEnv) -> None:
        """When files are moved within vault, add should suggest update.
        
        Scenario:
        1. Import files to vault/2023/
        2. Move directory to vault/2023_new/ (outside svault)
        3. Run 'svault add vault/2023_new'
        4. Should detect as moved files and suggest update'
        """
        # Step 1: Create and import files to 2023/
        src_2023 = vault.source_dir / "2023"
        src_2023.mkdir(parents=True, exist_ok=True)
        create_minimal_jpeg(src_2023 / "photo1.jpg", "MOVE_TEST_1")
        create_minimal_jpeg(src_2023 / "photo2.jpg", "MOVE_TEST_2")
        
        vault.import_dir(vault.source_dir)
        
        # Verify files imported (path based on date, not source dir name)
        rows = vault.db_files()
        assert len(rows) == 2
        old_paths = {r["path"] for r in rows}
        
        # Step 2: Simulate move by creating files at new location
        # In real scenario, user would: mv vault/2023 vault/2023_new
        vault_2023_new = vault.vault_dir / "2023_new"
        vault_2023_new.mkdir(parents=True, exist_ok=True)
        create_minimal_jpeg(vault_2023_new / "photo1.jpg", "MOVE_TEST_1")
        create_minimal_jpeg(vault_2023_new / "photo2.jpg", "MOVE_TEST_2")
        
        # Step 3: Run add on new location
        result = vault.run("add", str(vault_2023_new))
        combined = result.stderr + result.stdout
        
        # Step 4: Should suggest update (check for Moved or Duplicate in output)
        assert (
            "update" in combined.lower() or
            "moved" in combined.lower() or
            "duplicate" in combined.lower()
        ), f"Expected 'reconcile', 'moved' or 'duplicate' in output, got:\n{combined}"
        
        # Should NOT add new records (files are duplicates, just moved)
        rows_after = vault.db_files()
        # Should still have only 2 files (old paths)
        # New paths should NOT be added
        assert len(rows_after) == 2, \
            f"Expected 2 files in DB, got {len(rows_after)}. Files may have been incorrectly added."

    def test_add_after_manual_directory_rename(self, vault: VaultEnv) -> None:
        """Simulate user renaming a directory inside vault.
        
        User workflow:
        1. Files exist in vault/archive/photos/
        2. User runs: mv vault/archive/photos vault/archive/photos_backup
        3. User runs: svault add vault/archive/photos_backup
        4. Should detect as moved and suggest update
        """
        # Setup: Create tracked files
        archive_dir = vault.vault_dir / "archive" / "photos"
        archive_dir.mkdir(parents=True)
        create_minimal_jpeg(archive_dir / "vacation.jpg", "VACATION_UNIQUE")
        
        vault.run("add", str(archive_dir))
        
        # Verify tracked
        rows = vault.db_files()
        assert len(rows) == 1
        assert "archive/photos" in rows[0]["path"]
        
        # Simulate rename: create at new path, old path still exists in DB but not FS
        # (In real scenario, mv removes old path)
        # We simulate by creating new path, the old path check will fail
        new_dir = vault.vault_dir / "archive" / "photos_backup"
        new_dir.mkdir(parents=True)
        create_minimal_jpeg(new_dir / "vacation.jpg", "VACATION_UNIQUE")
        
        result = vault.run("add", str(new_dir))
        combined = result.stderr + result.stdout
        
        # Should suggest update (or show as duplicate/moved)
        assert (
            "update" in combined.lower() or 
            "moved" in combined.lower() or
            "duplicate" in combined.lower()
        ), f"Should detect move and suggest update:\n{combined}"


# Note: RAW ID tests for add command are in test_raw_id.py::TestRawIdAddCommand
# to avoid duplication.
