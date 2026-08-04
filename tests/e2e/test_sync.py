"""Tests for `svault sync` — copying files from a peer vault.

中文说明：
sync 以 Beyond Compare 风格比对两个 vault 的数据库记录（hash 加速），
把本 vault 缺失的文件复制过来。源 vault 以只读方式打开，永不被修改；
只存在于本 vault 的文件只会被报告，永不被删除。
"""

from __future__ import annotations

import json
import shutil
from pathlib import Path

import pytest
from conftest import VaultEnv, copy_fixture


@pytest.fixture
def peer_vault(vault: VaultEnv) -> VaultEnv:
    """A second vault living next to the primary one (same binary/ramdisk)."""
    peer_dir = vault.root / f"peer_{vault.vault_dir.name}"
    peer_source = vault.root / f"peer_source_{vault.vault_dir.name}"
    peer_dir.mkdir(parents=True, exist_ok=True)
    peer_source.mkdir(parents=True, exist_ok=True)

    peer = VaultEnv(
        root=vault.root,
        binary=vault.binary,
        vault_dir=peer_dir,
        source_dir=peer_source,
        output_dir=vault.output_dir,
    )
    peer.init()
    return peer


def _import_into(env: VaultEnv, fixture: str) -> Path:
    """Copy a fixture into env's source dir and import it."""
    src = env.source_dir / fixture
    shutil.copy(copy_fixture_source(fixture), src)
    env.import_dir(env.source_dir)
    return src


def copy_fixture_source(fixture_name: str) -> Path:
    """Absolute path of a fixture in tests/e2e/fixtures/source/."""
    from conftest import FIXTURES_DIR

    return FIXTURES_DIR / "source" / fixture_name


class TestSyncCommand:
    """End-to-end tests for `svault sync`."""

    def test_sync_copies_missing_files(self, vault: VaultEnv, peer_vault: VaultEnv) -> None:
        """Files only in the source vault are copied and registered."""
        _import_into(vault, "no_exif.jpg")          # source (peer direction: vault=source)
        _import_into(peer_vault, "apple_with_exif.jpg")

        # Sync peer (which only has apple) FROM vault (which only has no_exif)
        result = peer_vault.run("--output=json", "sync", str(vault.vault_dir), "--yes")
        assert result.returncode == 0

        # Assert via the JSON event stream (not UI wording)
        events = [json.loads(line) for line in result.stdout.strip().split("\n") if line]
        plan = next(e for e in events if e.get("event") == "sync_plan")
        assert plan["to_copy"] == 1
        assert plan["identical"] == 0

        # no_exif.jpg now exists in peer vault and is registered
        rows = peer_vault.find_file_in_db("no_exif.jpg")
        assert len(rows) == 1
        assert rows[0]["status"] == "imported"
        imported = list(peer_vault.vault_dir.rglob("no_exif.jpg"))
        assert len(imported) == 1
        assert imported[0].read_bytes() == copy_fixture_source("no_exif.jpg").read_bytes()

    def test_sync_is_idempotent(self, vault: VaultEnv, peer_vault: VaultEnv) -> None:
        """Second sync finds everything identical and copies nothing."""
        _import_into(vault, "no_exif.jpg")

        peer_vault.run("sync", str(vault.vault_dir), "--yes")
        result = peer_vault.run("--output=json", "sync", str(vault.vault_dir), "--yes")
        events = [json.loads(line) for line in result.stdout.strip().split("\n") if line]
        plan = next(e for e in events if e.get("event") == "sync_plan")
        assert plan["identical"] == 1
        assert plan["to_copy"] == 0
        rows = peer_vault.find_file_in_db("no_exif.jpg")
        assert len(rows) == 1  # still exactly one record

    def test_sync_keeps_dest_only_files(self, vault: VaultEnv, peer_vault: VaultEnv) -> None:
        """Files only in the local vault are reported and kept, never deleted."""
        _import_into(vault, "no_exif.jpg")
        _import_into(peer_vault, "apple_with_exif.jpg")

        result = peer_vault.run("--output=json", "sync", str(vault.vault_dir), "--yes")
        events = [json.loads(line) for line in result.stdout.strip().split("\n") if line]
        plan = next(e for e in events if e.get("event") == "sync_plan")
        assert plan["only_dest"] == 1

        # apple_with_exif.jpg still exists in peer vault
        assert len(list(peer_vault.vault_dir.rglob("apple_with_exif.jpg"))) == 1
        assert len(peer_vault.find_file_in_db("apple_with_exif.jpg")) == 1

    def test_sync_conflict_keeps_local(self, vault: VaultEnv, peer_vault: VaultEnv) -> None:
        """Same vault-relative path but different content → local file kept."""
        # Import files with the SAME name into both vaults but different content.
        # The import path template places them at the same vault-relative path.
        shared_name = "shared_photo.jpg"

        src1 = vault.source_dir / shared_name
        src1.write_bytes(b"content-AAA")
        vault.import_dir(vault.source_dir)

        src2 = peer_vault.source_dir / shared_name
        src2.write_bytes(b"content-BBB-different")
        peer_vault.import_dir(peer_vault.source_dir)

        result = peer_vault.run("--output=json", "sync", str(vault.vault_dir), "--yes")
        events = [json.loads(line) for line in result.stdout.strip().split("\n") if line]
        plan = next(e for e in events if e.get("event") == "sync_plan")
        assert plan["conflicts"] == 1

        # Local content preserved
        local_file = list(peer_vault.vault_dir.rglob(shared_name))
        assert len(local_file) == 1
        assert local_file[0].read_bytes() == b"content-BBB-different"

    def test_sync_detects_moved_files(self, vault: VaultEnv, peer_vault: VaultEnv) -> None:
        """Same hash at a different path is reported as moved, not copied."""
        _import_into(vault, "no_exif.jpg")

        # First sync the file in, then move it inside the peer vault + update
        peer_vault.run("sync", str(vault.vault_dir), "--yes")
        moved_dir = peer_vault.vault_dir / "archive"
        moved_dir.mkdir(exist_ok=True)
        original = list(peer_vault.vault_dir.rglob("no_exif.jpg"))[0]
        shutil.move(str(original), moved_dir / "no_exif.jpg")
        peer_vault.run("update", "--yes")

        result = peer_vault.run("--output=json", "sync", str(vault.vault_dir), "--yes")
        events = [json.loads(line) for line in result.stdout.strip().split("\n") if line]
        plan = next(e for e in events if e.get("event") == "sync_plan")
        assert plan["moved"] == 1
        assert plan["to_copy"] == 0
        # Nothing copied again
        assert len(peer_vault.find_file_in_db("no_exif.jpg")) == 1

    def test_sync_refuses_same_vault(self, vault: VaultEnv) -> None:
        """Syncing a vault with itself is an error."""
        result = vault.run("sync", str(vault.vault_dir), "--yes", check=False)
        assert result.returncode != 0
        assert "same vault" in (result.stderr + result.stdout)

    def test_sync_refuses_non_vault_source(self, vault: VaultEnv) -> None:
        """A plain directory without .svault/vault.db is rejected."""
        plain = vault.root / "not_a_vault"
        plain.mkdir(exist_ok=True)
        result = vault.run("sync", str(plain), "--yes", check=False)
        assert result.returncode != 0
        assert "not a svault vault" in (result.stderr + result.stdout)

    def test_sync_does_not_modify_source(self, vault: VaultEnv, peer_vault: VaultEnv) -> None:
        """The source vault's DB and files must remain untouched."""
        _import_into(vault, "no_exif.jpg")
        events_before = vault.db_query("SELECT COUNT(*) AS c FROM events")

        peer_vault.run("sync", str(vault.vault_dir), "--yes")

        events_after = vault.db_query("SELECT COUNT(*) AS c FROM events")
        assert events_before == events_after

    def test_sync_json_output(self, vault: VaultEnv, peer_vault: VaultEnv) -> None:
        """JSON mode emits a parseable event stream with a sync summary."""
        _import_into(vault, "no_exif.jpg")

        result = peer_vault.run(
            "--output=json", "sync", str(vault.vault_dir), "--yes"
        )
        events = [json.loads(line) for line in result.stdout.strip().split("\n") if line]
        summaries = [e for e in events if e.get("event") == "summary" and e.get("kind") == "sync"]
        assert len(summaries) == 1
        assert summaries[0]["copied"] == 1
