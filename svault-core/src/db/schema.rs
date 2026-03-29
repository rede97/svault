use rusqlite::Connection;

/// Create all required tables in a fresh database.
pub fn initialize(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch("
        PRAGMA journal_mode = WAL;

        CREATE TABLE IF NOT EXISTS files (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            path        TEXT    NOT NULL UNIQUE,
            hash        TEXT    NOT NULL,
            size        INTEGER NOT NULL,
            created_at  INTEGER NOT NULL DEFAULT (strftime('%s','now')),
            updated_at  INTEGER NOT NULL DEFAULT (strftime('%s','now'))
        );

        CREATE TABLE IF NOT EXISTS chunks (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            file_id     INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
            chunk_index INTEGER NOT NULL,
            hash        TEXT    NOT NULL,
            size        INTEGER NOT NULL,
            UNIQUE(file_id, chunk_index)
        );
    ")?;
    Ok(())
}
