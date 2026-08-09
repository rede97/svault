//! `svault album` — hierarchical, named collections of vault files.
//!
//! Design (maintainer decisions, 2026-08-09):
//!
//! - Albums form a tree via `albums.parent_id`; paths look like
//!   `trips/norway/tromso`. `create` auto-creates missing parents.
//! - Membership references `files.id` (stable: `files` rows are never
//!   physically deleted and the DB is never rebuilt by id).
//! - Ratings live on the membership (`album_items.rating`): the same photo
//!   may be rated differently in different albums.
//! - Everything here is a fast CRUD operation → Pull model (serializable
//!   return values, no events).
//!
//! Deleting files is impossible by design; `delete` removes only an empty
//! album's metadata row.

use std::path::Path;

use serde::Serialize;

use crate::db::{Db, albums::AlbumRow};

/// Result of `album create`: the full path plus segments created this call.
#[derive(Debug, Clone, Serialize)]
pub struct AlbumCreated {
    pub path: String,
    /// Segments (full paths) that were newly created.
    pub created: Vec<String>,
    /// Segments (full paths) that already existed.
    pub existed: Vec<String>,
}

/// One node of the album tree (`album list`).
#[derive(Debug, Clone, Serialize)]
pub struct AlbumNode {
    pub name: String,
    pub path: String,
    pub member_count: i64,
    pub children: Vec<AlbumNode>,
}

/// One member of an album (`album show`).
#[derive(Debug, Clone, Serialize)]
pub struct AlbumMember {
    pub path: String,
    pub rating: Option<i64>,
    pub added_at: i64,
}

/// Album detail for `album show`.
#[derive(Debug, Clone, Serialize)]
pub struct AlbumDetail {
    pub path: String,
    pub members: Vec<AlbumMember>,
}

/// A path that could not be processed, with the reason.
#[derive(Debug, Clone, Serialize)]
pub struct SkippedPath {
    pub path: String,
    pub reason: String,
}

/// Outcome of a membership-changing operation (add/remove/rate).
#[derive(Debug, Clone, Serialize)]
pub struct AlbumChange {
    pub album: String,
    /// Paths the operation applied to.
    pub affected: Vec<String>,
    /// Paths skipped (already in that state / not a member / not in vault).
    pub skipped: Vec<SkippedPath>,
}

/// Split `trips/norway/tromso` into validated segments.
fn parse_album_path(path: &str) -> anyhow::Result<Vec<String>> {
    let segments: Vec<String> = path
        .split('/')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if segments.is_empty() {
        anyhow::bail!("empty album path");
    }
    for seg in &segments {
        if seg == "." || seg == ".." || seg.contains('\\') {
            anyhow::bail!("invalid album path segment: '{seg}'");
        }
    }
    Ok(segments)
}

/// Walk the tree from the root; returns the album id if the full path exists.
fn resolve_album(db: &Db, segments: &[String]) -> anyhow::Result<Option<i64>> {
    let mut parent: Option<i64> = None;
    for seg in segments {
        match db.album_get(parent, seg)? {
            Some(row) => parent = Some(row.id),
            None => return Ok(None),
        }
    }
    Ok(parent)
}

/// `album create <path>` — create the path, auto-creating missing parents.
pub fn create(db: &Db, path: &str) -> anyhow::Result<AlbumCreated> {
    let segments = parse_album_path(path)?;
    let mut parent: Option<i64> = None;
    let mut created = Vec::new();
    let mut existed = Vec::new();
    let mut prefix = String::new();
    let now = crate::ops::utils::unix_now_ms();

    for seg in &segments {
        prefix = if prefix.is_empty() {
            seg.clone()
        } else {
            format!("{prefix}/{seg}")
        };
        match db.album_get(parent, seg)? {
            Some(row) => {
                existed.push(prefix.clone());
                parent = Some(row.id);
            }
            None => {
                let id = db.album_create(parent, seg, now)?;
                created.push(prefix.clone());
                parent = Some(id);
            }
        }
    }

    Ok(AlbumCreated {
        path: segments.join("/"),
        created,
        existed,
    })
}

/// `album list` — the full tree with direct member counts, root albums first.
pub fn list(db: &Db) -> anyhow::Result<Vec<AlbumNode>> {
    let rows = db.albums_all()?;
    build_tree(db, &rows, None)
}

fn build_tree(db: &Db, rows: &[AlbumRow], parent: Option<i64>) -> anyhow::Result<Vec<AlbumNode>> {
    build_tree_with_prefix(db, rows, parent, "")
}

fn build_tree_with_prefix(
    db: &Db,
    rows: &[AlbumRow],
    parent: Option<i64>,
    prefix: &str,
) -> anyhow::Result<Vec<AlbumNode>> {
    let mut nodes: Vec<&AlbumRow> = rows.iter().filter(|r| r.parent_id == parent).collect();
    nodes.sort_by(|a, b| a.name.cmp(&b.name));

    let mut out = Vec::with_capacity(nodes.len());
    for row in nodes {
        let path = if prefix.is_empty() {
            row.name.clone()
        } else {
            format!("{prefix}/{}", row.name)
        };
        out.push(AlbumNode {
            name: row.name.clone(),
            member_count: db.album_item_count(row.id)?,
            children: build_tree_with_prefix(db, rows, Some(row.id), &path)?,
            path,
        });
    }
    Ok(out)
}

/// `album show <path>` — members with per-membership ratings.
pub fn show(db: &Db, path: &str) -> anyhow::Result<AlbumDetail> {
    let segments = parse_album_path(path)?;
    let id =
        resolve_album(db, &segments)?.ok_or_else(|| anyhow::anyhow!("album not found: {path}"))?;
    let members = db
        .album_items(id)?
        .into_iter()
        .map(|r| AlbumMember {
            path: r.path,
            rating: r.rating,
            added_at: r.added_at,
        })
        .collect();
    Ok(AlbumDetail {
        path: segments.join("/"),
        members,
    })
}

/// Normalize a user-supplied path to the vault-relative Unix-style form
/// stored in `files.path`. Accepts DB-style relative paths as-is and strips
/// the vault root from filesystem paths.
fn normalize_file_path(vault_root: &Path, input: &str) -> String {
    let p = Path::new(input);
    let rel = if p.is_absolute() {
        p.strip_prefix(vault_root).unwrap_or(p)
    } else {
        p
    };
    rel.to_string_lossy().replace('\\', "/")
}

/// Resolve input paths to `files.id`s; unknown paths become skips.
fn resolve_files(
    db: &Db,
    vault_root: &Path,
    paths: &[String],
    skipped: &mut Vec<SkippedPath>,
) -> anyhow::Result<Vec<(String, i64)>> {
    let mut resolved = Vec::with_capacity(paths.len());
    for input in paths {
        let rel = normalize_file_path(vault_root, input);
        match db.get_file_by_path(&rel)? {
            Some(row) => resolved.push((rel, row.id)),
            None => skipped.push(SkippedPath {
                path: input.clone(),
                reason: "not in vault database".to_string(),
            }),
        }
    }
    Ok(resolved)
}

/// `album add <album> <paths...>`.
pub fn add(
    db: &Db,
    vault_root: &Path,
    album: &str,
    paths: &[String],
) -> anyhow::Result<AlbumChange> {
    let segments = parse_album_path(album)?;
    let album_id = resolve_album(db, &segments)?
        .ok_or_else(|| anyhow::anyhow!("album not found: {album} (create it first)"))?;

    let mut skipped = Vec::new();
    let resolved = resolve_files(db, vault_root, paths, &mut skipped)?;
    let now = crate::ops::utils::unix_now_ms();

    let mut affected = Vec::new();
    for (rel, file_id) in resolved {
        if db.album_item_add(album_id, file_id, now)? {
            affected.push(rel);
        } else {
            skipped.push(SkippedPath {
                path: rel,
                reason: "already a member".to_string(),
            });
        }
    }
    Ok(AlbumChange {
        album: segments.join("/"),
        affected,
        skipped,
    })
}

/// `album remove <album> <paths...>` — membership only, files are untouched.
pub fn remove(
    db: &Db,
    vault_root: &Path,
    album: &str,
    paths: &[String],
) -> anyhow::Result<AlbumChange> {
    let segments = parse_album_path(album)?;
    let album_id =
        resolve_album(db, &segments)?.ok_or_else(|| anyhow::anyhow!("album not found: {album}"))?;

    let mut skipped = Vec::new();
    let resolved = resolve_files(db, vault_root, paths, &mut skipped)?;

    let mut affected = Vec::new();
    for (rel, file_id) in resolved {
        if db.album_item_remove(album_id, file_id)? {
            affected.push(rel);
        } else {
            skipped.push(SkippedPath {
                path: rel,
                reason: "not a member".to_string(),
            });
        }
    }
    Ok(AlbumChange {
        album: segments.join("/"),
        affected,
        skipped,
    })
}

/// `album rate <album> <rating> <paths...>` — per-membership rating.
/// `rating` is 1-5; `None` (CLI: 0) clears it. The target must already be
/// a member — rating a non-member would silently invent membership.
pub fn rate(
    db: &Db,
    vault_root: &Path,
    album: &str,
    rating: Option<u8>,
    paths: &[String],
) -> anyhow::Result<AlbumChange> {
    if let Some(r) = rating
        && !(1..=5).contains(&r)
    {
        anyhow::bail!("rating must be 1-5 (0 to clear)");
    }
    let segments = parse_album_path(album)?;
    let album_id =
        resolve_album(db, &segments)?.ok_or_else(|| anyhow::anyhow!("album not found: {album}"))?;

    let mut skipped = Vec::new();
    let resolved = resolve_files(db, vault_root, paths, &mut skipped)?;

    let mut affected = Vec::new();
    for (rel, file_id) in resolved {
        if db.album_item_set_rating(album_id, file_id, rating.map(i64::from))? {
            affected.push(rel);
        } else {
            skipped.push(SkippedPath {
                path: rel,
                reason: "not a member (add it first)".to_string(),
            });
        }
    }
    Ok(AlbumChange {
        album: segments.join("/"),
        affected,
        skipped,
    })
}

/// `album delete <path>` — only empty albums (no children, no members).
pub fn delete(db: &Db, path: &str) -> anyhow::Result<()> {
    let segments = parse_album_path(path)?;
    let id =
        resolve_album(db, &segments)?.ok_or_else(|| anyhow::anyhow!("album not found: {path}"))?;
    let children = db.album_child_count(id)?;
    if children > 0 {
        anyhow::bail!("album '{path}' has {children} child album(s); delete them first");
    }
    let members = db.album_item_count(id)?;
    if members > 0 {
        anyhow::bail!("album '{path}' has {members} member(s); remove them first");
    }
    db.album_delete(id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    /// Insert a minimal files row, returning its id.
    fn insert_file(db: &Db, path: &str) -> i64 {
        db.insert_file_row(path, 1, 0, None, None, None, None, "imported", 0)
            .unwrap()
    }

    fn setup() -> (Db, i64, i64) {
        let db = Db::open_in_memory().unwrap();
        let a = insert_file(&db, "2026/a.jpg");
        let b = insert_file(&db, "2026/b.jpg");
        (db, a, b)
    }

    #[test]
    fn create_nested_auto_creates_parents() {
        let (db, _, _) = setup();
        let result = create(&db, "trips/norway/tromso").unwrap();
        assert_eq!(result.created.len(), 3);
        assert!(result.existed.is_empty());

        // Re-create: everything already exists, nothing duplicated.
        let again = create(&db, "trips/norway").unwrap();
        assert_eq!(again.created.len(), 0);
        assert_eq!(again.existed.len(), 2);

        let tree = list(&db).unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "trips");
        assert_eq!(tree[0].children[0].children[0].path, "trips/norway/tromso");
    }

    #[test]
    fn sibling_names_unique_but_reusable_across_parents() {
        let (db, _, _) = setup();
        create(&db, "a/x").unwrap();
        create(&db, "b/x").unwrap(); // same leaf name under another parent: OK
        let dup = create(&db, "a").unwrap(); // existing root: idempotent
        assert_eq!(dup.existed, vec!["a".to_string()]);

        // Direct INSERT of a duplicate sibling must hit the unique index.
        let a = db.album_get(None, "a").unwrap().unwrap();
        let conflict = db.album_create(a.parent_id, "a", 0);
        assert!(conflict.is_err());
    }

    #[test]
    fn membership_add_show_remove() {
        let (db, a, b) = setup();
        let vault = Path::new("/vault");
        create(&db, "favs").unwrap();

        let change = add(
            &db,
            vault,
            "favs",
            &["2026/a.jpg".into(), "2026/b.jpg".into(), "ghost.jpg".into()],
        )
        .unwrap();
        assert_eq!(change.affected.len(), 2);
        assert_eq!(change.skipped.len(), 1);
        assert_eq!(change.skipped[0].reason, "not in vault database");

        // Re-adding is a skip, not an error and not a duplicate row.
        let again = add(&db, vault, "favs", &["2026/a.jpg".into()]).unwrap();
        assert!(again.affected.is_empty());
        assert_eq!(again.skipped[0].reason, "already a member");

        let detail = show(&db, "favs").unwrap();
        assert_eq!(detail.members.len(), 2);
        assert!(detail.members.iter().all(|m| m.rating.is_none()));

        let removed = remove(&db, vault, "favs", &["2026/a.jpg".into()]).unwrap();
        assert_eq!(removed.affected, vec!["2026/a.jpg".to_string()]);
        assert_eq!(show(&db, "favs").unwrap().members.len(), 1);
        let _ = (a, b);
    }

    #[test]
    fn rating_is_per_membership_not_per_file() {
        let (db, _, _) = setup();
        let vault = Path::new("/vault");
        create(&db, "keep").unwrap();
        create(&db, "review").unwrap();
        add(&db, vault, "keep", &["2026/a.jpg".into()]).unwrap();
        add(&db, vault, "review", &["2026/a.jpg".into()]).unwrap();

        rate(&db, vault, "keep", Some(5), &["2026/a.jpg".into()]).unwrap();
        rate(&db, vault, "review", Some(2), &["2026/a.jpg".into()]).unwrap();

        let keep = show(&db, "keep").unwrap();
        let review = show(&db, "review").unwrap();
        assert_eq!(keep.members[0].rating, Some(5));
        assert_eq!(review.members[0].rating, Some(2));

        // Clear via None; rating a non-member is skipped, not invented.
        rate(&db, vault, "keep", None, &["2026/a.jpg".into()]).unwrap();
        assert_eq!(show(&db, "keep").unwrap().members[0].rating, None);
        let not_member = rate(&db, vault, "keep", Some(1), &["2026/b.jpg".into()]).unwrap();
        assert!(not_member.affected.is_empty());
        assert_eq!(not_member.skipped[0].reason, "not a member (add it first)");

        assert!(rate(&db, vault, "keep", Some(6), &["2026/a.jpg".into()]).is_err());
    }

    #[test]
    fn delete_requires_empty_album() {
        let (db, _, _) = setup();
        let vault = Path::new("/vault");
        create(&db, "parent/child").unwrap();
        assert!(delete(&db, "parent").is_err(), "has a child");

        add(&db, vault, "parent/child", &["2026/a.jpg".into()]).unwrap();
        assert!(delete(&db, "parent/child").is_err(), "has members");

        remove(&db, vault, "parent/child", &["2026/a.jpg".into()]).unwrap();
        delete(&db, "parent/child").unwrap();
        delete(&db, "parent").unwrap();
        assert!(list(&db).unwrap().is_empty());
    }

    #[test]
    fn absolute_paths_are_relativized_to_vault() {
        let (db, _, _) = setup();
        let vault = Path::new("/vault");
        create(&db, "favs").unwrap();
        let change = add(&db, vault, "favs", &["/vault/2026/a.jpg".into()]).unwrap();
        assert_eq!(change.affected, vec!["2026/a.jpg".to_string()]);
    }
}
