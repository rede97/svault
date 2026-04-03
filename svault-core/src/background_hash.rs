//! Background SHA-256 computation for files imported without it.

use std::path::Path;
use std::thread;
use std::time::Duration;

use crate::db::Db;
use crate::hash::sha256_file;

/// Options for background hash computation.
pub struct BackgroundHashOptions {
    /// Vault root directory.
    pub vault_root: std::path::PathBuf,
    /// Maximum number of files to process (None = all pending).
    pub limit: Option<usize>,
    /// If true, yield between files to reduce IO impact.
    pub nice: bool,
}

/// Result of a background hash run.
#[derive(Debug, Default)]
pub struct BackgroundHashSummary {
    pub processed: usize,
    pub failed: usize,
}

/// Compute missing SHA-256 hashes for files in the vault.
pub fn run_background_hash(opts: BackgroundHashOptions, db: &Db) -> anyhow::Result<BackgroundHashSummary> {
    let files = db.get_files_pending_sha256(opts.limit)?;
    let total = files.len();

    if total == 0 {
        return Ok(BackgroundHashSummary::default());
    }

    let mut summary = BackgroundHashSummary::default();

    for file in files {
        let full_path = Path::new(&opts.vault_root).join(&file.path);

        match sha256_file(&full_path) {
            Ok(digest) => {
                let hash_bytes = digest.to_bytes();
                let payload = serde_json::json!({
                    "path": file.path,
                    "sha256": digest.to_hex(),
                }).to_string();

                if let Err(e) = db.append_event(
                    "file.sha256_resolved",
                    "file",
                    file.id,
                    &payload,
                    |conn| {
                        conn.execute(
                            "UPDATE files SET sha256 = ?1 WHERE id = ?2",
                            rusqlite::params![hash_bytes, file.id],
                        )?;
                        Ok(())
                    },
                ) {
                    eprintln!("  Failed to update {}: {}", file.path, e);
                    summary.failed += 1;
                } else {
                    summary.processed += 1;
                }
            }
            Err(e) => {
                eprintln!("  Failed to hash {}: {}", full_path.display(), e);
                summary.failed += 1;
            }
        }

        if opts.nice {
            thread::sleep(Duration::from_millis(10));
        }
    }

    Ok(summary)
}
