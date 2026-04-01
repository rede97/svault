//! Vault statistics and status queries.

use rusqlite::Result;

use super::Db;

/// Overall vault statistics for `svault status`.
#[derive(Debug, Clone, Default)]
pub struct VaultStats {
    /// Total number of files in the database.
    pub total_files: i64,
    /// Total number of events in the event log.
    pub total_events: i64,
    /// Total size of all files (in bytes).
    pub total_size_bytes: i64,
    /// Number of files with status 'imported'.
    pub imported_count: i64,
    /// Number of files with status 'duplicate'.
    pub duplicate_count: i64,
    /// Number of files with xxh3_128 but no sha256 (pending background hash).
    pub pending_sha256_count: i64,
    /// Number of files with sha256 computed.
    pub has_sha256_count: i64,
    /// Database file size (in bytes).
    pub db_size_bytes: i64,
}

/// Storage statistics by file extension.
#[derive(Debug, Clone)]
pub struct ExtensionStats {
    /// File extension (lowercase, without dot).
    pub extension: String,
    /// Number of files with this extension.
    pub count: i64,
    /// Total size of files with this extension (in bytes).
    pub total_size_bytes: i64,
}

impl Db {
    /// Returns overall vault statistics.
    pub fn vault_stats(&self) -> Result<VaultStats> {
        // Total files and size
        let (total_files, total_size_bytes): (i64, i64) = self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM files",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        // Total events
        let total_events: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM events",
            [],
            |row| row.get(0),
        )?;

        // Status counts
        let imported_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM files WHERE status = 'imported'",
            [],
            |row| row.get(0),
        )?;

        let duplicate_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM files WHERE status = 'duplicate'",
            [],
            |row| row.get(0),
        )?;

        // Hash computation status
        let pending_sha256_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM files WHERE xxh3_128 IS NOT NULL AND sha256 IS NULL",
            [],
            |row| row.get(0),
        )?;

        let has_sha256_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM files WHERE sha256 IS NOT NULL",
            [],
            |row| row.get(0),
        )?;

        // Database file size (SQLite page_count * page_size)
        let db_size_bytes: i64 = self.conn.query_row(
            "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
            [],
            |row| row.get(0),
        )?;

        Ok(VaultStats {
            total_files,
            total_events,
            total_size_bytes,
            imported_count,
            duplicate_count,
            pending_sha256_count,
            has_sha256_count,
            db_size_bytes,
        })
    }

    /// Returns storage statistics grouped by file extension.
    /// Limited to top N extensions by total size.
    pub fn extension_stats(&self, limit: i64) -> Result<Vec<ExtensionStats>> {
        let mut stmt = self.conn.prepare(
            "SELECT 
                LOWER(SUBSTR(path, INSTR(path, '.') + 1)) as ext,
                COUNT(*) as cnt,
                SUM(size) as total_size
             FROM files
             WHERE path LIKE '%.%'
             GROUP BY ext
             ORDER BY total_size DESC
             LIMIT ?1"
        )?;

        let rows = stmt.query_map([limit], |row| {
            Ok(ExtensionStats {
                extension: row.get(0)?,
                count: row.get(1)?,
                total_size_bytes: row.get(2)?,
            })
        })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Returns the number of files imported in the last N days.
    pub fn recent_imports(&self, days: i64) -> Result<i64> {
        let cutoff_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
            - (days * 24 * 60 * 60 * 1000);

        self.conn.query_row(
            "SELECT COUNT(*) FROM files WHERE imported_at > ?1",
            [cutoff_ms],
            |row| row.get(0),
        )
    }
}

/// Formats bytes into human-readable string (B, KB, MB, GB, TB).
pub fn format_bytes(bytes: i64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[unit_idx])
    } else {
        format!("{:.2} {}", size, UNITS[unit_idx])
    }
}

/// Formats a count with thousands separator.
pub fn format_count(n: i64) -> String {
    let mut result = String::new();
    let s = n.to_string();
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();

    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(*c);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.00 GB");
    }

    #[test]
    fn test_format_count() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1000), "1,000");
        assert_eq!(format_count(1234567), "1,234,567");
    }
}
