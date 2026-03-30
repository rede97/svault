//! DB helpers for the `files` materialised-view table.

use rusqlite::{OptionalExtension, Result, params};

use crate::config::HashAlgorithm;

use super::Db;

// ---------------------------------------------------------------------------
// Row type
// ---------------------------------------------------------------------------

/// A row from the `files` materialised-view table.
#[derive(Debug, Clone)]
pub struct FileRow {
    pub id: i64,
    pub path: String,
    pub size: i64,
    pub mtime: i64,
    /// CRC32C of the probed region (head or tail, format-dependent).
    pub crc32c_val: Option<i64>,
    /// XXH3-128 as raw 16-byte BLOB (None if not yet computed).
    pub xxh3_128: Option<Vec<u8>>,
    /// SHA-256 as raw 32-byte BLOB (None if not yet computed).
    pub sha256: Option<Vec<u8>>,
    pub status: String,
}

// ---------------------------------------------------------------------------
// Epoch helpers
// ---------------------------------------------------------------------------

impl Db {
    /// Returns the current CRC32C cache epoch from `metadata`.
    /// Epoch 1 is the default written during `init`.
    pub fn crc32c_epoch(&self) -> Result<i64> {
        let epoch: Option<String> = self.conn.query_row(
            "SELECT value FROM metadata WHERE key = 'crc32c_epoch'",
            [],
            |row| row.get(0),
        ).optional()?;
        Ok(epoch.and_then(|s| s.parse().ok()).unwrap_or(1))
    }

    // -----------------------------------------------------------------------
    // Lookup queries
    // -----------------------------------------------------------------------

    /// Look up a file by CRC32C value + size.
    /// Returns the first matching row whose `crc32c_val` and `size` match.
    /// The caller should treat a hit as `likely_duplicate` only.
    pub fn lookup_by_crc32c(&self, size: i64, crc32c: u32) -> Result<Option<FileRow>> {
        self.conn.query_row(
            "SELECT id, path, size, mtime, crc32c_val, xxh3_128, sha256, status \
             FROM files WHERE size = ?1 AND crc32c_val = ?2 LIMIT 1",
            params![size, crc32c as i64],
            file_row_from_row,
        ).optional()
    }

    /// Look up a file by its strong hash (XXH3-128 or SHA-256, stored as BLOBs).
    pub fn lookup_by_hash(&self, hash_bytes: &[u8], algo: &HashAlgorithm) -> Result<Option<FileRow>> {
        let col = match algo {
            HashAlgorithm::Xxh3_128 => "xxh3_128",
            HashAlgorithm::Sha256 => "sha256",
        };
        let sql = format!(
            "SELECT id, path, size, mtime, crc32c_val, xxh3_128, sha256, status \
             FROM files WHERE {col} = ?1 LIMIT 1"
        );
        self.conn.query_row(&sql, params![hash_bytes], file_row_from_row).optional()
    }

    // -----------------------------------------------------------------------
    // Insert
    // -----------------------------------------------------------------------

    /// Insert a newly imported file into the `files` materialised view.
    /// Must be called inside `append_event`'s `update_fn` closure.
    pub fn insert_file_row(
        &self,
        path: &str,
        size: i64,
        mtime: i64,
        crc32c: Option<u32>,
        xxh3_128: Option<&[u8]>,
        sha256: Option<&[u8]>,
        status: &str,
        imported_at: i64,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO files \
             (path, size, mtime, crc32c_val, xxh3_128, sha256, status, imported_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                path,
                size,
                mtime,
                crc32c.map(|v| v as i64),
                xxh3_128,
                sha256,
                status,
                imported_at,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn file_row_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileRow> {
    Ok(FileRow {
        id: row.get(0)?,
        path: row.get(1)?,
        size: row.get(2)?,
        mtime: row.get(3)?,
        crc32c_val: row.get(4)?,
        xxh3_128: row.get(5)?,
        sha256: row.get(6)?,
        status: row.get(7)?,
    })
}
