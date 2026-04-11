//! Background SHA-256 computation for files imported without it.

use std::path::Path;
use std::thread;
use std::time::Duration;

use crate::db::Db;
use crate::hash::sha256_file;
use crate::reporting::{BackgroundHashReporter, ReporterBuilder};

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

    if total == 0 {
        return Ok(BackgroundHashSummary::default());
    }

    let reporter = reporter_builder.background_hash_reporter(total as u64);
    reporter.started(total as u64);

    let mut summary = BackgroundHashSummary::default();

    for (idx, file) in files.iter().enumerate() {
        let rel_path = Path::new(&file.path)
            .strip_prefix(&opts.vault_root)
            .unwrap_or(Path::new(&file.path));
        let full_path = Path::new(&opts.vault_root).join(&file.path);

        reporter.progress((idx + 1) as u64, total as u64, rel_path);

        match sha256_file(&full_path) {
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
                    reporter.error(rel_path, &e.to_string());
                    summary.failed += 1;
                } else {
                    reporter.hashed(rel_path);
                    summary.processed += 1;
                }
            }
            Err(e) => {
                reporter.error(rel_path, &e.to_string());
                summary.failed += 1;
            }
        }

        if opts.nice {
            thread::sleep(Duration::from_millis(10));
        }
    }

    reporter.finish();
    reporter.summary(summary.processed, summary.failed);

    Ok(summary)
}
