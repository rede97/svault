//! Background SHA-256 computation for files imported without it.

use std::path::Path;
use std::thread;
use std::time::Duration;

use serde::Serialize;

use crate::db::Db;
use crate::event::{Event, EventSink, Phase, PhaseContext};
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
#[derive(Debug, Clone, Default, Serialize)]
pub struct BackgroundHashSummary {
    pub processed: usize,
    pub failed: usize,
}

/// Compute missing SHA-256 hashes for files in the vault.
pub fn run_background_hash(
    opts: BackgroundHashOptions,
    db: &Db,
    sink: &dyn EventSink,
) -> anyhow::Result<BackgroundHashSummary> {
    let files = db.get_files_pending_sha256(opts.limit)?;
    let total = files.len();

    sink.emit(&Event::PhaseStarted {
        phase: Phase::Hash,
        total: Some(total as u64),
        context: PhaseContext::vault(opts.vault_root.clone()),
    });

    if total == 0 {
        sink.emit(&Event::PhaseFinished { phase: Phase::Hash });
        return Ok(BackgroundHashSummary::default());
    }

    let mut summary = BackgroundHashSummary::default();

    for file in files.iter() {
        let full_path = Path::new(&opts.vault_root).join(&file.path);

        sink.emit(&Event::HashStarted {
            path: full_path.clone(),
            bytes: file.size as u64,
        });

        let error = match sha256_file(&full_path) {
            Ok(digest) => {
                let hash_bytes = digest.to_bytes();
                let result = db.conn_ref().execute(
                    "UPDATE files SET sha256 = ?1 WHERE id = ?2",
                    rusqlite::params![hash_bytes, file.id],
                );
                match result {
                    Ok(_) => {
                        summary.processed += 1;
                        None
                    }
                    Err(e) => {
                        summary.failed += 1;
                        Some(e.to_string())
                    }
                }
            }
            Err(e) => {
                summary.failed += 1;
                Some(e.to_string())
            }
        };

        sink.emit(&Event::HashFinished {
            path: full_path,
            bytes: file.size as u64,
            error,
        });

        if opts.nice {
            thread::sleep(Duration::from_millis(10));
        }
    }

    sink.emit(&Event::PhaseFinished { phase: Phase::Hash });

    Ok(summary)
}
