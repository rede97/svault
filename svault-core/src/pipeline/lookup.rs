//! DB lookup stage: Check for duplicates using CRC32C + RAW ID.

use std::path::Path;

use crate::db::Db;
use crate::pipeline::types::{CrcEntry, FileStatus, LookupResult};

/// Lookup files in DB to check for duplicates.
///
/// For each entry:
/// 1. Query DB by CRC32C + size + extension + RAW ID
/// 2. Check if vault file still exists (may have been deleted)
/// 3. Check RAW ID match for precise duplicate detection
///
/// # Arguments
/// * `entries` - CRC entries from Stage B
/// * `db` - Database handle
/// * `vault_root` - Vault root path
/// * `show_progress` - Whether to print per-file status
///
/// # Returns
/// List of lookup results with status (LikelyNew / LikelyCacheDuplicate / Failed)
pub fn lookup_duplicates(
    entries: Vec<CrcEntry>,
    db: &Db,
    vault_root: &Path,
) -> anyhow::Result<Vec<LookupResult>> {
    let mut results = Vec::with_capacity(entries.len());

    for entry in entries {
        // Get file extension for format-specific lookup
        let ext = entry
            .file
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        // DB lookup with CRC32C + RAW ID
        let cached = db
            .lookup_by_crc32c(
                entry.file.size as i64,
                entry.crc32c,
                ext,
                entry.raw_unique_id.as_deref(),
            )
            .unwrap_or(None);

        let status = if let Some(ref row) = cached {
            // For RAW files with unique IDs, check if IDs match
            let is_same_raw_id = match (&entry.raw_unique_id, &row.raw_unique_id) {
                (Some(new_id), Some(existing_id)) => new_id == existing_id,
                _ => true, // Can't compare, fall back to CRC-only
            };

            // Check if vault file still exists
            let vault_path = vault_root.join(&row.path);
            if vault_path.exists() && is_same_raw_id {
                FileStatus::LikelyCacheDuplicate
            } else {
                FileStatus::LikelyNew
            }
        } else {
            FileStatus::LikelyNew
        };

        results.push(LookupResult { entry, status });
    }

    Ok(results)
}

/// Filter to likely new files (with optional force mode).
pub fn filter_new(
    results: Vec<LookupResult>,
    force: bool,
) -> (Vec<CrcEntry>, Vec<CrcEntry>) {
    let mut new_files = Vec::new();
    let mut duplicates = Vec::new();

    for r in results {
        match r.status {
            FileStatus::LikelyNew => {
                new_files.push(r.entry);
            }
            FileStatus::LikelyCacheDuplicate => {
                if force {
                    new_files.push(r.entry);
                } else {
                    duplicates.push(r.entry);
                }
            }
            FileStatus::Failed(_) => {
                // Already tracked as failed
            }
        }
    }

    (new_files, duplicates)
}

/// Count files by status.
pub fn count_by_status(results: &[LookupResult]) -> (usize, usize, usize) {
    let new_count = results
        .iter()
        .filter(|r| matches!(r.status, FileStatus::LikelyNew))
        .count();
    let dup_count = results
        .iter()
        .filter(|r| matches!(r.status, FileStatus::LikelyCacheDuplicate))
        .count();
    let fail_count = results
        .iter()
        .filter(|r| matches!(r.status, FileStatus::Failed(_)))
        .count();

    (new_count, dup_count, fail_count)
}
