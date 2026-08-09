"""Cross-platform path compatibility tests.

Verifies that vault databases are portable between Windows and Linux
by ensuring all paths are stored in Unix format (forward slashes).
"""

from __future__ import annotations

import json
from pathlib import PurePosixPath

import pytest

from conftest import VaultEnv, copy_fixture


def db_file_rows(vault: VaultEnv) -> list[dict]:
    """Query the files table via `svault db dump` (replaces removed `history`)."""
    result = vault.run("db", "dump", "files", "--format=json", capture=True)
    tables = json.loads(result.stdout)
    files = [t for t in tables if t.get("name") == "files"]
    return files[0]["rows"] if files else []


def _load_manifests(vault: VaultEnv) -> list[dict]:
    """Load all session manifests (.svault/sessions/<kind>/<id>/manifest.json)."""
    root = vault.vault_dir / ".svault" / "sessions"
    manifests = [
        json.loads(p.read_text()) for p in sorted(root.glob("*/*/manifest.json"))
    ]
    assert manifests, f"No session manifests found under {root}"
    return manifests


class TestCrossPlatformPathCompatibility:
    """Cross-platform path compatibility tests."""

    def test_imported_paths_use_unix_format(self, vault: VaultEnv) -> None:
        """Imported file paths should be stored with forward slashes."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        # Query imported file records from the database
        rows = db_file_rows(vault)
        assert len(rows) > 0, "Should have at least one file record"
        
        for row in rows:
            vault_path = row.get("path", "")
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
        
        # Read and verify manifests
        for manifest in _load_manifests(vault):
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
        
        # Query imported file records from the database
        rows = db_file_rows(vault)
        if not rows:
            pytest.skip("No file records found")
        
        # Read manifest
        manifest = _load_manifests(vault)[0]
        
        # Compare paths
        manifest_dest_paths = {
            f.get("dest_path", "") for f in manifest.get("files", [])
            if f.get("dest_path")
        }
        
        db_vault_paths = {row.get("path", "") for row in rows}
        
        # Paths should be consistent (both use Unix format)
        common_paths = manifest_dest_paths & db_vault_paths
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
            if e.get("event") == "verify_item" and e.get("result", {}).get("result") == "missing"
        ]
        
        # Should not have missing files for freshly imported content
        assert len(missing_events) == 0, (
            f"Files reported missing - path format issue? {missing_events}"
        )
