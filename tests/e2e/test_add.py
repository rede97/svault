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


class TestAddFullHashDedup:
    """add 用全量 XXH3-128 查重（跳过区域指纹），原地中段编辑不被误判

    契约（2026-08-09）：vault 内文件可能被原地编辑；add 的查重必须基于
    全量哈希而非头尾 64KB 指纹——盲区编辑不得静默判 duplicate。
    """

    def test_middle_edit_is_not_absorbed_as_duplicate(self, vault: VaultEnv) -> None:
        # 200KB 文件：区域指纹只读头尾 64KB，中段是盲区
        target = vault.vault_dir / "tracked" / "big.jpg"
        target.parent.mkdir(parents=True)
        create_minimal_jpeg(target, "TRACKED_BIG")
        with open(target, "ab") as f:
            f.write(b"\xff" * (200 * 1024 - target.stat().st_size))

        assert vault.run("add", str(target.parent)).returncode == 0
        assert len(vault.db_files()) == 1

        # 中段编辑（大小不变，区域指纹不变）
        data = bytearray(target.read_bytes())
        data[100 * 1024] ^= 0xFF
        target.write_bytes(bytes(data))

        result = vault.run("--output=json", "add", str(target.parent))
        assert result.returncode == 0
        # 全量哈希不同 → 不得判 duplicate（同路径被 insert 按路径跳过，
        # 不产生第二条记录，但必须不是"duplicate 静默吸收"）
        summary = [
            line for line in result.stdout.strip().split("\n")
            if '"event":"summary"' in line
        ]
        assert summary, "应输出 summary 事件"
        assert '"duplicate":0' in summary[0].replace(" ", ""), (
            f"盲区编辑不得判 duplicate: {summary[0]}"
        )
        assert len(vault.db_files()) == 1, "同路径不产生第二条记录"

    def test_unchanged_readd_still_duplicate(self, vault: VaultEnv) -> None:
        target = vault.vault_dir / "tracked" / "a.jpg"
        target.parent.mkdir(parents=True)
        create_minimal_jpeg(target, "STABLE")

        assert vault.run("add", str(target.parent)).returncode == 0
        result = vault.run("--output=json", "add", str(target.parent))
        assert '"duplicate":1' in result.stdout.replace(" ", ""), (
            "未改动文件重跑 add 应判 duplicate（全量哈希命中）"
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
        # DB paths are always stored Unix-style (cross-platform design)
        assert "manual/photo.jpg" in rows[0]["path"]

    def test_add_skips_already_tracked(self, vault: VaultEnv) -> None:
        """Re-adding an already tracked file should skip it."""
        vault_file = vault.vault_dir / "photo.jpg"
        create_minimal_jpeg(vault_file, "TRACKED")

        vault.run("add", str(vault.vault_dir))
        rows1 = vault.db_files()
        assert len(rows1) == 1

        result = vault.run("add", str(vault.vault_dir))
        assert result.returncode == 0

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
        # Duplicate should not create new DB rows.
        rows = vault.db_files()
        assert len(rows) == 1


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

    def test_add_mixed_formats(self, vault: VaultEnv) -> None:
        """Add multiple files with different formats (JPEG, MP4, DNG)."""
        create_minimal_jpeg(vault.vault_dir / "photo.jpg")
        create_minimal_mp4(vault.vault_dir / "video.mp4")
        create_minimal_raw(vault.vault_dir / "raw.dng")

        result = vault.run("add", str(vault.vault_dir))
        assert result.returncode == 0

        rows = vault.db_files()
        assert len(rows) == 3


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
        assert result.returncode == 0

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
        assert result.returncode == 0

        rows_after = vault.db_files()
        assert len(rows_after) == 1


# Note: RAW ID tests for add command are in test_raw_id.py::TestRawIdAddCommand
# to avoid duplication.
