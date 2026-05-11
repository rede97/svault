//! Reporter traits for the `sync` / `clone` commands.

use std::path::Path;

use super::types::CopyItemResult;

// ── Diff phase ───────────────────────────────────────────────────────────────

/// Reporter for the sync diff phase (SHA-256 set comparison).
///
/// Compares the source vault's imported files against the target
/// to determine which files need to be transferred.
pub trait SyncDiffReporter: Send + Sync {
    /// Diff phase started.
    /// `source_count` — imported files in the source vault.
    /// `target_count` — files already in the target vault (0 for clone/export).
    fn started(&self, source_count: usize, target_count: usize);

    /// Diff computed.
    /// `new_count` — files to transfer.
    /// `skip_count` — files already present on target.
    /// `total_bytes` — total bytes to copy.
    fn diff_computed(&self, new_count: usize, skip_count: usize, total_bytes: u64);

    /// Target already has everything — nothing to sync.
    fn nothing_to_sync(&self);

    /// The diff phase is complete.
    fn finish(&self);
}

// ── Transfer phase ───────────────────────────────────────────────────────────

/// Reporter for the sync transfer phase (file copy to target).
///
/// Tracks per-file progress and provides a final summary.
/// The actual file transfer is performed by the same engine that drives
/// [`CopyReporter`](super::CopyReporter); this reporter sits above it at
/// the sync level.
pub trait SyncTransferReporter: Send + Sync {
    /// Transfer phase started.
    fn started(&self, total_files: u64, total_bytes: u64);

    /// A file is about to be transferred.
    fn item_started(&self, path: &Path, bytes: u64);

    /// Progress update for the current file.
    fn item_progress(&self, path: &Path, bytes_copied: u64, bytes_total: u64);

    /// A file transfer completed (success or failure).
    fn item_finished(&self, path: &Path, result: &CopyItemResult);

    /// The transfer phase is complete.
    fn finish(&self);

    /// Final transfer summary.
    fn summary(&self, transferred: usize, skipped: usize, failed: usize, total_bytes: u64);
}
