"""Tests for `svault reconcile` — recovering moved or renamed vault files."""

from __future__ import annotations

from conftest import VaultEnv, create_minimal_jpeg


class TestReconcileCommand:
    """End-to-end tests for `svault reconcile`."""

    def test_reconcile_finds_moved_file(self, vault: VaultEnv) -> None:
        """Import a file, rename it inside the vault, then reconcile."""
        src_file = vault.source_dir / "photo.jpg"
        create_minimal_jpeg(src_file, "RELOCATE_ME_12345")
        vault.import_dir(vault.source_dir)

        # Rename inside vault
        vault_files = vault.get_vault_files("*.jpg")
        assert len(vault_files) == 1
        old_path = vault_files[0]
        new_path = vault.vault_dir / "renamed.jpg"
        old_path.rename(new_path)
        assert not old_path.exists()
        assert new_path.exists()

        # Reconcile with --yes
        result = vault.run("reconcile", "--yes", f"--target={vault.vault_dir}")
        assert result.returncode == 0
        combined = result.stderr + result.stdout
        assert "Matched:" in combined
        assert "Updated:" in combined
        assert "renamed.jpg" in combined

        # DB path should be updated
        rows = vault.db_files()
        assert len(rows) == 1
        assert "renamed.jpg" in rows[0]["path"]

    def test_reconcile_dry_run_no_changes(self, vault: VaultEnv) -> None:
        """Default dry-run should not modify the database."""
        src_file = vault.source_dir / "photo.jpg"
        create_minimal_jpeg(src_file, "RELOCATE_ME_12345")
        vault.import_dir(vault.source_dir)

        vault_files = vault.get_vault_files("*.jpg")
        old_path = vault_files[0]
        new_path = vault.vault_dir / "renamed.jpg"
        old_path.rename(new_path)

        original_path = vault.db_files()[0]["path"]

        # Run without --yes (dry-run by implication, but reconcile doesn't have explicit dry-run flag in CLI)
        # Actually reconcile uses `cli.dry_run` and `cli.yes` — let's run without --yes
        result = vault.run("reconcile", f"--target={vault.vault_dir}")
        assert result.returncode == 0
        combined = result.stderr + result.stdout
        assert "Matches found:" in combined

        # DB should be unchanged
        rows = vault.db_files()
        assert rows[0]["path"] == original_path

    def test_reconcile_no_missing_files(self, vault: VaultEnv) -> None:
        """When all files are in place, reconcile should report nothing to do."""
        src_file = vault.source_dir / "photo.jpg"
        create_minimal_jpeg(src_file, "STAY_PUT_12345")
        vault.import_dir(vault.source_dir)

        result = vault.run("reconcile", f"--target={vault.vault_dir}")
        assert result.returncode == 0
        combined = result.stderr + result.stdout
        assert "nothing to reconcile" in combined.lower() or "All tracked files exist" in combined
