"""Import scan filter tests: --max-depth / --include / --exclude.

Contracts (cli.md): globs match the source-relative path case-insensitively;
exclusions win over inclusions; -r 1 scans only the source directory itself,
-r 0 (default) recurses without limit. All filters combine (AND) with the
extension allowlist.
"""

from __future__ import annotations

from conftest import VaultEnv, create_minimal_jpeg


def _make_tree(vault: VaultEnv) -> None:
    """top.jpg / top2.jpg at root, sub/mid.jpg, sub/deep/bottom.jpg below."""
    create_minimal_jpeg(vault.source_dir / "top.jpg", "TOP")
    create_minimal_jpeg(vault.source_dir / "top2.jpg", "TOP2")
    sub = vault.source_dir / "sub"
    (sub / "deep").mkdir(parents=True)
    create_minimal_jpeg(sub / "mid.jpg", "MID")
    create_minimal_jpeg(sub / "deep" / "bottom.jpg", "BOTTOM")


class TestImportScanFilter:
    def test_max_depth_one_scans_only_top_level(self, vault: VaultEnv) -> None:
        _make_tree(vault)
        result = vault.run(
            "--yes", "import", str(vault.source_dir), "-r", "1"
        )
        assert result.returncode == 0
        paths = [f["path"] for f in vault.db_files()]
        assert any(p.endswith("top.jpg") for p in paths)
        assert any(p.endswith("top2.jpg") for p in paths)
        assert len(paths) == 2
        assert not any("mid.jpg" in p or "bottom.jpg" in p for p in paths)

    def test_default_depth_recurses_fully(self, vault: VaultEnv) -> None:
        _make_tree(vault)
        result = vault.run("--yes", "import", str(vault.source_dir))
        assert result.returncode == 0
        assert len(vault.db_files()) == 4

    def test_include_glob_case_insensitive(self, vault: VaultEnv) -> None:
        _make_tree(vault)
        result = vault.run(
            "--yes", "import", str(vault.source_dir), "--include", "*.JPG"
        )
        assert result.returncode == 0
        paths = [f["path"] for f in vault.db_files()]
        assert len(paths) == 4, f"应导入全部 4 个 jpg: {paths}"
        assert all(p.lower().endswith(".jpg") for p in paths)

    def test_exclude_wins_over_include(self, vault: VaultEnv) -> None:
        _make_tree(vault)
        result = vault.run(
            "--yes", "import", str(vault.source_dir),
            "--include", "*.jpg", "--exclude", "sub/**",
        )
        assert result.returncode == 0
        paths = sorted(f["path"] for f in vault.db_files())
        assert len(paths) == 2
        assert paths[0].endswith("top.jpg") and paths[1].endswith("top2.jpg")

    def test_invalid_glob_fails_loudly(self, vault: VaultEnv) -> None:
        _make_tree(vault)
        result = vault.run(
            "--yes", "import", str(vault.source_dir), "--include", "[unclosed",
            check=False,
        )
        assert result.returncode != 0
        assert vault.db_files() == []
