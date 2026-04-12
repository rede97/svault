"""Tests for `svault reconcile` — recovering moved or renamed vault files."""

from __future__ import annotations

from conftest import VaultEnv, create_minimal_jpeg


class TestReconcileCommand:
    """End-to-end tests for `svault reconcile`."""

    def test_update_finds_moved_file(self, vault: VaultEnv) -> None:
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
        result = vault.run("update", "--yes", f"--target={vault.vault_dir}")
        assert result.returncode == 0

        # DB path should be updated
        rows = vault.db_files()
        assert len(rows) == 1
        assert "renamed.jpg" in rows[0]["path"]

    def test_update_dry_run_no_changes(self, vault: VaultEnv) -> None:
        """Default dry-run should not modify the database."""
        src_file = vault.source_dir / "photo.jpg"
        create_minimal_jpeg(src_file, "RELOCATE_ME_12345")
        vault.import_dir(vault.source_dir)

        vault_files = vault.get_vault_files("*.jpg")
        old_path = vault_files[0]
        new_path = vault.vault_dir / "renamed.jpg"
        old_path.rename(new_path)

        original_path = vault.db_files()[0]["path"]

        # Run without --yes
        result = vault.run("update", f"--target={vault.vault_dir}")
        assert result.returncode == 0

        # DB should be unchanged
        rows = vault.db_files()
        assert rows[0]["path"] == original_path

    def test_update_no_missing_files(self, vault: VaultEnv) -> None:
        """When all files are in place, reconcile should report nothing to do."""
        src_file = vault.source_dir / "photo.jpg"
        create_minimal_jpeg(src_file, "STAY_PUT_12345")
        vault.import_dir(vault.source_dir)

        result = vault.run("update", f"--target={vault.vault_dir}")
        assert result.returncode == 0
        rows = vault.db_files()
        assert len(rows) == 1
        assert rows[0]["status"] == "imported"

    def test_update_renamed_directory_then_verify_passes(self, vault: VaultEnv) -> None:
        """Rename an entire directory inside the vault, reconcile, then verify should pass."""
        from conftest import copy_fixture
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        # Find the imported path (e.g. 2024/05-01/Apple iPhone 15/apple_with_exif.jpg)
        imported = vault.db_files()
        assert len(imported) == 1
        old_db_path = imported[0]["path"]
        # Determine the top-level directory to rename
        top_dir = old_db_path.split("/")[0]

        old_dir = vault.vault_dir / top_dir
        new_dir = vault.vault_dir / (top_dir + "X")
        old_dir.rename(new_dir)

        # Reconcile from the vault root
        result = vault.run("update", "--yes", f"--target={vault.vault_dir}")
        assert result.returncode == 0

        # Verify should now pass (no missing files)
        result = vault.run("verify", capture=True)
        assert result.returncode == 0
        rows = vault.db_files()
        assert all(r["status"] == "imported" for r in rows)

    def test_update_missing_files_marked(self, vault: VaultEnv) -> None:
        """Import a file, delete it from vault, then update should mark it missing."""
        src_file = vault.source_dir / "photo.jpg"
        create_minimal_jpeg(src_file, "DELETE_ME_12345")
        vault.import_dir(vault.source_dir)

        # Delete file from vault (simulate external deletion)
        vault_files = vault.get_vault_files("*.jpg")
        assert len(vault_files) == 1
        vault_files[0].unlink()

        # Update with --yes (clean is default behavior)
        result = vault.run("update", "--yes", f"--target={vault.vault_dir}")
        assert result.returncode == 0

        # DB status should be 'missing'
        rows = vault.db_query("SELECT status FROM files WHERE path LIKE '%.jpg%'")
        assert len(rows) == 1
        assert rows[0]["status"] == "missing"

    def test_update_missing_dry_run_no_changes(self, vault: VaultEnv) -> None:
        """update --dry-run should not modify the database for missing files."""
        src_file = vault.source_dir / "photo.jpg"
        create_minimal_jpeg(src_file, "CLEAN_TEST_12345")
        vault.import_dir(vault.source_dir)

        # Delete file from vault
        vault_files = vault.get_vault_files("*.jpg")
        vault_files[0].unlink()

        original_status = vault.db_files()[0]["status"]

        # Run --dry-run (preview mode)
        result = vault.run("update", "--dry-run", f"--target={vault.vault_dir}")
        assert result.returncode == 0

        # DB should be unchanged
        rows = vault.db_files()
        assert rows[0]["status"] == original_status

    def test_update_after_recover_and_delete(self, vault: VaultEnv) -> None:
        """Recover a missing file, then delete it again - update should mark it missing.
        
        This tests the scenario where:
        1. File is imported
        2. File is deleted from vault, update marks it as 'missing'
        3. File is re-imported (Recover), status becomes 'imported'
        4. File is deleted again
        5. Update should find it and mark as 'missing' again
        """
        src_file = vault.source_dir / "photo.jpg"
        create_minimal_jpeg(src_file, "RECOVER_TEST_12345")
        
        # Step 1: Initial import
        vault.import_dir(vault.source_dir)
        rows = vault.db_files()
        assert len(rows) == 1
        assert rows[0]["status"] == "imported"
        
        # Step 2: Delete file from vault and run update to mark as missing
        vault_files = vault.get_vault_files("*.jpg")
        assert len(vault_files) == 1
        vault_path = vault_files[0]
        vault_path.unlink()
        
        result = vault.run("update", "--yes", f"--target={vault.vault_dir}")
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert rows[0]["status"] == "missing"
        
        # Step 3: Re-import the same file (should trigger Recover)
        # First restore the source file
        src_file.unlink()
        create_minimal_jpeg(src_file, "RECOVER_TEST_12345")
        vault.import_dir(vault.source_dir)
        
        rows = vault.db_files()
        assert rows[0]["status"] == "imported"
        print(f"After recover: path={rows[0]['path']}, status={rows[0]['status']}")
        
        # Step 4: Delete the recovered file from vault
        vault_files = vault.get_vault_files("*.jpg")
        assert len(vault_files) == 1
        vault_files[0].unlink()
        
        # Step 5: Run update again - should find the file and mark as missing
        result = vault.run("update", "--yes", f"--target={vault.vault_dir}")
        print(f"Second update output: {result.stdout}")
        print(f"Second update stderr: {result.stderr}")
        assert result.returncode == 0
        
        rows = vault.db_files()
        print(f"After second update: path={rows[0]['path']}, status={rows[0]['status']}")
        assert rows[0]["status"] == "missing", f"Expected 'missing', got '{rows[0]['status']}'"

    def test_update_after_recover_and_move(self, vault: VaultEnv) -> None:
        """Recover a missing file, then move it - update should find and fix the path.
        
        This tests the scenario where:
        1. File is imported
        2. File is deleted from vault, update marks it as 'missing'
        3. File is re-imported (Recover), status becomes 'imported'
        4. File is renamed/moved inside vault
        5. Update should find it and update the path
        """
        src_file = vault.source_dir / "photo.jpg"
        create_minimal_jpeg(src_file, "RECOVER_MOVE_12345")
        
        # Step 1: Initial import
        vault.import_dir(vault.source_dir)
        
        # Step 2: Delete file from vault and run update to mark as missing
        vault_files = vault.get_vault_files("*.jpg")
        vault_path = vault_files[0]
        vault_path.unlink()
        
        result = vault.run("update", "--yes", f"--target={vault.vault_dir}")
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert rows[0]["status"] == "missing"
        
        # Step 3: Re-import the same file (should trigger Recover)
        src_file.unlink()
        create_minimal_jpeg(src_file, "RECOVER_MOVE_12345")
        vault.import_dir(vault.source_dir)
        
        rows = vault.db_files()
        assert rows[0]["status"] == "imported"
        
        # Step 4: Rename the recovered file inside vault
        vault_files = vault.get_vault_files("*.jpg")
        old_path = vault_files[0]
        new_path = vault.vault_dir / "moved_photo.jpg"
        old_path.rename(new_path)
        
        # Step 5: Run update - should find and fix the path
        result = vault.run("update", "--yes", f"--target={vault.vault_dir}")
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert "moved_photo.jpg" in rows[0]["path"]
        assert rows[0]["status"] == "imported"
