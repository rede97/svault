"""Cross-platform path compatibility tests.

Verifies that vault databases are portable between Windows and Linux
by ensuring all paths are stored in Unix format (forward slashes).
"""

from __future__ import annotations

import json
from pathlib import PurePosixPath

import pytest

from conftest import VaultEnv, copy_fixture


class TestCrossPlatformPathCompatibility:
    """Cross-platform path compatibility tests."""

    def test_imported_paths_use_unix_format(self, vault: VaultEnv) -> None:
        """Imported file paths should be stored with forward slashes."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        # Get session and items
        result = vault.run("history", "sessions", "--output=json", capture=True)
        events = [json.loads(line) for line in result.stdout.strip().split('\n') if line]
        session_items = [e for e in events if e.get("event") == "history_sessions_item"]
        assert len(session_items) > 0
        
        session_id = session_items[0]["session_id"]
        
        # Query items
        result = vault.run(
            "history", "items", f"--session={session_id}", "--output=json", capture=True
        )
        item_events = [json.loads(line) for line in result.stdout.strip().split('\n') if line]
        item_rows = [e for e in item_events if e.get("event") == "history_items_item"]
        
        assert len(item_rows) > 0, "Should have at least one item"
        
        for item in item_rows:
            vault_path = item.get("vault_path", "")
            # Strong assertion: explicitly forbid backslashes
            assert '\\' not in vault_path, (
                f"Path should use forward slashes, got: {vault_path}"
            )
            # Verify path matches PurePosixPath format
            expected = PurePosixPath(vault_path).as_posix()
            assert vault_path == expected, (
                f"Path {vault_path} does not match PurePosixPath format {expected}"
            )

    def test_manifest_paths_are_unix_format(self, vault: VaultEnv) -> None:
        """Manifest JSON should store paths in Unix format."""
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "no_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        # Find manifest file
        manifests_dir = vault.vault_dir / ".svault" / "manifests"
        if not manifests_dir.exists():
            pytest.skip("No manifests directory found")
        
        manifest_files = list(manifests_dir.glob("*.json"))
        if not manifest_files:
            pytest.skip("No manifest files found")
        
        # Read and verify manifest
        for manifest_file in manifest_files:
            with open(manifest_file, 'r') as f:
                manifest = json.load(f)
            
            for file_record in manifest.get("files", []):
                # Check dest_path (relative, should be Unix format)
                dest_path = file_record.get("dest_path", "")
                
                if dest_path:
                    assert '\\' not in str(dest_path), (
                        f"dest_path should use forward slashes: {dest_path}"
                    )
                    # Verify PurePosixPath format
                    expected = PurePosixPath(str(dest_path)).as_posix()
                    assert str(dest_path) == expected, (
                        f"dest_path {dest_path} does not match PurePosixPath format"
                    )

    def test_path_consistency_between_db_and_manifest(self, vault: VaultEnv) -> None:
        """Database and manifest should use consistent path formats."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        # Get history items
        result = vault.run("history", "sessions", "--output=json", capture=True)
        events = [json.loads(line) for line in result.stdout.strip().split('\n') if line]
        session_items = [e for e in events if e.get("event") == "history_sessions_item"]
        
        if not session_items:
            pytest.skip("No sessions found")
        
        session_id = session_items[0]["session_id"]
        
        result = vault.run(
            "history", "items", f"--session={session_id}", "--output=json", capture=True
        )
        item_events = [json.loads(line) for line in result.stdout.strip().split('\n') if line]
        item_rows = [e for e in item_events if e.get("event") == "history_items_item"]
        
        # Read manifest
        manifests_dir = vault.vault_dir / ".svault" / "manifests"
        if not manifests_dir.exists():
            pytest.skip("No manifests directory found")
        
        manifest_files = list(manifests_dir.glob(f"import-{session_id}.json"))
        if not manifest_files:
            pytest.skip("Manifest not found for session")
        
        with open(manifest_files[0], 'r') as f:
            manifest = json.load(f)
        
        # Compare paths
        manifest_dest_paths = {
            f.get("dest_path", "") for f in manifest.get("files", [])
            if f.get("dest_path")
        }
        
        history_vault_paths = {item.get("vault_path", "") for item in item_rows}
        
        # Paths should be consistent (both use Unix format)
        common_paths = manifest_dest_paths & history_vault_paths
        assert len(common_paths) > 0 or len(manifest_dest_paths) == 0, (
            "Paths in DB and manifest should be consistent"
        )

    def test_verify_can_find_files_with_unix_paths(self, vault: VaultEnv) -> None:
        """Verify command should work with Unix-style paths in database."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        # Run verify - it uses DB paths to find files
        result = vault.run("verify", "--output=json", capture=True)
        assert result.returncode == 0
        
        # If paths were wrong format, verify would fail to find files
        verify_events = [json.loads(line) for line in result.stdout.strip().split('\n') if line]
        
        # Check for missing files - if paths are wrong, files would be reported missing
        missing_events = [
            e for e in verify_events 
            if e.get("event") == "verify_item" and e.get("status") == "missing"
        ]
        
        # Should not have missing files for freshly imported content
        assert len(missing_events) == 0, (
            f"Files reported missing - path format issue? {missing_events}"
        )
