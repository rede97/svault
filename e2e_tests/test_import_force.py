"""Tests for `svault import --force`.

Covers the behavior of force-importing duplicates, conflict resolution
when the same filename maps to the same vault path, and post-force verify.
"""

from __future__ import annotations

import json
import time

from conftest import VaultEnv


class TestForceImport:
    """Test `import --force` behavior."""

    def test_force_import_duplicate(self, vault: VaultEnv, source_factory: callable) -> None:
        """Force-importing an exact duplicate should overwrite the vault file.

        Because `files.path` is unique, the DB still contains exactly one row,
        but the file on disk is re-copied and verify remains clean.
        """
        source_factory(
            "photo.jpg",
            exif_date="2024:01:01 12:00:00",
            exif_make="Apple",
        )

        # First import
        r1 = vault.import_dir(vault.source_dir)
        assert r1.returncode == 0
        data1 = json.loads(r1.stdout)
        assert data1["imported"] == 1

        # Second import without force should be skipped as duplicate
        r2 = vault.import_dir(vault.source_dir)
        assert r2.returncode == 0
        data2 = json.loads(r2.stdout)
        assert data2["duplicate"] == 1
        assert data2["imported"] == 0

        # Third import with force should re-process the file
        r3 = vault.import_dir(vault.source_dir, force=True)
        assert r3.returncode == 0
        data3 = json.loads(r3.stdout)
        assert data3["imported"] == 1

        # Database still has exactly one row for this path
        rows = vault.find_file_in_db("photo.jpg")
        assert len(rows) == 1
        assert rows[0]["status"] == "imported"

        # Only one file on disk (overwrite)
        files = vault.get_vault_files("photo.jpg")
        assert len(files) == 1

        # Verify passes
        v = vault.run("verify")
        assert v.returncode == 0

    def test_force_import_same_name_different_content(
        self, vault: VaultEnv, source_factory: callable
    ) -> None:
        """Two different files that resolve to the same vault path.

        Force-importing the second should trigger auto-rename so both coexist.
        """
        common_mtime = time.time()
        source_factory(
            "IMG.jpg",
            content=b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00" + b"A" * 20,
            mtime=common_mtime,
            subdir="dir_a",
        )
        source_factory(
            "IMG.jpg",
            content=b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00" + b"B" * 20,
            mtime=common_mtime,
            subdir="dir_b",
        )

        # Import first directory
        r1 = vault.import_dir(vault.source_dir / "dir_a")
        assert r1.returncode == 0
        data1 = json.loads(r1.stdout)
        assert data1["imported"] == 1
        hash1 = vault.find_file_in_db("IMG.jpg")[0]["xxh3_128"]

        # Import second directory with force
        r2 = vault.import_dir(vault.source_dir / "dir_b", force=True)
        assert r2.returncode == 0
        data2 = json.loads(r2.stdout)
        assert data2["imported"] == 1

        # Both should exist in vault; one should have been auto-renamed
        vault_files = vault.get_vault_files("IMG*.jpg")
        assert len(vault_files) == 2
        basenames = {f.name for f in vault_files}
        assert "IMG.jpg" in basenames
        # Default rename_template is "$filename.$n.$ext" => IMG.1.jpg
        assert "IMG.1.jpg" in basenames

        # Both should be in DB as imported (two rows for two distinct paths)
        db_rows = vault.db_query(
            "SELECT * FROM files WHERE path LIKE '%IMG%.jpg%' AND status = 'imported'"
        )
        assert len(db_rows) == 2
        hashes = {r["xxh3_128"] for r in db_rows}
        assert len(hashes) == 2
        assert hash1 in hashes

        # Verify should pass
        v = vault.run("verify")
        assert v.returncode == 0

    def test_force_import_recovers_deleted_file(
        self, vault: VaultEnv, source_factory: callable
    ) -> None:
        """Force-importing after the vault copy was deleted should restore it."""
        source_factory(
            "photo.jpg",
            exif_date="2024:01:01 12:00:00",
            exif_make="Apple",
        )

        # First import
        r1 = vault.import_dir(vault.source_dir)
        assert r1.returncode == 0

        # Delete the imported vault file
        vault_files = vault.get_vault_files("photo.jpg")
        assert len(vault_files) == 1
        vault_files[0].unlink()

        # Re-import with force
        r2 = vault.import_dir(vault.source_dir, force=True)
        assert r2.returncode == 0
        data2 = json.loads(r2.stdout)
        assert data2["imported"] == 1

        # File restored
        restored = vault.get_vault_files("photo.jpg")
        assert len(restored) == 1

        # Verify passes
        v = vault.run("verify")
        assert v.returncode == 0
