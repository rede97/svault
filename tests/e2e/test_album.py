"""Album command tests: hierarchical albums + per-membership ratings.

Contracts under test (docs/database-schema.md):
- nested create auto-creates parents; sibling names unique per level
- membership references files.id; add/remove never touch the files
- rating lives on the membership: same photo, different albums, different ratings
- delete refuses non-empty albums
"""

from __future__ import annotations

import json

from conftest import VaultEnv, create_minimal_jpeg


def _import_two(vault: VaultEnv) -> tuple[str, str]:
    create_minimal_jpeg(vault.source_dir / "a.jpg", "ALBUM_A")
    create_minimal_jpeg(vault.source_dir / "b.jpg", "ALBUM_B")
    result = vault.import_dir(vault.source_dir)
    assert result.returncode == 0
    paths = [f["path"] for f in vault.db_files()]
    a = next(p for p in paths if p.endswith("a.jpg"))
    b = next(p for p in paths if p.endswith("b.jpg"))
    return a, b


class TestAlbumHierarchy:
    def test_nested_create_auto_creates_parents(self, vault: VaultEnv) -> None:
        vault.run("album", "create", "trips/norway/tromso")

        rows = vault.db_query("SELECT id, name, parent_id FROM albums ORDER BY id")
        assert [r["name"] for r in rows] == ["trips", "norway", "tromso"]
        assert rows[0]["parent_id"] is None
        assert rows[1]["parent_id"] == rows[0]["id"]
        assert rows[2]["parent_id"] == rows[1]["id"]

        # Idempotent: creating again adds no rows.
        vault.run("album", "create", "trips/norway")
        assert len(vault.db_query("SELECT id FROM albums")) == 3

    def test_same_name_allowed_under_different_parents(self, vault: VaultEnv) -> None:
        vault.run("album", "create", "2024/favs")
        vault.run("album", "create", "2025/favs")
        names = vault.db_query("SELECT COUNT(*) AS c FROM albums WHERE name = 'favs'")
        assert names[0]["c"] == 2

    def test_list_shows_tree_with_counts(self, vault: VaultEnv) -> None:
        a, _ = _import_two(vault)
        vault.run("album", "create", "trips/norway")
        vault.run("album", "add", "trips/norway", a)

        result = vault.run("--output=json", "album", "list")
        tree = json.loads(result.stdout)
        assert tree[0]["name"] == "trips"
        assert tree[0]["member_count"] == 0
        assert tree[0]["children"][0]["name"] == "norway"
        assert tree[0]["children"][0]["member_count"] == 1


class TestAlbumMembership:
    def test_add_show_remove_cycle(self, vault: VaultEnv) -> None:
        a, b = _import_two(vault)
        vault.run("album", "create", "favs")

        vault.run("album", "add", "favs", a, b, "ghost.jpg")
        detail = json.loads(vault.run("--output=json", "album", "show", "favs").stdout)
        assert {m["path"] for m in detail["members"]} == {a, b}

        # Re-add is a skip, not a duplicate row.
        vault.run("album", "add", "favs", a)
        count = vault.db_query("SELECT COUNT(*) AS c FROM album_items")
        assert count[0]["c"] == 2

        vault.run("album", "remove", "favs", a)
        detail = json.loads(vault.run("--output=json", "album", "show", "favs").stdout)
        assert [m["path"] for m in detail["members"]] == [b]
        # The file itself is untouched (G1).
        assert (vault.vault_dir / a).exists()

    def test_add_rejects_unknown_album(self, vault: VaultEnv) -> None:
        a, _ = _import_two(vault)
        result = vault.run("album", "add", "nope", a, check=False)
        assert result.returncode != 0
        assert "album not found" in (result.stderr + result.stdout)


class TestAlbumRating:
    def test_rating_is_independent_per_album(self, vault: VaultEnv) -> None:
        a, _ = _import_two(vault)
        vault.run("album", "create", "keep")
        vault.run("album", "create", "review")
        vault.run("album", "add", "keep", a)
        vault.run("album", "add", "review", a)

        vault.run("album", "rate", "keep", "5", a)
        vault.run("album", "rate", "review", "2", a)

        keep = json.loads(vault.run("--output=json", "album", "show", "keep").stdout)
        review = json.loads(vault.run("--output=json", "album", "show", "review").stdout)
        assert keep["members"][0]["rating"] == 5
        assert review["members"][0]["rating"] == 2

        # files 表不持有评级（评级是成员关系属性，不是文件属性）
        cols = [r["name"] for r in vault.db_query("SELECT name FROM pragma_table_info('files')")]
        assert "rating" not in cols

    def test_rate_clear_and_non_member(self, vault: VaultEnv) -> None:
        a, b = _import_two(vault)
        vault.run("album", "create", "favs")
        vault.run("album", "add", "favs", a)

        vault.run("album", "rate", "favs", "4", a)
        vault.run("album", "rate", "favs", "0", a)  # 清除
        detail = json.loads(vault.run("--output=json", "album", "show", "favs").stdout)
        assert detail["members"][0]["rating"] is None

        # 非成员评级：跳过，不隐式创建成员关系
        vault.run("album", "rate", "favs", "3", b)
        assert len(vault.db_query("SELECT * FROM album_items")) == 1

        # 越界评级报错
        result = vault.run("album", "rate", "favs", "6", a, check=False)
        assert result.returncode != 0


class TestAlbumDelete:
    def test_delete_requires_empty(self, vault: VaultEnv) -> None:
        a, _ = _import_two(vault)
        vault.run("album", "create", "parent/child")
        vault.run("album", "add", "parent/child", a)

        r1 = vault.run("album", "delete", "parent", check=False)
        assert r1.returncode != 0  # has child
        r2 = vault.run("album", "delete", "parent/child", check=False)
        assert r2.returncode != 0  # has members

        vault.run("album", "remove", "parent/child", a)
        vault.run("album", "delete", "parent/child")
        vault.run("album", "delete", "parent")
        assert vault.db_query("SELECT id FROM albums") == []
