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
//! | [`recheck`] | Re-verify an import against its manifest |

pub mod add;
pub mod clone;
mod exif;
pub mod import;
mod path;
pub mod recheck;
pub mod sync;
pub mod types;
pub mod update;
pub mod utils;

pub use recheck::{RecheckOptions, RecheckResult, RecheckStatus, run_recheck};
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
/// * `entry`      – `CrcEntry` with CRC32C and file metadata
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
    entry: &pipeline::types::CrcEntry,
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

    let cached = match db.lookup_by_crc32c(
        entry.file.size as i64,
        entry.crc32c,
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
