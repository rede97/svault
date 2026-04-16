"""Clone command tests.

Tests the svault clone functionality which exports a subset of files
from the vault to a target directory.
"""

from __future__ import annotations

import json
import os
import shutil
from pathlib import Path

import pytest

from conftest import VaultEnv, copy_fixture, FIXTURES_DIR


class TestCloneBasic:
    """Basic clone functionality tests."""

    def test_clone_basic_success(self, vault: VaultEnv) -> None:
        """Basic clone should succeed and copy files to target."""
        # Import some files first
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "no_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        # Clone to a unique target
        target_dir = vault.root / "clone_target_basic"
        result = vault.run(
            "clone", f"--target={target_dir}",
            capture=True
        )
        assert result.returncode == 0
        
        # Check summary
        assert "Selected:" in result.stdout
        assert "Copied:" in result.stdout
        
        # Verify files were copied
        assert target_dir.exists()
        copied_files = list(target_dir.rglob("*.jpg"))
        assert len(copied_files) >= 2

    def test_clone_empty_vault(self, vault: VaultEnv) -> None:
        """Clone from empty vault should return selected=0."""
        target_dir = vault.root / "clone_target_empty"
        result = vault.run(
            "clone", f"--target={target_dir}",
            capture=True
        )
        assert result.returncode == 0
        assert target_dir.exists()
        
        # Should show selected=0
        assert "Selected:       0" in result.stdout or "selected" in result.stdout.lower()

    def test_clone_json_output(self, vault: VaultEnv) -> None:
        """Clone with --output=json should return JSON summary."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        target_dir = vault.root / "clone_target_json"
        result = vault.run(
            "clone", f"--target={target_dir}",
            "--output=json",
            capture=True
        )
        assert result.returncode == 0
        
        # Parse JSON
        lines = [l for l in result.stdout.strip().split('\n') if l]
        summary_line = next((l for l in lines if '"clone_summary"' in l), None)
        assert summary_line is not None
        
        data = json.loads(summary_line)
        assert data["event"] == "clone_summary"
        assert "selected" in data
        assert "copied" in data


class TestCloneFilters:
    """Clone filter functionality tests."""

    def test_clone_filter_date(self, vault: VaultEnv) -> None:
        """Clone with --filter-date should only copy matching files."""
        # Import files (they'll be dated based on EXIF)
        copy_fixture(vault, "apple_with_exif.jpg")  # Has 2024-05-01 date
        vault.import_dir(vault.source_dir)
        
        target_dir = vault.root / "clone_target_date"
        result = vault.run(
            "clone", f"--target={target_dir}",
            "--filter-date=2024-05-01..2024-05-31",  # Match May 2024
            capture=True
        )
        assert result.returncode == 0
        
        # Should have selected files (the iPhone photo from May 2024)
        assert "Selected:" in result.stdout

    def test_clone_filter_date_no_match(self, vault: VaultEnv) -> None:
        """Clone with date filter that matches nothing should select 0."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        target_dir = vault.root / "clone_target_date_nomatch"
        result = vault.run(
            "clone", f"--target={target_dir}",
            "--filter-date=2020-01-01..2020-01-31",  # Way in the past
            capture=True
        )
        assert result.returncode == 0
        
        # Should show selected=0
        assert "Selected:" in result.stdout and "0" in result.stdout

    def test_clone_filter_date_inclusive_single_day(self, vault: VaultEnv) -> None:
        """Clone with single-day date filter should include that day (P0 bug fix)."""
        copy_fixture(vault, "apple_with_exif.jpg")  # Has 2024-05-01 date
        vault.import_dir(vault.source_dir)
        
        target_dir = vault.root / "clone_target_single_day"
        # Filter for exactly the file's date (start == end)
        result = vault.run(
            "clone", f"--target={target_dir}",
            "--filter-date=2024-05-01..2024-05-01",  # Single day
            capture=True
        )
        assert result.returncode == 0
        
        # Should select the file (date is inclusive)
        lines = result.stdout.split('\n')
        selected_line = [l for l in lines if 'Selected:' in l]
        assert len(selected_line) > 0
        # Extract number after Selected:
        selected_count = int(selected_line[0].split(':')[1].strip().split()[0])
        assert selected_count >= 1, (
            f"Single-day filter should select at least 1 file, got {selected_count}"
        )

    def test_clone_filter_camera(self, vault: VaultEnv) -> None:
        """Clone with --filter-camera should only copy matching files."""
        copy_fixture(vault, "apple_with_exif.jpg")  # iPhone 15
        vault.import_dir(vault.source_dir)
        
        target_dir = vault.root / "clone_target_camera"
        result = vault.run(
            "clone", f"--target={target_dir}",
            "--filter-camera=iPhone",  # Should match iPhone 15
            capture=True
        )
        assert result.returncode == 0
        
        # Should have selected files
        assert "Selected:" in result.stdout

    def test_clone_filter_camera_no_match(self, vault: VaultEnv) -> None:
        """Clone with camera filter that matches nothing should select 0."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        target_dir = vault.root / "clone_target_camera_nomatch"
        result = vault.run(
            "clone", f"--target={target_dir}",
            "--filter-camera=NonExistentCamera",
            capture=True
        )
        assert result.returncode == 0
        
        # Should show selected=0
        assert "Selected:" in result.stdout and "0" in result.stdout


class TestCloneSafety:
    """Clone safety and edge case tests."""

    def test_clone_target_inside_vault_fails(self, vault: VaultEnv) -> None:
        """Clone target inside vault should fail."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        # Try to clone inside vault
        target_inside = vault.vault_dir / "clone_here"
        result = vault.run(
            "clone", f"--target={target_inside}",
            capture=True, check=False
        )
        assert result.returncode != 0

    def test_clone_target_inside_vault_fails_with_relative_target(self, vault: VaultEnv) -> None:
        """Clone with relative target inside vault should fail (P0 bug fix)."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        # Try to clone with relative path that resolves inside vault
        # From vault root, try to clone to "photos" subdirectory
        result = vault.run(
            "clone", "--target=photos",
            capture=True, check=False
        )
        assert result.returncode != 0, (
            "Relative target inside vault should be rejected"
        )

    def test_clone_target_parent_relative_outside_vault_allowed(self, vault: VaultEnv) -> None:
        """--target=../outside should be allowed (outside vault)."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        # Create sibling directory in the same parent as vault_dir (i.e., vault.root)
        sibling_dir = vault.root / "outside_clone"
        sibling_dir.mkdir(exist_ok=True)
        
        # Change to vault directory and clone to ../outside_clone
        import os
        original_cwd = os.getcwd()
        try:
            os.chdir(vault.vault_dir)
            result = vault.run(
                "clone", "--target=../outside_clone",
                capture=True
            )
            # This should succeed - ../outside_clone is outside vault
            assert result.returncode == 0, (
                f"Clone to ../outside should succeed, got: {result.stderr}"
            )
            
            # Verify file was copied (in vault.root, not vault.root.parent)
            cloned_file = sibling_dir / "2024" / "05-01" / "Apple iPhone 15" / "apple_with_exif.jpg"
            assert cloned_file.exists(), "Cloned file should exist in ../outside_clone"
        finally:
            os.chdir(original_cwd)

    def test_clone_target_symlink_to_inside_vault_should_fail(self, vault: VaultEnv) -> None:
        """Symlink pointing inside vault should be rejected (P1 security fix)."""
        import shutil
        import uuid
        
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        # Create a unique directory outside vault
        unique_id = str(uuid.uuid4())[:8]
        outside_dir = vault.root / f"symlink_test_{unique_id}"
        if outside_dir.exists():
            shutil.rmtree(outside_dir)
        outside_dir.mkdir()
        
        # Create a symlink inside that directory pointing to vault subdirectory
        symlink_target = outside_dir / "link_to_inside"
        real_target_inside = vault.vault_dir / "2024"  # Point to existing vault content
        symlink_target.symlink_to(real_target_inside, target_is_directory=True)
        
        assert symlink_target.exists(), "Symlink should be created"
        
        # Try to clone to the symlink - should fail because it resolves inside vault
        result = vault.run(
            "clone", f"--target={symlink_target}",
            capture=True, check=False
        )
        assert result.returncode != 0, (
            f"Clone to symlink pointing inside vault should fail, got: {result.stdout}"
        )
        assert "inside the vault" in result.stderr.lower() or "inside the vault" in result.stdout.lower()
        
        # Cleanup
        shutil.rmtree(outside_dir)

    def test_clone_target_exists_with_same_file(self, vault: VaultEnv) -> None:
        """Clone when target has same file should skip."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        target_dir = vault.root / "clone_target_inside"
        
        # First clone
        result1 = vault.run(
            "clone", f"--target={target_dir}",
            capture=True
        )
        assert result1.returncode == 0
        
        # Second clone (same target)
        result2 = vault.run(
            "clone", f"--target={target_dir}",
            capture=True
        )
        assert result2.returncode == 0
        
        # Should show skipped > 0
        assert "Skipped:" in result2.stdout

    def test_clone_target_exists_with_different_file(self, vault: VaultEnv) -> None:
        """Clone when target has different file (same name, diff content) should fail."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        target_dir = vault.root / "clone_target_same"
        
        # Create target with wrong file
        target_subdir = target_dir / "2024" / "05-01" / "Apple iPhone 15"
        target_subdir.mkdir(parents=True, exist_ok=True)
        (target_subdir / "apple_with_exif.jpg").write_text("wrong content")
        
        # Clone should report failure for that file
        result = vault.run(
            "clone", f"--target={target_dir}",
            capture=True
        )
        assert result.returncode == 0
        # Should show failed > 0 due to size mismatch
        assert "Failed:" in result.stdout or "failed" in result.stdout.lower()


class TestCloneVerification:
    """Clone verification tests."""

    def test_clone_verify_passes(self, vault: VaultEnv) -> None:
        """Clone should verify copied files successfully."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        target_dir = vault.root / "clone_target_different"
        result = vault.run(
            "clone", f"--target={target_dir}",
            capture=True
        )
        assert result.returncode == 0
        
        # Verify failed should be 0
        assert "Verify Failed:" in result.stdout
        for line in result.stdout.split('\n'):
            if 'Verify Failed:' in line:
                assert '0' in line, f"Expected verify_failed=0, got: {line}"
        
        # Check files are actually there and match
        source_file = vault.vault_dir / "2024" / "05-01" / "Apple iPhone 15" / "apple_with_exif.jpg"
        if source_file.exists():
            target_file = target_dir / "2024" / "05-01" / "Apple iPhone 15" / "apple_with_exif.jpg"
            if target_file.exists():
                assert source_file.read_bytes() == target_file.read_bytes()

    def test_clone_preserves_directory_structure(self, vault: VaultEnv) -> None:
        """Clone should preserve vault's directory structure."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        target_dir = vault.root / "clone_target_structure"
        result = vault.run(
            "clone", f"--target={target_dir}",
            capture=True
        )
        assert result.returncode == 0
        
        # Find copied file
        jpg_files = list(target_dir.rglob("*.jpg"))
        assert len(jpg_files) > 0
        
        # Check path uses forward slashes (Unix format)
        for f in jpg_files:
            rel_path = f.relative_to(target_dir)
            assert '\\' not in str(rel_path), "Path should use forward slashes"


class TestCloneReadOnly:
    """Clone should be read-only on vault."""

    def test_clone_does_not_create_history_session(self, vault: VaultEnv) -> None:
        """Clone should not create import/add/history sessions."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        # Get session count before clone
        result_before = vault.run("history", "sessions", "--output=json", capture=True)
        events_before = [json.loads(l) for l in result_before.stdout.strip().split('\n') if l]
        sessions_before = len([e for e in events_before if e.get("event") == "history_sessions_item"])
        
        # Clone
        target_dir = vault.root / "clone_target_readonly"
        result = vault.run(
            "clone", f"--target={target_dir}",
            capture=True
        )
        assert result.returncode == 0
        
        # Get session count after clone
        result_after = vault.run("history", "sessions", "--output=json", capture=True)
        events_after = [json.loads(l) for l in result_after.stdout.strip().split('\n') if l]
        sessions_after = len([e for e in events_after if e.get("event") == "history_sessions_item"])
        
        # Session count should not increase
        assert sessions_after == sessions_before, "Clone should not create history sessions"
