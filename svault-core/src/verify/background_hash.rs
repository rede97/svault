//! Background SHA-256 computation for files imported without it.

use std::path::Path;
use std::thread;
use std::time::Duration;

use crate::db::Db;
use crate::hash::sha256_file;
use crate::reporting::{HashReporter, ReporterBuilder};

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
pub fn run_background_hash<RB: ReporterBuilder>(
    opts: BackgroundHashOptions,
    db: &Db,
    reporter_builder: &RB,
) -> anyhow::Result<BackgroundHashSummary> {
    let files = db.get_files_pending_sha256(opts.limit)?;
    let total = files.len();

    // Always create reporter, even when no files pending
    // Use a dummy source path since background hash doesn't have a source directory
    let dummy_source = std::path::PathBuf::from(".");
    let reporter = reporter_builder.hash_reporter(&dummy_source, total as u64);

    if total == 0 {
        reporter.finish();
        return Ok(BackgroundHashSummary::default());
    }

    let mut summary = BackgroundHashSummary::default();

    for file in files.iter() {
        let full_path = Path::new(&opts.vault_root).join(&file.path);

        reporter.item_started(&full_path, file.size as u64);

        let error = match sha256_file(&full_path) {
            Ok(digest) => {
                let hash_bytes = digest.to_bytes();
                let payload = serde_json::json!({
                    "path": file.path,
                    "sha256": digest.to_hex(),
                })
                .to_string();

                if let Err(e) =
                    db.append_event("file.sha256_resolved", "file", file.id, &payload, |conn| {
                        conn.execute(
                            "UPDATE files SET sha256 = ?1 WHERE id = ?2",
                            rusqlite::params![hash_bytes, file.id],
                        )?;
                        Ok(())
                    })
                {
                    summary.failed += 1;
                    Some(e.to_string())
                } else {
                    summary.processed += 1;
                    None
                }
            }
            Err(e) => {
                summary.failed += 1;
                Some(e.to_string())
            }
        };

        reporter.item_finished(&full_path, error.as_deref());

        if opts.nice {
            thread::sleep(Duration::from_millis(10));
        }
    }

    reporter.finish();

    Ok(summary)
}
