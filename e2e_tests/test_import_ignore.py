"""Tests for import ignore/safety behavior.

Covers cases where the import source overlaps with the vault itself,
ensuring svault does not recursively import its own metadata or archived files.
"""

from __future__ import annotations

import json
import sqlite3
import tempfile
from pathlib import Path

from conftest import VaultEnv


class TestImportIgnoresVault:
    """Test that import skips the vault when source is an ancestor."""

    def test_import_from_ancestor_skips_vault(self, vault: VaultEnv) -> None:
        """Importing from a parent directory must not re-import vault files.

        Simulates the `svault import ../../` scenario where the source
        directory happens to contain the vault root as a sub-directory.
        """
        with tempfile.TemporaryDirectory() as tmp:
            tree = Path(tmp)
            vault_sub = tree / "myvault"
            src_sub = tree / "my_source"
            vault_sub.mkdir()
            src_sub.mkdir()

            # Initialize a fresh vault inside the temp tree
            vault.run("init", cwd=vault_sub, check=True)

            # Source file to import
            header = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00"
            (src_sub / "outside.jpg").write_bytes(header + b"A" * 20)

            # File placed directly inside the vault — must be ignored
            (vault_sub / "existing.jpg").write_bytes(header + b"B" * 20)

            # Import from the ancestor directory
            r = vault.run(
                "import",
                "--yes",
                "--output=json",
                "--target",
                str(vault_sub),
                str(tree),
            )
            assert r.returncode == 0, f"Import failed: {r.stderr}"
            data = json.loads(r.stdout)

            # Only the external file should be imported
            assert data["imported"] == 1, f"Expected 1 imported, got {data}"
            assert data["total"] == 1

            # Verify DB was not polluted by vault-internal files
            db_path = vault_sub / ".svault" / "vault.db"
            conn = sqlite3.connect(str(db_path))
            try:
                rows = conn.execute(
                    "SELECT path FROM files WHERE path LIKE '%.svault%'"
                ).fetchall()
                assert len(rows) == 0, f"Vault metadata leaked into DB: {rows}"

                rows = conn.execute(
                    "SELECT path FROM files WHERE path LIKE '%existing.jpg%'"
                ).fetchall()
                assert len(rows) == 0, f"Vault file was re-imported: {rows}"
            finally:
                conn.close()
