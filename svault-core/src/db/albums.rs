//! Album storage: hierarchical albums with per-membership ratings.
//!
//! - `albums` is an adjacency list (`parent_id`); sibling names are unique
//!   (see `idx_albums_sibling` in the schema).
//! - `album_items` references `files.id` — stable because `files` rows are
//!   never physically deleted (enforced by the FK itself) and the DB is
//!   never rebuilt by id.
//! - `rating` lives on the membership, not on the file: the same photo may
//!   be rated differently in different albums.

use rusqlite::{OptionalExtension, Result, params};

use super::Db;

/// One album row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumRow {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub created_at: i64,
}

fn album_row_from_row(row: &rusqlite::Row) -> Result<AlbumRow> {
    Ok(AlbumRow {
        id: row.get(0)?,
        parent_id: row.get(1)?,
        name: row.get(2)?,
        created_at: row.get(3)?,
    })
}

/// One album member, joined with its vault-relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumItemRow {
    pub file_id: i64,
    pub path: String,
    pub rating: Option<i64>,
    pub added_at: i64,
}

impl Db {
    /// Insert an album; returns its id. Sibling-name conflicts surface as
    /// a constraint error from `idx_albums_sibling`.
    pub fn album_create(&self, parent_id: Option<i64>, name: &str, created_at: i64) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO albums (parent_id, name, created_at) VALUES (?1, ?2, ?3)",
            params![parent_id, name, created_at],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Fetch one album by parent + name (`None` parent = root level).
    pub fn album_get(&self, parent_id: Option<i64>, name: &str) -> Result<Option<AlbumRow>> {
        let sql = match parent_id {
            Some(_) => {
                "SELECT id, parent_id, name, created_at FROM albums \
                 WHERE parent_id = ?1 AND name = ?2 LIMIT 1"
            }
            None => {
                "SELECT id, parent_id, name, created_at FROM albums \
                 WHERE parent_id IS NULL AND name = ?2 LIMIT 1"
            }
        };
        self.conn
            .query_row(sql, params![parent_id, name], album_row_from_row)
            .optional()
    }

    /// Fetch one album by id.
    pub fn album_get_by_id(&self, id: i64) -> Result<Option<AlbumRow>> {
        self.conn
            .query_row(
                "SELECT id, parent_id, name, created_at FROM albums WHERE id = ?1",
                [id],
                album_row_from_row,
            )
            .optional()
    }

    /// All albums (unordered; the caller builds the tree).
    pub fn albums_all(&self) -> Result<Vec<AlbumRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, parent_id, name, created_at FROM albums")?;
        let rows = stmt.query_map([], album_row_from_row)?;
        rows.collect()
    }

    /// Number of direct child albums.
    pub fn album_child_count(&self, id: i64) -> Result<i64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM albums WHERE parent_id = ?1",
            [id],
            |row| row.get(0),
        )
    }

    /// Number of direct members.
    pub fn album_item_count(&self, id: i64) -> Result<i64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM album_items WHERE album_id = ?1",
            [id],
            |row| row.get(0),
        )
    }

    /// Delete an album row. Callers MUST ensure it is empty (no children,
    /// no members) — this function deletes only the album row itself.
    pub fn album_delete(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM albums WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Add a member; returns false when the file is already a member.
    pub fn album_item_add(&self, album_id: i64, file_id: i64, added_at: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO album_items (album_id, file_id, added_at) \
             VALUES (?1, ?2, ?3)",
            params![album_id, file_id, added_at],
        )?;
        Ok(changed > 0)
    }

    /// Remove a member; returns false when it was not a member.
    pub fn album_item_remove(&self, album_id: i64, file_id: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "DELETE FROM album_items WHERE album_id = ?1 AND file_id = ?2",
            params![album_id, file_id],
        )?;
        Ok(changed > 0)
    }

    /// Set (or clear, `None`) a member's rating; returns false when the
    /// file is not a member. Rating values are validated by the caller.
    pub fn album_item_set_rating(
        &self,
        album_id: i64,
        file_id: i64,
        rating: Option<i64>,
    ) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE album_items SET rating = ?3 WHERE album_id = ?1 AND file_id = ?2",
            params![album_id, file_id, rating],
        )?;
        Ok(changed > 0)
    }

    /// List members (oldest first), joined with the vault-relative path.
    pub fn album_items(&self, album_id: i64) -> Result<Vec<AlbumItemRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.file_id, f.path, i.rating, i.added_at \
             FROM album_items i JOIN files f ON f.id = i.file_id \
             WHERE i.album_id = ?1 ORDER BY i.added_at, i.file_id",
        )?;
        let rows = stmt.query_map([album_id], |row| {
            Ok(AlbumItemRow {
                file_id: row.get(0)?,
                path: row.get(1)?,
                rating: row.get(2)?,
                added_at: row.get(3)?,
            })
        })?;
        rows.collect()
    }
}
