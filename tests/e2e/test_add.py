"""Tests for `svault add` — registering files already inside the vault."""

from __future__ import annotations

from pathlib import Path

from conftest import VaultEnv, create_minimal_jpeg


class TestAddCommand:
    """End-to-end tests for `svault add`."""

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

    def test_add_detects_duplicates(self, vault: VaultEnv) -> None:
        """Add a file that is a byte-for-byte duplicate of an imported one."""
        # First import from source
        src_file = vault.source_dir / "photo.jpg"
        create_minimal_jpeg(src_file, "DUP_CONTENT_12345")
        vault.import_dir(vault.source_dir)

        # Place identical content inside vault under a different name
        dup_file = vault.vault_dir / "dup.jpg"
        create_minimal_jpeg(dup_file, "DUP_CONTENT_12345")

        result = vault.run("add", str(vault.vault_dir))
        assert result.returncode == 0
        combined = result.stderr + result.stdout
        assert "duplicate" in combined.lower()

        # Only the original imported file should be in DB
        rows = vault.find_file_in_db("dup.jpg")
        assert len(rows) == 0
