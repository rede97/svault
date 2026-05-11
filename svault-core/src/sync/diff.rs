//! SHA-256 diff engine for vault→vault sync.
//!
//! Builds a `HashSet<[u8; 32]>` from the target vault's SHA-256 hashes,
//! then iterates source files to determine which need transfer.

use std::collections::HashSet;

use crate::db::{Db, FileRow};
use crate::reporting::SyncDiffReporter;

/// Result of comparing source vault files against target vault.
#[derive(Debug, Clone, Default)]
pub struct SyncDiff {
    /// Files to transfer (present in source, missing from target by content hash).
    pub to_copy: Vec<FileRow>,
    /// Count of files on source that already exist on target.
    pub skipped: usize,
    /// Total bytes to copy for files in `to_copy`.
    pub total_bytes: u64,
}

/// Compute the diff between two vault databases by SHA-256 content identity.
///
/// # Algorithm
///
/// 1. Query all imported files with SHA-256 from the source DB.
/// 2. Query all imported files with SHA-256 from the target DB.
/// 3. Build a `HashSet<[u8; 32]>` of target SHA-256 values.
/// 4. For each source file, check if its SHA-256 is present in the target set.
///
/// Files without SHA-256 are excluded. Use `svault verify --background-hash`
/// to compute SHA-256 for all files before syncing.
///
/// `finish()` is NOT called here — the caller decides whether to emit
/// `nothing_to_sync()` before calling `finish()`.
pub fn compute_vault_diff<DR: SyncDiffReporter>(
    source_db: &Db,
    target_db: &Db,
    reporter: &DR,
) -> anyhow::Result<SyncDiff> {
    let source_files: Vec<FileRow> = source_db
        .get_all_files()?
        .into_iter()
        .filter(|f| f.status == "imported" && f.sha256.is_some())
        .collect();

    let target_files: Vec<FileRow> = target_db
        .get_all_files()?
        .into_iter()
        .filter(|f| f.status == "imported" && f.sha256.is_some())
        .collect();

    reporter.started(source_files.len(), target_files.len());

    let target_sha256_set: HashSet<[u8; 32]> = target_files
        .iter()
        .filter_map(|f| {
            f.sha256
                .as_ref()
                .and_then(|v| v.as_slice().try_into().ok())
        })
        .collect();

    let mut to_copy = Vec::new();
    let mut skipped = 0usize;
    let mut total_bytes = 0u64;

    for f in source_files {
        let sha256_bytes: [u8; 32] = match f.sha256.as_ref().and_then(|v| v.as_slice().try_into().ok())
        {
            Some(b) => b,
            None => continue,
        };

        if target_sha256_set.contains(&sha256_bytes) {
            skipped += 1;
        } else {
            total_bytes += f.size as u64;
            to_copy.push(f);
        }
    }

    reporter.diff_computed(to_copy.len(), skipped, total_bytes);

    Ok(SyncDiff { to_copy, skipped, total_bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::reporting::Noop;
    use rusqlite::params;

    fn insert_test_file(db: &Db, path: &str, size: i64, sha256: Option<[u8; 32]>) {
        db.conn_ref().execute(
            "INSERT INTO files (path, size, mtime, sha256, status, imported_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                path,
                size,
                0i64,
                sha256.as_ref().map(|v| v.to_vec()),
                "imported",
                0i64,
            ],
        )
        .unwrap();
    }

    #[test]
    fn empty_source_and_target() {
        let source = Db::open_in_memory().unwrap();
        let target = Db::open_in_memory().unwrap();
        let diff = compute_vault_diff(&source, &target, &Noop).unwrap();
        assert!(diff.to_copy.is_empty());
        assert_eq!(diff.skipped, 0);
        assert_eq!(diff.total_bytes, 0);
    }

    #[test]
    fn all_new_files() {
        let source = Db::open_in_memory().unwrap();
        let target = Db::open_in_memory().unwrap();
        insert_test_file(&source, "a.jpg", 100, Some([1u8; 32]));

        let diff = compute_vault_diff(&source, &target, &Noop).unwrap();
        assert_eq!(diff.to_copy.len(), 1);
        assert_eq!(diff.skipped, 0);
        assert_eq!(diff.total_bytes, 100);
    }

    #[test]
    fn file_already_on_target_is_skipped() {
        let source = Db::open_in_memory().unwrap();
        let target = Db::open_in_memory().unwrap();
        let sha = [0xABu8; 32];
        insert_test_file(&source, "a.jpg", 100, Some(sha));
        insert_test_file(&target, "a.jpg", 100, Some(sha));

        let diff = compute_vault_diff(&source, &target, &Noop).unwrap();
        assert!(diff.to_copy.is_empty());
        assert_eq!(diff.skipped, 1);
    }

    #[test]
    fn files_without_sha256_are_excluded() {
        let source = Db::open_in_memory().unwrap();
        let target = Db::open_in_memory().unwrap();
        source.conn_ref().execute(
            "INSERT INTO files (path, size, mtime, xxh3_128, status, imported_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["b.jpg", 200i64, 0i64, vec![2u8; 16], "imported", 0i64],
        ).unwrap();

        let diff = compute_vault_diff(&source, &target, &Noop).unwrap();
        assert!(diff.to_copy.is_empty());
        assert_eq!(diff.skipped, 0);
    }

    #[test]
    fn mixed_scenario() {
        let source = Db::open_in_memory().unwrap();
        let target = Db::open_in_memory().unwrap();

        let sha1 = [1u8; 32];
        let sha2 = [2u8; 32];
        let sha3 = [3u8; 32];

        insert_test_file(&source, "new.jpg", 100, Some(sha1));
        insert_test_file(&source, "common.jpg", 200, Some(sha2));
        insert_test_file(&source, "another.jpg", 300, Some(sha3));
        insert_test_file(&target, "common.jpg", 200, Some(sha2));

        let diff = compute_vault_diff(&source, &target, &Noop).unwrap();
        assert_eq!(diff.to_copy.len(), 2);
        assert_eq!(diff.skipped, 1);
        assert_eq!(diff.total_bytes, 400);
    }
}
