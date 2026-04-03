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

pub mod dump;
pub mod files;
pub mod stats;

pub use dump::{TableDump, DumpOptions, dump_database, dump_table, list_tables, 
               render_csv, render_json, render_sql};
pub use files::FileRow;
pub use stats::{VaultStats, ExtensionStats, format_bytes, format_count};

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

    /// Dump database contents for debugging.
    pub fn dump(&self, tables: Vec<String>, limit: Option<usize>) -> Result<Vec<TableDump>> {
        let opts = DumpOptions { tables, limit };
        dump::dump_database(&self.conn, opts)
    }

    /// Query events with optional filters.
    pub fn get_events(
        &self,
        limit: usize,
        event_type: Option<&str>,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
        file_path: Option<&str>,
    ) -> Result<Vec<EventRow>> {
        let path_like_1 = file_path.map(|p| format!("%\"path\":\"{}\"%", p));
        let path_like_2 = file_path.map(|p| format!("%\"old_path\":\"{}\"%", p));
        let path_like_3 = file_path.map(|p| format!("%\"new_path\":\"{}\"%", p));

        let sql = String::from(
            "SELECT seq, occurred_at, event_type, entity_type, entity_id, payload, prev_hash, self_hash \
             FROM events \
             WHERE (?1 IS NULL OR event_type = ?1) \
               AND (?2 IS NULL OR occurred_at >= ?2) \
               AND (?3 IS NULL OR occurred_at <= ?3) \
               AND (?4 IS NULL OR payload LIKE ?5 OR payload LIKE ?6 OR payload LIKE ?7) \
             ORDER BY seq DESC \
             LIMIT ?8"
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![
                event_type,
                from_ms,
                to_ms,
                file_path,
                path_like_1.as_deref().unwrap_or(""),
                path_like_2.as_deref().unwrap_or(""),
                path_like_3.as_deref().unwrap_or(""),
                limit as i64,
            ],
            |row| {
                Ok(EventRow {
                    seq: row.get(0)?,
                    occurred_at: row.get(1)?,
                    event_type: row.get(2)?,
                    entity_type: row.get(3)?,
                    entity_id: row.get(4)?,
                    payload: row.get(5)?,
                    prev_hash: row.get(6)?,
                    self_hash: row.get(7)?,
                })
            },
        )?;
        rows.collect()
    }
}

/// A single event from the event log.
#[derive(Debug, Clone)]
pub struct EventRow {
    pub seq: i64,
    pub occurred_at: i64,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: i64,
    pub payload: String,
    pub prev_hash: String,
    pub self_hash: String,
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

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

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Database lifecycle tests
    // -------------------------------------------------------------------------

    #[test]
    fn db_open_in_memory_creates_valid_db() {
        let db = Db::open_in_memory().unwrap();
        // Verify tables exist by querying
        let count: i64 = db.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(count > 0, "Database should have tables");
    }

    #[test]
    fn db_open_in_memory_is_isolated() {
        let db1 = Db::open_in_memory().unwrap();
        let db2 = Db::open_in_memory().unwrap();
        
        // Add event to db1
        db1.append_event("test", "file", 1, r#"{"path":"test.txt"}"#, |_conn| Ok(())).unwrap();
        
        // db2 should not see it
        let count1: i64 = db1.conn.query_row(
            "SELECT COUNT(*) FROM events",
            [],
            |row| row.get(0),
        ).unwrap();
        let count2: i64 = db2.conn.query_row(
            "SELECT COUNT(*) FROM events",
            [],
            |row| row.get(0),
        ).unwrap();
        
        assert_eq!(count1, 1);
        assert_eq!(count2, 0);
    }

    // -------------------------------------------------------------------------
    // Event hash chain tests
    // -------------------------------------------------------------------------

    #[test]
    fn last_event_hash_returns_genesis_for_empty_db() {
        let db = Db::open_in_memory().unwrap();
        let hash = db.last_event_hash().unwrap();
        assert_eq!(hash, GENESIS_HASH);
    }

    #[test]
    fn append_event_creates_valid_chain() {
        let db = Db::open_in_memory().unwrap();
        
        // Append first event
        db.append_event("import", "file", 1, r#"{"path":"a.jpg"}"#, |_conn| {
            // Could update materialized view here
            Ok(())
        }).unwrap();
        
        let hash1 = db.last_event_hash().unwrap();
        assert_ne!(hash1, GENESIS_HASH);
        assert_eq!(hash1.len(), 64); // SHA-256 hex
        
        // Append second event
        db.append_event("import", "file", 2, r#"{"path":"b.jpg"}"#, |_conn| Ok(())).unwrap();
        
        let hash2 = db.last_event_hash().unwrap();
        assert_ne!(hash2, hash1);
        assert_ne!(hash2, GENESIS_HASH);
    }

    #[test]
    fn verify_chain_passes_for_valid_chain() {
        let db = Db::open_in_memory().unwrap();
        
        // Add several events
        for i in 1..=3 {
            db.append_event("import", "file", i, &format!(r#"{{"path":"file{}.jpg"}}"#, i), |_conn| Ok(())).unwrap();
        }
        
        // Should verify without error
        db.verify_chain().unwrap();
    }

    #[test]
    fn verify_chain_detects_tampering() {
        let db = Db::open_in_memory().unwrap();
        
        // Add an event
        db.append_event("import", "file", 1, r#"{"path":"a.jpg"}"#, |_conn| Ok(())).unwrap();
        
        // Tamper with the event (direct SQL update)
        db.conn.execute(
            "UPDATE events SET payload = ? WHERE seq = 1",
            [r#"{"path":"tampered.jpg"}"#],
        ).unwrap();
        
        // Verification should fail
        let result = db.verify_chain();
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("chain broken") || err_msg.contains("mismatch"));
    }

    // -------------------------------------------------------------------------
    // Event query tests
    // -------------------------------------------------------------------------

    #[test]
    fn get_events_returns_events_in_descending_order() {
        let db = Db::open_in_memory().unwrap();
        
        // Add events with small delay for ordering
        db.append_event("import", "file", 1, "{}", |_conn| Ok(())).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        db.append_event("import", "file", 2, "{}", |_conn| Ok(())).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        db.append_event("update", "file", 1, "{}", |_conn| Ok(())).unwrap();
        
        let events = db.get_events(10, None, None, None, None).unwrap();
        assert_eq!(events.len(), 3);
        
        // Should be in descending seq order
        assert!(events[0].seq > events[1].seq);
        assert!(events[1].seq > events[2].seq);
    }

    #[test]
    fn get_events_filters_by_event_type() {
        let db = Db::open_in_memory().unwrap();
        
        db.append_event("import", "file", 1, "{}", |_conn| Ok(())).unwrap();
        db.append_event("update", "file", 1, "{}", |_conn| Ok(())).unwrap();
        db.append_event("import", "file", 2, "{}", |_conn| Ok(())).unwrap();
        
        let import_events = db.get_events(10, Some("import"), None, None, None).unwrap();
        assert_eq!(import_events.len(), 2);
        for ev in &import_events {
            assert_eq!(ev.event_type, "import");
        }
        
        let update_events = db.get_events(10, Some("update"), None, None, None).unwrap();
        assert_eq!(update_events.len(), 1);
    }

    #[test]
    fn get_events_respects_limit() {
        let db = Db::open_in_memory().unwrap();
        
        for i in 1..=5 {
            db.append_event("import", "file", i, "{}", |_conn| Ok(())).unwrap();
        }
        
        let events = db.get_events(3, None, None, None, None).unwrap();
        assert_eq!(events.len(), 3);
    }

    // -------------------------------------------------------------------------
    // Event hash computation tests
    // -------------------------------------------------------------------------

    #[test]
    fn compute_event_hash_is_deterministic() {
        let hash1 = compute_event_hash("import", "file", 1, r#"{"path":"a.jpg"}"#, 12345, GENESIS_HASH);
        let hash2 = compute_event_hash("import", "file", 1, r#"{"path":"a.jpg"}"#, 12345, GENESIS_HASH);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64);
    }

    #[test]
    fn compute_event_hash_changes_with_input() {
        let base = compute_event_hash("import", "file", 1, r#"{"path":"a.jpg"}"#, 12345, GENESIS_HASH);
        
        // Different event type
        let h2 = compute_event_hash("update", "file", 1, r#"{"path":"a.jpg"}"#, 12345, GENESIS_HASH);
        assert_ne!(base, h2);
        
        // Different payload
        let h3 = compute_event_hash("import", "file", 1, r#"{"path":"b.jpg"}"#, 12345, GENESIS_HASH);
        assert_ne!(base, h3);
        
        // Different timestamp
        let h4 = compute_event_hash("import", "file", 1, r#"{"path":"a.jpg"}"#, 12346, GENESIS_HASH);
        assert_ne!(base, h4);
        
        // Different prev_hash
        let h5 = compute_event_hash("import", "file", 1, r#"{"path":"a.jpg"}"#, 12345, &"a".repeat(64));
        assert_ne!(base, h5);
    }
}
