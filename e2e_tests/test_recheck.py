"""Recheck command tests.

Tests for the standalone `svault recheck` command, which compares
source files against vault entries using full-file hashes.

中文场景说明：
- 用户怀疑 vault 中的文件已损坏（例如硬盘故障、复制中断）
- 用户运行 recheck，svault 出具报告对比源文件和 vault 文件
- 用户查看报告后，手动删除错误的一方，然后重新导入
- Svault 遵循不删除用户文件的原则

关键 workflow：
1. 导入文件
2. recheck 应能通过（OK）
3. 篡改 vault 文件（保持 CRC32C 区域不变）
4. recheck 应能检测出 MISMATCH
5. 删除损坏的 vault 文件
6. 再次导入应能成功（因为文件在 DB 中但不在文件系统中，会被视为新文件）
7. recheck 再次通过
"""

from __future__ import annotations

from pathlib import Path

import pytest

from conftest import VaultEnv, create_minimal_jpeg


class TestRecheckWorkflow:
    """End-to-end recheck and re-import workflow."""

    def test_recheck_detects_corruption_and_reimport_succeeds(self, vault: VaultEnv) -> None:
        """Detect vault file corruption and recover by re-importing.
        
        Scenario:
        1. Import two files from source
        2. Corrupt one vault file (modify tail so CRC32C still matches)
        3. Run `svault recheck` — should report MISMATCH
        4. Delete the corrupted vault file
        5. Run `svault import` again — should re-import the missing file
        6. Run `svault recheck` again — should report all OK
        """
        # Create source files with enough content
        f1 = vault.source_dir / "keep.jpg"
        f2 = vault.source_dir / "corrupt.jpg"
        create_minimal_jpeg(f1, "KEEP_KEEP_KEEP_" * 1000)
        create_minimal_jpeg(f2, "CORRUPT_CORRUPT_" * 1000)

        # 1. First import (use copy so vault files are independent of source)
        result = vault.import_dir(vault.source_dir, strategy="copy")
        assert result.returncode == 0
        files = vault.db_files()
        assert len(files) == 2

        # 2. Corrupt one vault file (modify after first 64KB to keep CRC32C match)
        vault_files = vault.get_vault_files("*.jpg")
        assert len(vault_files) == 2

        corrupt_target = None
        for vf in vault_files:
            if "corrupt" in vf.name.lower():
                corrupt_target = vf
                break
        assert corrupt_target is not None, "corrupt.jpg not found in vault"

        data = corrupt_target.read_bytes()
        # Keep first 65536 bytes intact, change the rest
        preserved = data[:65536]
        modified = preserved + b"TAMPERED_TAIL_DATA"
        corrupt_target.write_bytes(modified)

        # 3. Run recheck
        result = vault.run("recheck", str(vault.source_dir.resolve()))
        assert result.returncode == 0
        # Summary printed to stderr says "differ from vault"; the detailed report says "MISMATCH"
        assert "differ from vault" in result.stderr or "differ from vault" in result.stdout
        assert "Report:" in result.stderr or "Report:" in result.stdout

        # Find and read the recheck report
        staging_dir = vault.vault_dir / ".svault" / "staging"
        reports = sorted(staging_dir.glob("recheck-*.txt"))
        assert len(reports) >= 1
        latest_report = reports[-1]
        report_text = latest_report.read_text()
        assert "MISMATCH" in report_text
        assert "OK" in report_text

        # 4. Delete corrupted vault file
        corrupt_target.unlink()
        assert not corrupt_target.exists()

        # 5. Re-import — the deleted file should be treated as new
        result = vault.import_dir(vault.source_dir, strategy="copy")
        assert result.returncode == 0
        # Should re-import 1 file
        assert "1 file(s) imported" in result.stderr or "1 file(s) imported" in result.stdout

        # Verify both files exist in vault again
        vault_files_after = vault.get_vault_files("*.jpg")
        assert len(vault_files_after) == 2

        # 6. Final recheck — everything should be OK
        result = vault.run("recheck", str(vault.source_dir.resolve()))
        assert result.returncode == 0
        assert "files match vault" in result.stderr or "files match vault" in result.stdout
        # Should have no MISMATCH
        assert "MISMATCH" not in result.stderr and "MISMATCH" not in result.stdout

    def test_recheck_no_cache_hits(self, vault: VaultEnv) -> None:
        """Recheck on a source that has never been imported."""
        f1 = vault.source_dir / "new.jpg"
        create_minimal_jpeg(f1, "NEW_FILE")

        result = vault.run("recheck", str(vault.source_dir.resolve()))
        assert result.returncode == 0
        assert "No cache hits found" in result.stderr or "No cache hits found" in result.stdout

    def test_recheck_all_ok(self, vault: VaultEnv) -> None:
        """Recheck after successful import should report all OK."""
        f1 = vault.source_dir / "a.jpg"
        f2 = vault.source_dir / "b.jpg"
        create_minimal_jpeg(f1, "FILE_A" * 500)
        create_minimal_jpeg(f2, "FILE_B" * 500)

        vault.import_dir(vault.source_dir, strategy="copy")

        result = vault.run("recheck", str(vault.source_dir.resolve()))
        assert result.returncode == 0
        assert "files match vault" in result.stderr or "files match vault" in result.stdout
        assert "MISMATCH" not in result.stderr and "MISMATCH" not in result.stdout

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
