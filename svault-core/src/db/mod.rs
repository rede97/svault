//! SQLite database for the vault.
//!
//! The `files` table is the operational state index (path / size / hashes /
//! status); `assets`, `media_groups`, and `derivatives` back media grouping.
//! State changes are written in single transactions (see
//! [`Db::with_transaction`]); per-session history lives outside the DB in
//! `.svault/sessions/` (plan + manifest JSON), not in the database.

pub mod dump;
pub mod files;
pub mod stats;

pub use dump::{
    DumpOptions, DumpResult, TableDump, dump_database, dump_table, list_tables, render_csv,
    render_json, render_sql,
};
pub use files::FileRow;
pub use stats::{ExtensionStats, VaultStats, format_bytes, format_count};

use rusqlite::{Connection, Result};
use std::path::Path;

/// Initialize a new vault at `root`.
/// Creates `.svault/` and the database inside it.
pub fn init(root: &Path) -> anyhow::Result<()> {
    let svault_dir = root.join(".svault");
    if svault_dir.exists() {
        anyhow::bail!("vault already initialized at {}", svault_dir.display());
    }
    std::fs::create_dir_all(&svault_dir)?;
    let db_path = svault_dir.join("vault.db");
    Db::open(&db_path)?;
    crate::config::Config::write_default(root)?;
    Ok(())
}

/// A handle to the Svault SQLite database.
pub struct Db {
    conn: Connection,
}

impl Db {
    /// Open (or create) the database at `path`. Runs all schema migrations.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Get a reference to the underlying SQLite connection.
    /// This allows running custom queries.
    pub fn conn_ref(&self) -> &Connection {
        &self.conn
    }

    /// Open an in-memory database (used for testing).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Open an existing database read-only.
    ///
    /// Used by `svault sync` to inspect a peer vault without holding a write
    /// lock or running migrations against it. Fails if the file does not exist.
    pub fn open_readonly(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        Ok(Self { conn })
    }

    /// Apply schema migrations idempotently.
    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(SCHEMA)
    }

    /// Execute a function within a database transaction.
    /// The transaction is committed if `f` returns Ok, otherwise rolled back.
    pub fn with_transaction<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> Result<R>,
    {
        let tx = self.conn.unchecked_transaction()?;
        let result = f(&self.conn)?;
        tx.commit()?;
        Ok(result)
    }

    /// Dump database contents for debugging.
    pub fn dump(&self, tables: Vec<String>, limit: Option<usize>) -> Result<dump::DumpResult> {
        let opts = DumpOptions { tables, limit };
        dump::dump_database(&self.conn, opts)
    }
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS assets (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at  INTEGER NOT NULL,
    title       TEXT
);

CREATE TABLE IF NOT EXISTS media_groups (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id            INTEGER NOT NULL REFERENCES assets(id),
    group_type          TEXT    NOT NULL,
    content_identifier  TEXT,
    captured_at         INTEGER
);

CREATE TABLE IF NOT EXISTS files (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    xxh3_128             BLOB,              -- XXH3-128 strong hash (fast dedup)
    sha256               BLOB,              -- SHA-256 strong hash (secure identity)
    size                 INTEGER NOT NULL,  -- File size in bytes
    path                 TEXT    NOT NULL,  -- Current vault path (mutable)
    mtime                INTEGER NOT NULL,  -- Source file modification time
    group_id             INTEGER REFERENCES media_groups(id),
    role                 TEXT,              -- primary/motion/depth/auxiliary
    crc32c               INTEGER,           -- Format-specific CRC32C (see media/crc.rs)
    raw_unique_id        TEXT,              -- Camera serial + image ID for RAW files (format: serial:image_id)
    exif_fp              TEXT,              -- EXIF fingerprint for grouping
    status               TEXT    NOT NULL DEFAULT 'imported',
    duplicate_of         INTEGER REFERENCES files(id),
    imported_at          INTEGER NOT NULL
);
-- Identity rule: sha256 IS NOT NULL takes precedence over xxh3_128 as the
-- canonical content identity. If only xxh3_128 is present it serves as the
-- temporary identity until sha256 is computed (lazily, on collision or via
-- background-hash). Both are stored as raw bytes (BLOB) for compact storage
-- and fast binary comparison.

CREATE INDEX IF NOT EXISTS idx_files_sha256  ON files(sha256);
CREATE INDEX IF NOT EXISTS idx_files_xxh3    ON files(xxh3_128);
CREATE INDEX IF NOT EXISTS idx_files_size    ON files(size);
CREATE INDEX IF NOT EXISTS idx_files_group   ON files(group_id);

CREATE TABLE IF NOT EXISTS derivatives (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id        INTEGER NOT NULL REFERENCES assets(id),
    source_file_id  INTEGER NOT NULL REFERENCES files(id),
    deriv_type      TEXT    NOT NULL,
    params          TEXT,
    path            TEXT,
    created_at      INTEGER NOT NULL
);
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_open_in_memory_creates_valid_db() {
        let db = Db::open_in_memory().unwrap();
        // Verify tables exist by querying
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count > 0, "Database should have tables");

        // The removed event-sourcing table must not be recreated (2026-08-09:
        // events/verify-chain removed as a pseudo-requirement; per-session
        // history lives in .svault/sessions/ instead).
        let events_exists: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='events'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(events_exists, 0);
    }

    #[test]
    fn db_open_in_memory_is_isolated() {
        let db1 = Db::open_in_memory().unwrap();
        let db2 = Db::open_in_memory().unwrap();

        db1.conn
            .execute(
                "INSERT INTO files (size, path, mtime, imported_at) VALUES (1, 'a.jpg', 0, 0)",
                [],
            )
            .unwrap();

        let count1: i64 = db1
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
            .unwrap();
        let count2: i64 = db2
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
            .unwrap();

        assert_eq!(count1, 1);
        assert_eq!(count2, 0);
    }
}
