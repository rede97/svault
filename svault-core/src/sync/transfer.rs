//! File transfer for sync/clone operations.
//!
//! Provides two transfer targets: vault (with DB insert) and plain directory.
//! Wraps `fs::transfer_file_with_reporter` with a `SyncCopyAdapter` that
//! bridges `SyncTransferReporter` → `CopyReporter`.

use std::path::Path;

use crate::db::{Db, FileRow};
use crate::fs::{self, TransferStrategy};
use crate::import::utils::unix_now_ms;
use crate::reporting::{CopyItemResult, CopyReporter, SyncTransferReporter};

/// Summary after a sync transfer phase completes.
#[derive(Debug, Clone, Default)]
pub struct SyncTransferSummary {
    pub transferred: usize,
    pub skipped: usize,
    pub failed: usize,
    pub total_bytes: u64,
}

// ── CopyReporter adapter ──────────────────────────────────────────────────────

/// Adapts a `SyncTransferReporter` into a `CopyReporter` so it can be passed
/// to `fs::transfer_file_with_reporter`.
///
/// `item_started` is a no-op because the caller already fires the sync-level
/// `item_started` before the adapter is created per file.
struct SyncCopyAdapter<'a, TR: SyncTransferReporter> {
    inner: &'a TR,
    rel_path: &'a Path,
}

impl<TR: SyncTransferReporter> CopyReporter for SyncCopyAdapter<'_, TR> {
    fn item_started(&self, _src_abs: &Path, _dest_abs: &Path, _bytes_total: u64) {}

    fn item_progress(&self, _src_abs: &Path, bytes_copied: u64, bytes_total: u64) {
        self.inner.item_progress(self.rel_path, bytes_copied, bytes_total);
    }

    fn item_finished(&self, _src_abs: &Path, _dest_abs: &Path, result: &CopyItemResult) {
        self.inner.item_finished(self.rel_path, result);
    }

    fn finish(&self) {}
}

// ── Vault-to-vault transfer ──────────────────────────────────────────────────

/// Transfer files from a source vault to a target vault, preserving relative
/// paths and inserting records into the target DB.
///
/// Skips files that already exist on disk (handles partial-sync recovery).
/// `finish()` is not called — the caller calls it after receiving the summary.
pub fn transfer_to_vault<TR: SyncTransferReporter>(
    files: &[FileRow],
    source_root: &Path,
    target_root: &Path,
    target_db: &Db,
    strategies: &[TransferStrategy],
    reporter: &TR,
) -> anyhow::Result<SyncTransferSummary> {
    let total_files = files.len() as u64;
    let total_bytes: u64 = files.iter().map(|f| f.size as u64).sum();
    reporter.started(total_files, total_bytes);

    let mut transferred = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut bytes_transferred = 0u64;
    let now_ms = unix_now_ms();

    let strategies_fallback: Vec<TransferStrategy> = if strategies.is_empty() {
        vec![TransferStrategy::StreamCopy]
    } else {
        strategies.to_vec()
    };

    for file in files {
        let rel_path = Path::new(&file.path);
        let dst_full = target_root.join(rel_path);

        reporter.item_started(rel_path, file.size as u64);

        if dst_full.exists() {
            reporter.item_finished(rel_path, &CopyItemResult::Ok);
            skipped += 1;
            continue;
        }

        if let Some(parent) = dst_full.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                reporter.item_finished(
                    rel_path,
                    &CopyItemResult::Failed {
                        message: format!("cannot create parent directory: {e}"),
                    },
                );
                failed += 1;
                continue;
            }
        }

        let adapter = SyncCopyAdapter { inner: reporter, rel_path };

        match fs::transfer_file_with_reporter(
            source_root, rel_path,
            target_root, rel_path,
            &strategies_fallback,
            Some(&adapter),
        ) {
            Ok(()) => {
                if let Err(e) = target_db.insert_file_row(
                    &file.path,
                    file.size,
                    file.mtime,
                    file.crc32c.map(|v| v as u32),
                    file.raw_unique_id.as_deref(),
                    file.xxh3_128.as_deref(),
                    file.sha256.as_deref(),
                    "imported",
                    now_ms,
                ) {
                    reporter.item_finished(
                        rel_path,
                        &CopyItemResult::Failed {
                            message: format!("DB insert failed: {e}"),
                        },
                    );
                    failed += 1;
                    continue;
                }

                reporter.item_finished(rel_path, &CopyItemResult::Ok);
                transferred += 1;
                bytes_transferred += file.size as u64;
            }
            Err(e) => {
                reporter.item_finished(
                    rel_path,
                    &CopyItemResult::Failed {
                        message: e.to_string(),
                    },
                );
                failed += 1;
            }
        }
    }

    reporter.summary(transferred, skipped, failed, bytes_transferred);

    Ok(SyncTransferSummary { transferred, skipped, failed, total_bytes: bytes_transferred })
}

// ── Vault-to-directory transfer ──────────────────────────────────────────────

/// Transfer files from a source vault to a plain directory (no target DB).
///
/// Used by the `clone` command.  Same transfer logic as `transfer_to_vault`
/// but does not insert into any database.
pub fn transfer_to_dir<TR: SyncTransferReporter>(
    files: &[FileRow],
    source_root: &Path,
    target_dir: &Path,
    strategies: &[TransferStrategy],
    reporter: &TR,
) -> anyhow::Result<SyncTransferSummary> {
    let total_files = files.len() as u64;
    let total_bytes: u64 = files.iter().map(|f| f.size as u64).sum();
    reporter.started(total_files, total_bytes);

    let mut transferred = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut bytes_transferred = 0u64;

    let strategies_fallback: Vec<TransferStrategy> = if strategies.is_empty() {
        vec![TransferStrategy::StreamCopy]
    } else {
        strategies.to_vec()
    };

    for file in files {
        let rel_path = Path::new(&file.path);
        let dst_full = target_dir.join(rel_path);

        reporter.item_started(rel_path, file.size as u64);

        if dst_full.exists() {
            reporter.item_finished(rel_path, &CopyItemResult::Ok);
            skipped += 1;
            continue;
        }

        if let Some(parent) = dst_full.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                reporter.item_finished(
                    rel_path,
                    &CopyItemResult::Failed {
                        message: format!("cannot create parent directory: {e}"),
                    },
                );
                failed += 1;
                continue;
            }
        }

        let adapter = SyncCopyAdapter { inner: reporter, rel_path };

        match fs::transfer_file_with_reporter(
            source_root, rel_path,
            target_dir, rel_path,
            &strategies_fallback,
            Some(&adapter),
        ) {
            Ok(()) => {
                reporter.item_finished(rel_path, &CopyItemResult::Ok);
                transferred += 1;
                bytes_transferred += file.size as u64;
            }
            Err(e) => {
                reporter.item_finished(
                    rel_path,
                    &CopyItemResult::Failed {
                        message: e.to_string(),
                    },
                );
                failed += 1;
            }
        }
    }

    reporter.summary(transferred, skipped, failed, bytes_transferred);

    Ok(SyncTransferSummary { transferred, skipped, failed, total_bytes: bytes_transferred })
}
