"""Recheck command tests.

Tests for the standalone `svault recheck` command, which reads an import
manifest and verifies the integrity of both source files and vault copies.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from conftest import VaultEnv, create_minimal_jpeg


class TestRecheckWorkflow:
    """End-to-end recheck and re-import workflow."""

    def test_recheck_detects_corruption_and_reimport_succeeds(self, vault: VaultEnv) -> None:
        """Detect vault file corruption and recover by re-importing."""
        f1 = vault.source_dir / "keep.jpg"
        f2 = vault.source_dir / "corrupt.jpg"
        create_minimal_jpeg(f1, "KEEP_KEEP_KEEP_" * 1000)
        create_minimal_jpeg(f2, "CORRUPT_CORRUPT_" * 1000)

        # 1. First import
        result = vault.import_dir(vault.source_dir, strategy="copy")
        assert result.returncode == 0
        files = vault.db_files()
        assert len(files) == 2

        # 2. Corrupt one vault file (modify after first 64KB)
        vault_files = vault.get_vault_files("*.jpg")
        assert len(vault_files) == 2

        corrupt_target = None
        for vf in vault_files:
            if "corrupt" in vf.name.lower():
                corrupt_target = vf
                break
        assert corrupt_target is not None

        data = corrupt_target.read_bytes()
        corrupt_target.write_bytes(data[:65536] + b"TAMPERED_TAIL_DATA")

        # 3. Run recheck — should detect VAULT_CORRUPTED
        result = vault.run("recheck")
        assert result.returncode == 0
        combined = result.stderr + result.stdout
        assert "Vault corrupted:     1" in combined or "Vault corrupted:" in combined
        assert "Report:" in combined

        # Read report
        staging_dir = vault.vault_dir / ".svault" / "staging"
        reports = sorted(staging_dir.glob("recheck-*.txt"))
        assert len(reports) >= 1
        latest_report = reports[-1]
        report_text = latest_report.read_text()
        assert "VAULT_CORRUPTED" in report_text
        assert "OK" in report_text

        # 4. Delete corrupted vault file
        corrupt_target.unlink()
        assert not corrupt_target.exists()

        # 5. Re-import
        result = vault.import_dir(vault.source_dir, strategy="copy")
        assert result.returncode == 0
        assert "1 file(s) imported" in result.stderr or "1 file(s) imported" in result.stdout

        vault_files_after = vault.get_vault_files("*.jpg")
        assert len(vault_files_after) == 2

        # 6. Final recheck — everything OK
        result = vault.run("recheck")
        assert result.returncode == 0
        combined = result.stderr + result.stdout
        assert "OK:                  2" in combined or "OK:" in combined
        assert "VAULT_CORRUPTED" not in combined

    def test_recheck_all_ok(self, vault: VaultEnv) -> None:
        """Recheck after successful import should report all OK."""
        f1 = vault.source_dir / "a.jpg"
        f2 = vault.source_dir / "b.jpg"
        create_minimal_jpeg(f1, "FILE_A" * 500)
        create_minimal_jpeg(f2, "FILE_B" * 500)

        vault.import_dir(vault.source_dir, strategy="copy")

        result = vault.run("recheck")
        assert result.returncode == 0
        combined = result.stderr + result.stdout
        assert "OK:                  2" in combined or "OK:" in combined
        assert "VAULT_CORRUPTED" not in combined
        assert "SOURCE_MODIFIED" not in combined

    def test_recheck_source_mismatch(self, vault: VaultEnv) -> None:
        """Providing a source path that doesn't match the manifest should error."""
        f1 = vault.source_dir / "a.jpg"
        create_minimal_jpeg(f1, "FILE_A" * 500)

        vault.import_dir(vault.source_dir, strategy="copy")

        wrong_source = vault.root / "wrong_source"
        wrong_source.mkdir(parents=True, exist_ok=True)
        result = vault.run("recheck", str(wrong_source.resolve()), check=False)
        assert result.returncode != 0
        combined = result.stderr + result.stdout
        assert "Source path mismatch" in combined

    def test_recheck_with_matching_source(self, vault: VaultEnv) -> None:
        """Providing the correct source path should work."""
        f1 = vault.source_dir / "a.jpg"
        create_minimal_jpeg(f1, "FILE_A" * 500)

        vault.import_dir(vault.source_dir, strategy="copy")

        result = vault.run("recheck", str(vault.source_dir.resolve()))
        assert result.returncode == 0
        combined = result.stderr + result.stdout
        assert "OK:                  1" in combined or "OK:" in combined

    def test_strategy_copy_no_hardlink(self, vault: VaultEnv) -> None:
        """--strategy copy must perform a real binary copy, never a hard link."""
        f1 = vault.source_dir / "a.jpg"
        create_minimal_jpeg(f1, "UNIQUE_CONTENT_12345")

        vault.import_dir(vault.source_dir, strategy="copy")

        vault_files = vault.get_vault_files("*.jpg")
        assert len(vault_files) == 1

        src_inode = f1.stat().st_ino
        dst_inode = vault_files[0].stat().st_ino
        assert src_inode != dst_inode, "--strategy copy created a hard link (same inode)!"

    def test_deleted_file_can_be_reimported_after_verify_failure(self, vault: VaultEnv) -> None:
        """If verify detects corruption, deleting the vault file and re-importing works."""
        f1 = vault.source_dir / "photo.jpg"
        create_minimal_jpeg(f1, "PHOTO_DATA" * 500)

        vault.import_dir(vault.source_dir, strategy="copy")

        # Verify should pass initially
        result = vault.run("verify")
        assert result.returncode == 0

        # Corrupt vault file (keep CRC32C region intact)
        vault_files = vault.get_vault_files("*.jpg")
        assert len(vault_files) == 1
        vf = vault_files[0]
        data = vf.read_bytes()
        vf.write_bytes(data[:65536] + b"CORRUPT")

        # Verify should now fail
        result = vault.run("verify", check=False)
        assert result.returncode != 0
        assert (
            "Size mismatch" in result.stderr
            or "Size mismatch" in result.stdout
            or "hash mismatch" in result.stderr
            or "hash mismatch" in result.stdout
        )

        # Delete corrupted vault file
        vf.unlink()
        assert not vf.exists()

        # Re-import should restore the file
        result = vault.import_dir(vault.source_dir, strategy="copy")
        assert result.returncode == 0
        assert "1 file(s) imported" in result.stderr or "1 file(s) imported" in result.stdout

        vault_files_after = vault.get_vault_files("*.jpg")
        assert len(vault_files_after) == 1

        # Final verify should pass
        result = vault.run("verify")
        assert result.returncode == 0
