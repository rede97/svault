//! Event-sourced SQLite database.
//!
//! All state changes are recorded as append-only events in the `events` table.
//! The `files`, `assets`, `media_groups`, and `derivatives` tables are
//! materialised views rebuilt by replaying those events.
//!
//! Write protocol (any state change):
//!   1. Construct event payload.
//!   2. Read previous event's `self_hash` → `prev_hash`.
//!   3. Compute `self_hash = SHA-256(seq || occurred_at || event_type || entity_id || payload || prev_hash)`.
//!   4. INSERT INTO events  (append-only).
//!   5. UPDATE materialised view table.
//!   6. COMMIT (steps 4+5 in one transaction).

pub mod files;

pub use files::FileRow;

use rusqlite::{Connection, OptionalExtension, Result, params};
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
    println!("Initialized empty svault at {}", svault_dir.display());
    Ok(())
}

/// The genesis prev_hash used for the very first event in a vault.
const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

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

    /// Open an in-memory database (used for testing).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Apply schema migrations idempotently.
    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(SCHEMA)
    }

    /// Returns the `self_hash` of the most recent event, or the genesis hash
    /// if no events exist yet.
    pub fn last_event_hash(&self) -> Result<String> {
        let hash: Option<String> = self.conn.query_row(
            "SELECT self_hash FROM events ORDER BY seq DESC LIMIT 1",
            [],
            |row| row.get(0),
        ).optional()?;
        Ok(hash.unwrap_or_else(|| GENESIS_HASH.to_string()))
    }

    /// Appends a raw event and updates the materialised view in one transaction.
    /// `update_fn` receives the connection and should UPDATE/INSERT the
    /// relevant materialised-view row.
    pub fn append_event(
        &self,
        event_type: &str,
        entity_type: &str,
        entity_id: i64,
        payload: &str,
        update_fn: impl FnOnce(&Connection) -> Result<()>,
    ) -> Result<()> {
        let prev_hash = self.last_event_hash()?;
        let occurred_at = unix_now_ms();
        let self_hash = compute_event_hash(
            event_type,
            entity_type,
            entity_id,
            payload,
            occurred_at,
            &prev_hash,
        );

        self.conn.execute(
            "INSERT INTO events \
             (occurred_at, event_type, entity_type, entity_id, payload, prev_hash, self_hash) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                occurred_at,
                event_type,
                entity_type,
                entity_id,
                payload,
                prev_hash,
                self_hash,
            ],
        )?;

        update_fn(&self.conn)?;
        Ok(())
    }

    /// Verify the entire event hash chain. Returns `Ok(())` if intact, or an
    /// error describing the first broken link (by seq number).
    pub fn verify_chain(&self) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, event_type, entity_type, entity_id, payload, occurred_at, prev_hash, self_hash \
             FROM events ORDER BY seq",
        )?;

        let mut prev_hash = GENESIS_HASH.to_string();

        let rows = stmt.query_map([], |row| {
            Ok(EventRow {
                seq: row.get(0)?,
                event_type: row.get(1)?,
                entity_type: row.get(2)?,
                entity_id: row.get(3)?,
                payload: row.get(4)?,
                occurred_at: row.get(5)?,
                prev_hash: row.get(6)?,
                self_hash: row.get(7)?,
            })
        })?;

        for row in rows {
            let ev = row?;
            if ev.prev_hash != prev_hash {
                return Err(rusqlite::Error::InvalidParameterName(
                    format!("chain broken at seq {}: prev_hash mismatch", ev.seq),
                ));
            }
            let expected = compute_event_hash(
                &ev.event_type,
                &ev.entity_type,
                ev.entity_id,
                &ev.payload,
                ev.occurred_at,
                &ev.prev_hash,
            );
            if ev.self_hash != expected {
                return Err(rusqlite::Error::InvalidParameterName(
                    format!("chain broken at seq {}: self_hash mismatch", ev.seq),
                ));
            }
            prev_hash = ev.self_hash;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

struct EventRow {
    seq: i64,
    event_type: String,
    entity_type: String,
    entity_id: i64,
    payload: String,
    occurred_at: i64,
    prev_hash: String,
    self_hash: String,
}

fn unix_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Compute `self_hash = SHA-256(seq || occurred_at || event_type || entity_id || payload || prev_hash)`.
/// We use the printable concatenation as input to keep it auditable.
fn compute_event_hash(
    event_type: &str,
    entity_type: &str,
    entity_id: i64,
    payload: &str,
    occurred_at: i64,
    prev_hash: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let input = format!(
        "{}|{}|{}|{}|{}|{}",
        occurred_at, event_type, entity_type, entity_id, payload, prev_hash
    );
    let result = Sha256::digest(input.as_bytes());
    result.iter().map(|b| format!("{:02x}", b)).collect()
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS events (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    occurred_at  INTEGER NOT NULL,
    event_type   TEXT    NOT NULL,
    entity_type  TEXT    NOT NULL,
    entity_id    INTEGER NOT NULL,
    payload      TEXT    NOT NULL,
    prev_hash    TEXT    NOT NULL,
    self_hash    TEXT    NOT NULL
);

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
    xxh3_128             BLOB,
    sha256               BLOB,
    size                 INTEGER NOT NULL,
    path                 TEXT    NOT NULL,
    mtime                INTEGER NOT NULL,
    group_id             INTEGER REFERENCES media_groups(id),
    role                 TEXT,
    crc32c_val           INTEGER,
    crc32c_region        TEXT,
    crc32c_handler_ver   TEXT,
    exif_fp              TEXT,
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
