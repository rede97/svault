//! Stage D: Strong hash verification.

use std::path::Path;

use dashmap::{mapref::entry::Entry, DashMap};
use indicatif::ProgressBar;
use rayon::prelude::*;

use crate::config::HashAlgorithm;
use crate::db::Db;
use crate::hash::{sha256_file, xxh3_128_file};
use crate::pipeline::types::{CrcEntry, HashResult};

/// Compute strong hashes for all entries in parallel.
///
/// # Arguments
/// * `entries` - CRC entries (from lookup stage)
/// * `hash_algo` - Hash algorithm (XXH3-128 or SHA-256)
/// * `progress` - Optional progress bar
///
/// # Returns
/// List of hash results (errors preserved in result with dup_reason)
pub fn compute_hashes(
    entries: Vec<CrcEntry>,
    hash_algo: HashAlgorithm,
    progress: Option<&ProgressBar>,
) -> Vec<HashResult> {
    entries
        .into_par_iter()
        .map(|entry| {
            let abs_path = &entry.file.path;

            let hash_bytes = match hash_algo {
                HashAlgorithm::Xxh3_128 => match xxh3_128_file(abs_path) {
                    Ok(d) => d.to_bytes().to_vec(),
                    Err(e) => {
                        if let Some(pb) = progress {
                            pb.inc(1);
                        }
                        return HashResult {
                            path: abs_path.clone(),
                            size: entry.file.size,
                            mtime_ms: entry.file.mtime_ms,
                            crc32c: entry.crc32c,
                            raw_unique_id: entry.raw_unique_id.clone(),
                            hash_bytes: vec![],
                            is_duplicate: false,
                            dup_reason: Some(format!("hash error: {e}")),
                        };
                    }
                },
                HashAlgorithm::Sha256 => match sha256_file(abs_path) {
                    Ok(d) => d.to_bytes().to_vec(),
                    Err(e) => {
                        if let Some(pb) = progress {
                            pb.inc(1);
                        }
                        return HashResult {
                            path: abs_path.clone(),
                            size: entry.file.size,
                            mtime_ms: entry.file.mtime_ms,
                            crc32c: entry.crc32c,
                            raw_unique_id: entry.raw_unique_id.clone(),
                            hash_bytes: vec![],
                            is_duplicate: false,
                            dup_reason: Some(format!("hash error: {e}")),
                        };
                    }
                },
            };

            if let Some(pb) = progress {
                pb.inc(1);
            }

            HashResult {
                path: abs_path.clone(),
                size: entry.file.size,
                mtime_ms: entry.file.mtime_ms,
                crc32c: entry.crc32c,
                raw_unique_id: entry.raw_unique_id,
                hash_bytes,
                is_duplicate: false,
                dup_reason: None,
            }
        })
        .collect()
}

/// Check for duplicates using DB lookup + batch dedup.
///
/// This is a sequential pass that:
/// 1. Checks hash in DB
/// 2. Checks against already-seen hashes in current batch
///
/// # Arguments
/// * `results` - Hash results from parallel computation
/// * `db` - Database handle
/// * `vault_root` - Vault root path
/// * `hash_algo` - Hash algorithm used
/// * `allow_same_path` - If true, allow re-adding same path (for add command)
pub fn check_duplicates(
    mut results: Vec<HashResult>,
    db: &Db,
    vault_root: &Path,
    hash_algo: &HashAlgorithm,
    allow_same_path: bool,
) -> anyhow::Result<Vec<HashResult>> {
    let seen: DashMap<Vec<u8>, std::path::PathBuf> = DashMap::new();

    for r in &mut results {
        if r.dup_reason.is_some() {
            continue;
        }

        // Check hash duplicate in DB
        let existing = db.lookup_by_hash(&r.hash_bytes, hash_algo).unwrap_or(None);
        if let Some(ref row) = existing {
            let vault_path = vault_root.join(&row.path);
            
            // For add command: allow re-adding same path
            let is_same_file = allow_same_path && vault_path == r.path;
            
            if !is_same_file && vault_path.exists() {
                r.is_duplicate = true;
                r.dup_reason = Some("db".to_string());
                continue;
            }
        }

        // Check batch duplicate
        match seen.entry(r.hash_bytes.clone()) {
            Entry::Vacant(v) => {
                v.insert(r.path.clone());
            }
            Entry::Occupied(_) => {
                r.is_duplicate = true;
                r.dup_reason = Some("batch".to_string());
            }
        }
    }

    Ok(results)
}
