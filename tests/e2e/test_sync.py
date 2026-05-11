"""Sync command tests.

Tests the svault sync functionality which copies files between two vaults
using SHA-256 content-based deduplication.
"""

from __future__ import annotations

import json
import os
import shutil
from pathlib import Path

import pytest

from conftest import VaultEnv, copy_fixture, FIXTURES_DIR


def init_second_vault(binary: Path, vault_dir: Path) -> None:
    """Initialize a second vault at the given path."""
    svault_meta = vault_dir / ".svault"
    if svault_meta.exists():
        shutil.rmtree(svault_meta)
    config = vault_dir / "svault.toml"
    if config.exists():
        config.unlink()

    import subprocess
    subprocess.run(
        [str(binary), "init"],
        check=True, text=True, capture_output=True, cwd=str(vault_dir),
    )


class TestSyncBasic:
    """Basic sync functionality tests."""

    def test_sync_basic_success(self, vault: VaultEnv) -> None:
        """Basic sync should succeed and copy files from source to target vault."""
        # Import files into source vault
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "no_exif.jpg")
        vault.import_dir(vault.source_dir)

        # Ensure SHA-256 is computed for sync
        vault.run("verify", "--background-hash", capture=True)

        # Create and init target vault
        target_dir = vault.root / "target_vault"
        target_dir.mkdir(parents=True, exist_ok=True)
        init_second_vault(vault.binary, target_dir)

        # Sync from source vault to target vault (CWD = target)
        result = vault.run(
            "sync", str(vault.vault_dir),
            cwd=target_dir,
            capture=True
        )
        assert result.returncode == 0

        # Check summary
        assert "Transferred:" in result.stdout

        # Verify files exist in target vault DB
        db_path = target_dir / ".svault" / "vault.db"
        assert db_path.exists()

        import sqlite3
        conn = sqlite3.connect(str(db_path))
        conn.row_factory = sqlite3.Row
        rows = conn.execute(
            "SELECT path, status FROM files WHERE status = 'imported'"
        ).fetchall()
        conn.close()

        assert len(rows) >= 2, f"Expected at least 2 files in target, got {len(rows)}"

    def test_sync_json_output(self, vault: VaultEnv) -> None:
        """Sync with --output=json should emit JSON events."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        vault.run("verify", "--background-hash", capture=True)

        target_dir = vault.root / "target_vault_json"
        target_dir.mkdir(parents=True, exist_ok=True)
        init_second_vault(vault.binary, target_dir)

        result = vault.run(
            "sync", str(vault.vault_dir),
            "--output=json",
            cwd=target_dir,
            capture=True
        )
        assert result.returncode == 0

        lines = [l for l in result.stdout.strip().split('\n') if l]
        events = [json.loads(l) for l in lines]
        event_types = [e["event"] for e in events]

        assert "sync_diff_started" in event_types
        assert "sync_diff_computed" in event_types
        assert "sync_diff_finished" in event_types
        assert "sync_transfer_started" in event_types
        assert "sync_transfer_item_started" in event_types
        assert "sync_transfer_item_finished" in event_types
        assert "sync_transfer_finished" in event_types
        assert "sync_transfer_summary" in event_types

    def test_sync_nothing_to_sync(self, vault: VaultEnv) -> None:
        """Sync when target already has all files should report nothing to sync."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        vault.run("verify", "--background-hash", capture=True)

        target_dir = vault.root / "target_vault_nothing"
        target_dir.mkdir(parents=True, exist_ok=True)
        init_second_vault(vault.binary, target_dir)

        # First sync
        result1 = vault.run(
            "sync", str(vault.vault_dir),
            "--output=json",
            cwd=target_dir,
            capture=True
        )
        assert result1.returncode == 0

        # Second sync — nothing new
        result2 = vault.run(
            "sync", str(vault.vault_dir),
            "--output=json",
            cwd=target_dir,
            capture=True
        )
        assert result2.returncode == 0

        events = [json.loads(l) for l in result2.stdout.strip().split('\n') if l]
        event_types = [e["event"] for e in events]
        assert "sync_diff_nothing_to_sync" in event_types

    def test_sync_empty_source(self, vault: VaultEnv) -> None:
        """Sync from empty source vault should report nothing to sync."""
        target_dir = vault.root / "target_vault_empty_src"
        target_dir.mkdir(parents=True, exist_ok=True)
        init_second_vault(vault.binary, target_dir)

        result = vault.run(
            "sync", str(vault.vault_dir),
            "--output=json",
            cwd=target_dir,
            capture=True
        )
        assert result.returncode == 0

        events = [json.loads(l) for l in result.stdout.strip().split('\n') if l]
        event_types = [e["event"] for e in events]
        assert "sync_diff_nothing_to_sync" in event_types

    def test_sync_preserves_directory_structure(self, vault: VaultEnv) -> None:
        """Sync should preserve vault directory structure on target."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        vault.run("verify", "--background-hash", capture=True)

        target_dir = vault.root / "target_vault_structure"
        target_dir.mkdir(parents=True, exist_ok=True)
        init_second_vault(vault.binary, target_dir)

        result = vault.run(
            "sync", str(vault.vault_dir),
            cwd=target_dir,
            capture=True
        )
        assert result.returncode == 0

        import sqlite3
        db_path = target_dir / ".svault" / "vault.db"
        conn = sqlite3.connect(str(db_path))
        conn.row_factory = sqlite3.Row
        rows = conn.execute(
            "SELECT path FROM files WHERE status = 'imported'"
        ).fetchall()
        conn.close()

        for row in rows:
            target_file = target_dir / row["path"]
            assert target_file.exists(), f"Target file should exist: {target_file}"

    def test_sync_file_content_matches(self, vault: VaultEnv) -> None:
        """Synced files should have identical content to source."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        vault.run("verify", "--background-hash", capture=True)

        target_dir = vault.root / "target_vault_content"
        target_dir.mkdir(parents=True, exist_ok=True)
        init_second_vault(vault.binary, target_dir)

        result = vault.run(
            "sync", str(vault.vault_dir),
            cwd=target_dir,
            capture=True
        )
        assert result.returncode == 0

        import sqlite3
        db_path = target_dir / ".svault" / "vault.db"
        conn = sqlite3.connect(str(db_path))
        conn.row_factory = sqlite3.Row
        rows = conn.execute(
            "SELECT path FROM files WHERE status = 'imported'"
        ).fetchall()
        conn.close()

        for row in rows:
            source_file = vault.vault_dir / row["path"]
            target_file = target_dir / row["path"]
            assert source_file.read_bytes() == target_file.read_bytes()


class TestSyncTransfer:
    """Sync transfer behavior tests."""

    def test_sync_skips_existing_files(self, vault: VaultEnv) -> None:
        """Sync should skip files that already exist in target vault."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        vault.run("verify", "--background-hash", capture=True)

        target_dir = vault.root / "target_vault_skip"
        target_dir.mkdir(parents=True, exist_ok=True)
        init_second_vault(vault.binary, target_dir)

        # First sync
        vault.run("sync", str(vault.vault_dir), cwd=target_dir, capture=True)

        # Second sync — should find nothing new
        result = vault.run(
            "sync", str(vault.vault_dir),
            "--output=json",
            cwd=target_dir,
            capture=True
        )
        assert result.returncode == 0

        events = [json.loads(l) for l in result.stdout.strip().split('\n') if l]
        event_types = [e["event"] for e in events]
        assert "sync_diff_nothing_to_sync" in event_types


class TestCloneJson:
    """Clone JSON output tests (verifying JSON reporters work)."""

    def test_clone_json_output_events(self, vault: VaultEnv) -> None:
        """Clone with --output=json should emit proper event sequence."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        target_dir = vault.root / "clone_target_json_events"
        result = vault.run(
            "clone", f"--target={target_dir}",
            "--output=json",
            capture=True
        )
        assert result.returncode == 0

        lines = [l for l in result.stdout.strip().split('\n') if l]
        events = [json.loads(l) for l in lines]
        event_types = [e["event"] for e in events]

        assert "clone_started" in event_types
        assert "clone_diff_computed" in event_types
        assert "clone_finished" in event_types
        assert "sync_transfer_started" in event_types
        assert "sync_transfer_item_started" in event_types
        assert "sync_transfer_item_finished" in event_types
        assert "sync_transfer_finished" in event_types
        assert "sync_transfer_summary" in event_types
        assert "clone_summary" in event_types

    def test_clone_json_nothing_to_clone(self, vault: VaultEnv) -> None:
        """Clone with --output=json should emit nothing_to_clone for empty vault."""
        target_dir = vault.root / "clone_target_empty_json"

        result = vault.run(
            "clone", f"--target={target_dir}",
            "--output=json",
            capture=True
        )
        assert result.returncode == 0

        lines = [l for l in result.stdout.strip().split('\n') if l]
        events = [json.loads(l) for l in lines]

        event_types = [e["event"] for e in events]
        assert "clone_started" in event_types
        assert "clone_nothing_to_clone" in event_types
        assert "clone_finished" in event_types
