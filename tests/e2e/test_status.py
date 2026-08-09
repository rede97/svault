"""Status working-tree tests (git-status style).

Contracts (cli.md / database-schema.md):
- status 默认输出全部类别：untracked / moved / missing / modified
- --untracked/--moved/--missing/--modified 只输出指定类别
- svault.toml 与 .svault/ 永不出现在 untracked 中
"""

from __future__ import annotations

import json

from conftest import VaultEnv, create_minimal_jpeg


def _setup_vault(vault: VaultEnv) -> tuple[str, str]:
    """Import two files; return their vault-relative DB paths."""
    create_minimal_jpeg(vault.source_dir / "a.jpg", "STATUS_A")
    create_minimal_jpeg(vault.source_dir / "b.jpg", "STATUS_B")
    assert vault.import_dir(vault.source_dir).returncode == 0
    paths = [f["path"] for f in vault.db_files()]
    a = next(p for p in paths if p.endswith("a.jpg"))
    b = next(p for p in paths if p.endswith("b.jpg"))
    return a, b


class TestWorkingTreeStatus:
    def test_clean_vault_reports_no_changes(self, vault: VaultEnv) -> None:
        _setup_vault(vault)
        report = json.loads(vault.run("--output=json", "status").stdout)
        wt = report["working_tree"]
        assert wt["untracked"] == []
        assert wt["moved"] == []
        assert wt["missing"] == []
        assert wt["modified"] == []

    def test_all_categories_detected(self, vault: VaultEnv) -> None:
        a, b = _setup_vault(vault)

        # moved：DB 记录的文件被移到新路径
        moved_new = vault.vault_dir / "elsewhere"
        moved_new.mkdir()
        (vault.vault_dir / a).rename(moved_new / "a.jpg")
        # missing：文件从磁盘消失
        (vault.vault_dir / b).unlink()
        # untracked：未注册的新文件
        create_minimal_jpeg(vault.vault_dir / "stray.jpg", "STRAY")

        report = json.loads(vault.run("--output=json", "status").stdout)
        wt = report["working_tree"]
        assert wt["untracked"] == ["stray.jpg"]
        assert wt["moved"] == [[a, "elsewhere/a.jpg"]]
        assert wt["missing"] == [b]
        assert wt["modified"] == []

    def test_modified_detected_by_size_change(self, vault: VaultEnv) -> None:
        a, _ = _setup_vault(vault)
        with open(vault.vault_dir / a, "ab") as f:
            f.write(b"\x00" * 64)

        report = json.loads(vault.run("--output=json", "status").stdout)
        assert report["working_tree"]["modified"] == [a]

    def test_category_flags_filter_human_output(self, vault: VaultEnv) -> None:
        a, _ = _setup_vault(vault)
        (vault.vault_dir / a).unlink()
        create_minimal_jpeg(vault.vault_dir / "stray.jpg", "STRAY")

        moved_out = vault.run("status", "--missing")
        assert "Missing" in moved_out.stdout
        assert "Untracked" not in moved_out.stdout

        untracked_out = vault.run("status", "--untracked")
        assert "Untracked" in untracked_out.stdout
        assert "Missing" not in untracked_out.stdout

    def test_svault_internals_never_untracked(self, vault: VaultEnv) -> None:
        _setup_vault(vault)
        report = json.loads(vault.run("--output=json", "status").stdout)
        untracked = report["working_tree"]["untracked"]
        assert not any("svault.toml" in p or ".svault" in p for p in untracked)
