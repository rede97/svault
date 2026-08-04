"""Tests for `svault clone` — exporting a subset of the vault to a plain directory.

中文说明：
clone 命令将 vault 中的文件（可选按日期过滤）单向导出到普通目录。
目标目录不是 vault；导出内容附带 svault-clone-manifest.json。
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from conftest import VaultEnv, copy_fixture


class TestCloneCommand:
    """End-to-end tests for `svault clone`."""

    def test_clone_exports_all_files(self, vault: VaultEnv) -> None:
        """Clone with no filters copies every imported file + manifest."""
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "no_exif.jpg")
        vault.import_dir(vault.source_dir)

        target = vault.output_dir / "export"
        result = vault.run("clone", "--target", str(target))
        assert result.returncode == 0

        # Files exported preserving vault-relative paths
        exported = [p for p in target.rglob("*") if p.is_file()]
        exported_names = {p.name for p in exported}
        assert "apple_with_exif.jpg" in exported_names
        assert "no_exif.jpg" in exported_names

        # Manifest written
        manifest_path = target / "svault-clone-manifest.json"
        assert manifest_path.exists()
        manifest = json.loads(manifest_path.read_text())
        assert manifest["session_type"] == "clone"
        assert manifest["summary"]["copied"] == 2
        assert len(manifest["files"]) == 2

    def test_clone_preserves_content(self, vault: VaultEnv) -> None:
        """Exported files must be byte-identical to vault copies."""
        src = copy_fixture(vault, "no_exif.jpg")
        vault.import_dir(vault.source_dir)

        target = vault.output_dir / "export_content"
        vault.run("clone", "--target", str(target))

        exported = list(target.rglob("no_exif.jpg"))
        assert len(exported) == 1
        assert exported[0].read_bytes() == src.read_bytes()

    def test_clone_date_filter_excludes_everything(self, vault: VaultEnv) -> None:
        """A date range far in the past matches nothing."""
        copy_fixture(vault, "no_exif.jpg")
        vault.import_dir(vault.source_dir)

        target = vault.output_dir / "export_old"
        result = vault.run(
            "clone", "--target", str(target), "--filter-date", "2020-01-01..2020-12-31"
        )
        assert result.returncode == 0
        exported = [p for p in target.rglob("*") if p.is_file()]
        assert len(exported) == 0

    def test_clone_date_filter_includes_today(self, vault: VaultEnv) -> None:
        """A range covering the file's mtime matches it."""
        import datetime
        import os
        import time

        src = copy_fixture(vault, "no_exif.jpg")
        # copy_fixture preserves the fixture's old mtime (copy2); set it to now
        now = time.time()
        os.utime(src, (now, now))
        vault.import_dir(vault.source_dir)

        # Use a wide range around now — the filter is UTC-date-based while
        # mtime is local, so "today" alone is flaky near midnight.
        today = datetime.date.today()
        yesterday = today - datetime.timedelta(days=1)
        tomorrow = today + datetime.timedelta(days=1)
        rng = f"{yesterday.isoformat()}..{tomorrow.isoformat()}"
        target = vault.output_dir / "export_today"
        vault.run("clone", "--target", str(target), "--filter-date", rng)
        exported = [p for p in target.rglob("*.jpg") if p.is_file()]
        assert len(exported) == 1

    def test_clone_rejects_target_inside_vault(self, vault: VaultEnv) -> None:
        """Target inside the vault must be rejected."""
        copy_fixture(vault, "no_exif.jpg")
        vault.import_dir(vault.source_dir)

        result = vault.run(
            "clone", "--target", str(vault.vault_dir / "nested_export"), check=False
        )
        assert result.returncode != 0
        assert "inside the vault" in (result.stderr + result.stdout)

    def test_clone_invalid_date_range(self, vault: VaultEnv) -> None:
        """Garbage date range produces a clear error."""
        result = vault.run(
            "clone", "--target", str(vault.output_dir / "x"), "--filter-date", "march",
            check=False,
        )
        assert result.returncode != 0

    def test_clone_records_audit_event(self, vault: VaultEnv) -> None:
        """Clone appends a vault.cloned event to the source vault DB."""
        copy_fixture(vault, "no_exif.jpg")
        vault.import_dir(vault.source_dir)

        vault.run("clone", "--target", str(vault.output_dir / "export_audit"))
        rows = vault.db_query("SELECT event_type FROM events WHERE event_type = 'vault.cloned'")
        assert len(rows) == 1

    def test_clone_empty_vault(self, vault: VaultEnv) -> None:
        """Cloning an empty vault succeeds with zero files."""
        target = vault.output_dir / "export_empty"
        result = vault.run("clone", "--target", str(target))
        assert result.returncode == 0
