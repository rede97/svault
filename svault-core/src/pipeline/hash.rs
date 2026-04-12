//! Stage D: Strong hash computation.

use std::path::Path;

use dashmap::{DashMap, mapref::entry::Entry};
use rayon::prelude::*;

use crate::db::Db;
use crate::hash::{sha256_file, xxh3_128_file};
use crate::pipeline::types::{CrcEntry, FileHash, HashResult};
use crate::reporting::HashReporter;

/// Compute strong hashes for all entries in parallel.
///
/// Always computes XXH3-128 for deduplication.
/// Optionally computes SHA-256 when `compute_sha256` is true (for --force or --full-id).
///
/// # Arguments
/// * `entries` - CRC entries (from lookup stage)
/// * `compute_sha256` - If true, also compute SHA-256 for definitive identity
/// * `reporter` - Optional reporter for progress tracking
///
/// # Returns
/// List of hash results (errors preserved in result with dup_reason)
pub fn compute_hashes<R: HashReporter>(
    entries: Vec<CrcEntry>,
    compute_sha256: bool,
    reporter: Option<&R>,
) -> Vec<HashResult> {
    entries
        .into_par_iter()
        .map(|entry| {
            let abs_path = &entry.file.path;
            let size = entry.file.size;

            // Signal start of hashing this file
            if let Some(r) = reporter {
                r.item_started(abs_path, size);
            }

            // Always compute XXH3-128 for deduplication
            let xxh3_128 = match xxh3_128_file(abs_path) {
                Ok(h) => h.to_bytes().to_vec(),
                Err(e) => {
                    let err_msg = format!("xxh3_128 error: {e}");
                    if let Some(r) = reporter {
                        r.item_finished(abs_path, Some(&err_msg), size);
                    }
                    return HashResult {
                        path: abs_path.clone(),
                        src_path: entry.src_path.clone(),
                        size,
                        mtime_ms: entry.file.mtime_ms,
                        crc32c: entry.crc32c,
                        raw_unique_id: entry.raw_unique_id.clone(),
                        hash: FileHash::Fast(vec![]), // Empty hash indicates error
                        is_duplicate: false,
                        dup_reason: Some(err_msg),
                    };
                }
            };

            // Optionally compute SHA-256 for definitive identity
            let (hash, err) = if compute_sha256 {
                match sha256_file(abs_path) {
                    Ok(h) => (FileHash::Full(xxh3_128, h.to_bytes().to_vec()), None),
                    Err(e) => {
                        let err_msg = format!("sha256 error: {e}");
                        (FileHash::Fast(xxh3_128), Some(err_msg))
                    }
                }
            } else {
                (FileHash::Fast(xxh3_128), None)
            };

            // Signal end of hashing this file
            if let Some(r) = reporter {
                r.item_finished(abs_path, err.as_deref(), size);
            }
            
            if let Some(err_msg) = err {
                return HashResult {
                    path: abs_path.clone(),
                    src_path: entry.src_path.clone(),
                    size,
                    mtime_ms: entry.file.mtime_ms,
                    crc32c: entry.crc32c,
                    raw_unique_id: entry.raw_unique_id.clone(),
                    hash,
                    is_duplicate: false,
                    dup_reason: Some(err_msg),
                };
            }

            HashResult {
                path: abs_path.clone(),
                src_path: entry.src_path,
                size,
                mtime_ms: entry.file.mtime_ms,
                crc32c: entry.crc32c,
                raw_unique_id: entry.raw_unique_id,
                hash,
                is_duplicate: false,
                dup_reason: None,
            }
        })
        .collect()
}

/// Check for duplicates using DB lookup + batch dedup.
///
/// Uses SHA-256 for identity if available (definitive), otherwise XXH3-128.
/// This is a sequential pass that:
/// 1. Checks hash in DB
/// 2. Checks against already-seen hashes in current batch
///
/// # Arguments
/// * `results` - Hash results from parallel computation
/// * `db` - Database handle
/// * `vault_root` - Vault root path
/// * `allow_same_path` - If true, allow re-adding same path (for add command)
pub fn check_duplicates(
    mut results: Vec<HashResult>,
    db: &Db,
    vault_root: &Path,
    allow_same_path: bool,
) -> anyhow::Result<Vec<HashResult>> {
    let seen: DashMap<Vec<u8>, std::path::PathBuf> = DashMap::new();

    for r in &mut results {
        if r.dup_reason.is_some() {
            continue;
        }

        // Get identity hash and algorithm
        let (hash_bytes, hash_algo) = r.hash.identity();
        let algo_name = if r.hash.is_full() {
            "sha256"
        } else {
            "xxh3_128"
        };

        // Check hash duplicate in DB
        let existing = db.lookup_by_hash(hash_bytes, &hash_algo)?;

        if let Some(ref row) = existing {
            let vault_path = vault_root.join(&row.path);

            // For add command: allow re-adding same path
            let is_same_file = allow_same_path && vault_path == r.path;

            // For recover: allow re-importing if the existing file is 'missing'
            // This handles the case where:
            // - DB has a 'missing' record (file was previously imported then marked missing)
            // - File is being re-imported (vault file may or may not exist yet)
            let is_missing = row.status == "missing";

            // If the DB record is 'missing', this is a recover scenario - allow it
            if is_missing {
                // Allow recovery to proceed - the file in vault may have just been
                // copied by the current import operation
                continue;
            }

            if !is_same_file && vault_path.exists() {
                r.is_duplicate = true;
                r.dup_reason = Some(format!("db ({algo_name})"));
                continue;
            }
        }

        // Check batch duplicate
        match seen.entry(hash_bytes.to_vec()) {
            Entry::Vacant(v) => {
                v.insert(r.path.clone());
            }
            Entry::Occupied(_) => {
                r.is_duplicate = true;
                r.dup_reason = Some(format!("batch ({algo_name})"));
            }
        }
    }

    Ok(results)
}

/// Get the identity hash bytes for a HashResult.
/// Returns SHA-256 if available (definitive), otherwise XXH3-128.
pub fn get_identity_hash(result: &HashResult) -> &[u8] {
    result.hash.identity().0
}

/// Check if the hash result has a full identity (SHA-256).
pub fn has_definitive_identity(result: &HashResult) -> bool {
    result.hash.is_full()
}
