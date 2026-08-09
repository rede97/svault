//! Use-case orchestrators (application layer, L1).
//!
//! Each sub-module orchestrates one user-facing operation by composing the
//! shared `pipeline` stages. Orchestrators never touch the terminal — they
//! emit [`crate::event::Event`]s through an [`crate::event::EventSink`] and
//! ask for confirmation via an [`crate::event::Interactor`].
//!
//! | Module      | Operation                        |
//! |-------------|----------------------------------|
//! | [`import`]  | Import files from outside the vault |
//! | [`add`]     | Register files already in the vault |
//! | [`update`]  | Fix DB paths of moved/renamed files |
//! | [`album`]   | Hierarchical albums + per-membership ratings (Pull model) |

pub mod add;
pub mod album;
pub mod clone;
mod exif;
pub mod import;
mod path;
pub mod sync;
pub mod types;
pub mod update;
pub mod utils;

pub use types::{ImportOptions, ImportSummary};

use std::path::Path;

use crate::config::HashAlgorithm;
use crate::db::Db;
use crate::pipeline;

/// Check if a file is a duplicate via DB lookup.
///
/// Uses the shared [`pipeline::CheckResult`] type for consistent handling in
/// the import and add orchestrators.
///
/// # Arguments
/// * `entry`      – `FingerprintEntry` with CRC32C and file metadata
/// * `db`         – Database handle
/// * `vault_root` – Vault root path for existence checks
/// * `hash`       – Optional `(hash_bytes, algorithm)` for secondary
///   verification when CRC matches
///
/// # Special cases
/// - Status `'missing'` → returns `Recover` (allows re-import with path update)
/// - File exists at original path → returns `Duplicate`
/// - CRC matches but file missing → returns `Moved` (vault-internal move)
pub fn check_duplicate(
    entry: &pipeline::types::FingerprintEntry,
    db: &Db,
    vault_root: &Path,
    hash: Option<(&[u8], &HashAlgorithm)>,
) -> pipeline::CheckResult {
    let ext = entry
        .file
        .path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let cached = match db.lookup_by_fingerprint(
        entry.file.size as i64,
        entry.fingerprint.as_slice(),
        ext,
        entry.raw_unique_id.as_deref(),
    ) {
        Ok(c) => c,
        Err(_) => return pipeline::CheckResult::New,
    };

    if let Some(row) = cached {
        let is_same_raw_id = match (&entry.raw_unique_id, &row.raw_unique_id) {
            (Some(new_id), Some(existing_id)) => new_id == existing_id,
            _ => true,
        };

        let hash_matches = if let Some((hash_bytes, algo)) = hash {
            let db_hash = match algo {
                HashAlgorithm::Xxh3_128 => row.xxh3_128.as_ref(),
                HashAlgorithm::Sha256 => row.sha256.as_ref(),
            };
            db_hash.map(|db| db == hash_bytes).unwrap_or(false)
        } else {
            true
        };

        if row.status == "missing" && hash_matches {
            return pipeline::CheckResult::Recover {
                old_path: row.path,
                file_id: row.id,
            };
        }

        let vault_path = vault_root.join(&row.path);
        if vault_path.exists() && is_same_raw_id && hash_matches {
            return pipeline::CheckResult::Duplicate;
        } else if is_same_raw_id && hash_matches {
            return pipeline::CheckResult::Moved { old_path: row.path };
        }
    }

    pipeline::CheckResult::New
}

/// Fingerprint-suspected duplicates re-verified per [`types::CompareLevel`].
///
/// `Fast` returns the CRC-only verdict unchanged. `Mid`/`High` re-hash the
/// **source** file when the fingerprint says `Duplicate` and compare it
/// against the DB record (`High` uses SHA-256 when the record has one,
/// otherwise XXH3-128). A mismatch flips the verdict to `New` — the file is
/// then copied and, if the destination name is taken, renamed by the usual
/// unique-destination rule. A source-side hash error also flips to `New`,
/// letting the copy stage surface the IO error per-file (G3).
pub fn check_duplicate_with_level(
    entry: &pipeline::types::FingerprintEntry,
    db: &Db,
    vault_root: &Path,
    level: types::CompareLevel,
) -> pipeline::CheckResult {
    let result = check_duplicate(entry, db, vault_root, None);
    if level == types::CompareLevel::Fast || !matches!(result, pipeline::CheckResult::Duplicate) {
        return result;
    }

    // Fingerprint hit — pick the strongest hash the DB record actually has.
    let ext = entry
        .file
        .path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let row = db
        .lookup_by_fingerprint(
            entry.file.size as i64,
            entry.fingerprint.as_slice(),
            ext,
            entry.raw_unique_id.as_deref(),
        )
        .ok()
        .flatten();
    let Some(row) = row else {
        return result;
    };

    let algo = match level {
        types::CompareLevel::High if row.sha256.is_some() => HashAlgorithm::Sha256,
        _ => HashAlgorithm::Xxh3_128,
    };
    let hash_bytes = match algo {
        HashAlgorithm::Sha256 => {
            crate::hash::sha256_file(&entry.file.path).map(|h| h.to_bytes().to_vec())
        }
        HashAlgorithm::Xxh3_128 => {
            crate::hash::xxh3_128_file(&entry.file.path).map(|h| h.to_bytes().to_vec())
        }
    };
    match hash_bytes {
        Ok(bytes) => check_duplicate(entry, db, vault_root, Some((&bytes, &algo))),
        // Unverifiable suspicion: flip to New so the copy stage surfaces
        // the IO error instead of silently trusting the fingerprint.
        Err(_) => pipeline::CheckResult::New,
    }
}
