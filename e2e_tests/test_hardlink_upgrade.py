"""Hardlink upgrade tests.

Tests the `verify --upgrade-links` flag that detects hardlinked vault
files and atomically upgrades them to independent binary copies.

中文场景说明：
- hardlink 导入可节省空间，但用户可能希望 vault 文件独立
- verify --upgrade-links 在验证前将 hardlink 升级为二进制副本
- 升级过程是原子的（先写临时文件再 rename），不会破坏数据
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest

from conftest import VaultEnv, copy_fixture, IS_WINDOWS


def _nlink(path: Path) -> int:
    """Cross-platform hard link count."""
    return os.stat(path).st_nlink


class TestHardlinkUpgrade:
    """Hardlink-to-binary-copy upgrade tests."""

    def test_upgrade_hardlink_during_verify(self, vault: VaultEnv) -> None:
        """verify --upgrade-links should break hardlinks in the vault.

        Scenario: A vault file is a hardlink (nlink > 1).
        Expected: After --upgrade-links, nlink becomes 1 and content is preserved.
        """
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        # Find the imported file in the vault
        rows = vault.db_query(
            "SELECT id, path FROM files WHERE status = 'imported'"
        )
        assert len(rows) == 1
        file_id = rows[0]["id"]
        original_rel = rows[0]["path"]
        original_path = vault.vault_dir / original_rel
        assert original_path.exists()

        # Create a hardlink inside the vault directory
        link_rel = str(Path(original_rel).with_suffix(".link.jpg"))
        link_path = vault.vault_dir / link_rel
        link_path.parent.mkdir(parents=True, exist_ok=True)
        os.link(original_path, link_path)
        assert _nlink(link_path) > 1

        # Point the DB record to the hardlink path
        # Escape single quotes to avoid SQL injection in test data
        safe_link_rel = link_rel.replace("'", "''")
        vault.db_query(f"UPDATE files SET path = '{safe_link_rel}' WHERE id = {file_id}")

        # Run verify --upgrade-links
        result = vault.run("verify", "--upgrade-links", capture=True)
        assert result.returncode == 0
        combined = result.stdout + result.stderr
        assert "Upgraded hardlink" in combined

        # The hardlink should now be an independent file
        assert _nlink(link_path) == 1

        # Content must be preserved
        assert link_path.read_bytes() == original_path.read_bytes()

        # DB record should still point to the upgraded file
        files_after = vault.db_query(
            "SELECT path FROM files WHERE id = {}".format(file_id)
        )
        assert len(files_after) == 1
        assert files_after[0]["path"] == link_rel

    def test_upgrade_links_no_op_for_regular_files(self, vault: VaultEnv) -> None:
        """verify --upgrade-links should do nothing for regular non-hardlinked files."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        result = vault.run("verify", "--upgrade-links", capture=True)
        assert result.returncode == 0
        combined = result.stdout + result.stderr
        # Regular files should not trigger upgrade messages
        assert "Upgraded hardlink" not in combined
