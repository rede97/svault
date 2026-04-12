//! Reporting abstraction layer for svault-core.
//!
//! Core calls methods on typed phase reporters rather than emitting a generic
//! event enum.  CLI / GUI layers implement the traits to adapt those calls to
//! concrete rendering strategies (terminal progress bars, JSON stream,
//! pipeable text, …).
//!
//! # Architecture
//!
//! ```text
//! ReporterBuilder
//!   ├─ scan_reporter()         → ScanReporter          (walk + CRC + lookup + preflight)
//!   ├─ copy_reporter()         → CopyReporter          (file transfer)
//!   ├─ hash_reporter()         → HashReporter          (XXH3 / SHA-256)
//!   ├─ insert_reporter()       → InsertReporter        (DB insert + final summary)
//!   ├─ add_summary_reporter()  → AddSummaryReporter    (add command summary)
//!   ├─ recheck_reporter()      → RecheckReporter       (manifest integrity check)
//!   ├─ update_hash_reporter()  → UpdateHashReporter    (update: hash-to-match phase)
//!   └─ update_apply_reporter() → UpdateApplyReporter   (update: path-apply phase)
//! ```
//!
//! Each reporter is obtained from the builder, used for exactly one phase,
//! then dropped.  `Drop` implementations guarantee that any progress
//! indicator is cleared even on early exit or panic.

use std::path::{Path, PathBuf};

// ─────────────────────────────────────────────────────────────────────────────
// Shared enums
// ─────────────────────────────────────────────────────────────────────────────

/// Classification status of an item after scanning / deduplication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    New,
    Duplicate,
    Recover,
    MovedInVault,
    Failed,
}

// ─────────────────────────────────────────────────────────────────────────────
// Phase reporter traits
// ─────────────────────────────────────────────────────────────────────────────

/// Reporter for the scan phase (Stages A + B + C: walk + CRC + DB lookup).
pub trait ScanReporter: Send + Sync {
    /// A file has been scanned and classified.
    /// Called once per file with all relevant information.
    fn item(
        &self,
        path: &Path,
        size: u64,
        mtime_ms: i64,
        status: ItemStatus,
        error: Option<&str>,
    );

    /// Pre-flight summary emitted after scan and before the user is asked
    /// to confirm. Core provides raw counts; formatting is up to the
    /// implementation.
    /// 
    /// If `new_count == 0`, the implementation should report that nothing
    /// needs to be imported (all files were duplicates, moved, or failed).
    fn preflight(
        &self,
        total_scanned: usize,
        new_count: usize,
        duplicate_count: usize,
        moved_count: usize,
        failed_count: usize,
        source: &Path,
    );

    /// The scan phase is complete. Implementations should print any
    /// completion summary and clear progress indicators.
    fn finish(&self);
}

/// Result of a single file copy operation.
#[derive(Debug, Clone)]
pub enum CopyItemResult {
    /// File was successfully copied.
    Ok,
    /// File copy failed with an error message.
    Failed { message: String },
}

/// Reporter for the copy phase (Stage C: file transfer).
pub trait CopyReporter: Send + Sync {
    /// A file is about to be transferred.
    /// `src_abs` is the absolute source path.
    /// `dest_abs` is the absolute destination path.
    /// `bytes_total` is the file size in bytes.
    fn item_started(&self, src_abs: &Path, dest_abs: &Path, bytes_total: u64);

    /// Progress update for a file being transferred.
    /// Called periodically during the copy operation.
    /// `src_abs` is the absolute source path.
    /// `bytes_copied` is the number of bytes copied so far.
    /// `bytes_total` is the total file size in bytes.
    fn item_progress(&self, src_abs: &Path, bytes_copied: u64, bytes_total: u64);

    /// A file has finished transferring (success or failure).
    fn item_finished(&self, src_abs: &Path, dest_abs: &Path, result: &CopyItemResult);

    /// The copy phase is complete.
    fn finish(&self);
}

/// Reporter for the hash phase (Stage D: XXH3-128 / SHA-256 computation).
/// Also used for the `update` command's hash-and-match phase.
pub trait HashReporter: Send + Sync {
    /// A file has started hashing.
    /// `abs_path` is the absolute path of the file.
    /// `bytes_total` is the file size in bytes.
    fn item_started(&self, abs_path: &Path, bytes_total: u64);

    /// A file has finished hashing.
    /// `abs_path` is the absolute path of the file.
    /// `error` is None if successful, or contains an error message.
    /// `bytes_total` is the file size in bytes (for speed calculation).
    fn item_finished(&self, abs_path: &Path, error: Option<&str>, bytes_total: u64);

    /// A relocate match was found (for `update` command).
    /// Default implementation does nothing.
    fn matched(&self, _old_path: &str, _new_path: &str, _confidence: MatchConfidence) {}

    fn finish(&self);
}

/// Reporter for the DB insert phase (Stage E).
///
/// Also carries the final import summary via [`InsertReporter::summary`],
/// since insert is the last pipeline stage.
pub trait InsertReporter: Send + Sync {
    fn progress(&self, completed: u64, total: u64);

    /// The insert phase is complete (clears progress indicator).
    fn finish(&self);

    /// Emit the final import summary after all pipeline stages complete.
    ///
    /// Called after [`finish`](InsertReporter::finish).
    fn summary(
        &self,
        total: usize,
        imported: usize,
        duplicate: usize,
        failed: usize,
        manifest_path: Option<&Path>,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Builder
// ─────────────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────────────
// add command reporter
// ─────────────────────────────────────────────────────────────────────────────

/// Reporter for the `add` command's summary phase.
///
/// The scan and hash phases reuse [`ScanReporter`] and [`HashReporter`].
/// This trait handles the add-specific pre-flight summary and the
/// "vault-internal move detected" hints.
///
/// `moved_files` slices contain `(current_vault_path, old_recorded_path)` pairs.
pub trait AddSummaryReporter: Send + Sync {
    /// Pre-flight counts before inserting (no confirmation needed for `add`).
    fn preflight(&self, new_count: usize, duplicate_count: usize, moved_count: usize);

    /// All scanned files were vault-internal moves; suggest `svault update`.
    /// Called instead of `summary` when `new_count == 0 && moved_count > 0`.
    fn only_moved(&self, moved_files: &[(PathBuf, String)], vault_root: &Path);

    /// Final summary after the insert stage completes.
    fn summary(&self, total: usize, added: usize, duplicate: usize, failed: usize);

    /// Post-insert hint shown when some files were also detected as moved
    /// alongside new files.
    fn moved_hint(&self, moved_files: &[(PathBuf, String)], vault_root: &Path);

    /// The add summary phase is complete.
    fn finish(&self);
}

// ─────────────────────────────────────────────────────────────────────────────
// recheck command reporter
// ─────────────────────────────────────────────────────────────────────────────

/// Reporter for the `recheck` command (manifest integrity verification).
pub trait RecheckReporter: Send + Sync {
    /// Called once at the start with the total number of file pairs to check.
    fn started(&self, total: usize, session_id: &str, source: &Path);

    /// A recheck item has started.
    fn item_started(&self, src_path: &Path, vault_path: &Path);

    /// A recheck item has finished.
    fn item_finished(&self, src_path: &Path, vault_path: &Path, status: &crate::import::RecheckStatus);

    /// The check phase is complete (clears progress indicator).
    fn finish(&self);

    /// Final summary with per-status counts and the path of the written report.
    #[allow(clippy::too_many_arguments)]
    fn summary(
        &self,
        ok: usize,
        source_modified: usize,
        vault_corrupted: usize,
        both_diverged: usize,
        source_deleted: usize,
        vault_deleted: usize,
        errors: usize,
        sha256_verified: usize,
        report_path: &Path,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// verify command reporters
// ─────────────────────────────────────────────────────────────────────────────

/// Reporter for the `verify` command (vault integrity check).
pub trait VerifyReporter: Send + Sync {
    /// Called once at the start with the total number of files to verify.
    fn started(&self, total: u64);

    /// A file verification has started.
    fn item_started(&self, path: &Path);

    /// A file verification has finished.
    fn item_finished(&self, path: &Path, result: &crate::verify::VerifyResult);

    /// The verification phase is complete.
    fn finish(&self);

    /// Final summary with per-status counts.
    fn summary(&self, summary: &crate::verify::VerifySummary);
}



// ─────────────────────────────────────────────────────────────────────────────
// update command reporters
// ─────────────────────────────────────────────────────────────────────────────

/// Confidence level of a file-path match found by `svault update`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchConfidence {
    /// Matched by SHA-256 — cryptographically definitive.
    Definitive,
    /// Matched by XXH3-128 only — fast but theoretically collidable.
    Fast,
}

/// Reporter for the `update` command's path-apply phase.
///
/// Applies the matched path corrections to the database (and optionally
/// marks unmatched records as missing / deleted).
pub trait UpdateApplyReporter: Send + Sync {
    fn progress(&self, completed: u64, total: u64);

    /// A DB update failed for `path`.
    fn error(&self, message: &str, path: &str);

    /// The apply phase is complete.
    fn finish(&self);

    /// Final summary of the update operation.
    fn summary(
        &self,
        scanned: usize,
        missing: usize,
        matched: usize,
        unmatched: usize,
        updated: usize,
    );

    /// Called when there are no missing files to update.
    fn nothing_to_update(&self);

    /// Called in dry-run mode to preview files that would be marked as missing.
    fn dry_run_missing(&self, count: usize);
}

// ─────────────────────────────────────────────────────────────────────────────
// History reporters
// ─────────────────────────────────────────────────────────────────────────────

/// Query parameters for history sessions query.
#[derive(Debug, Clone, Default)]
pub struct HistorySessionsQuery {
    pub limit: usize,
    pub offset: usize,
    pub source: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
}

/// Query parameters for history items query.
#[derive(Debug, Clone, Default)]
pub struct HistoryItemsQuery {
    pub limit: usize,
    pub offset: usize,
    pub status: Option<String>,
}

/// Row data for a history session.
#[derive(Debug, Clone)]
pub struct HistorySessionRow {
    pub session_id: String,
    pub session_type: String,
    pub source: String,
    pub started_at_ms: i64,
    pub total_files: usize,
    pub added: usize,
    pub duplicate: usize,
    pub failed: usize,
    pub skipped: usize,
}

/// Row data for a history item.
#[derive(Debug, Clone)]
pub struct HistoryItemRow {
    /// Source file path
    pub source_path: String,
    /// Destination path in vault
    pub vault_path: String,
    /// Item status (added, duplicate, failed, etc.)
    pub status: String,
    /// File size in bytes
    pub size: u64,
    /// Modification time (Unix timestamp ms)
    pub mtime_ms: i64,
}

/// Summary for history sessions query.
#[derive(Debug, Clone)]
pub struct HistorySessionsSummary {
    pub total: usize,
    pub returned: usize,
    pub has_more: bool,
}

/// Summary for history items query.
#[derive(Debug, Clone)]
pub struct HistoryItemsSummary {
    pub total: usize,
    pub returned: usize,
    pub has_more: bool,
}

/// Reporter for history sessions query.
pub trait HistorySessionsReporter: Send + Sync {
    /// Query has started.
    fn started(&self, query: &HistorySessionsQuery);
    /// A session row is reported.
    fn item(&self, row: &HistorySessionRow);
    /// Query is complete.
    fn finish(&self, summary: &HistorySessionsSummary);
}

/// Reporter for history items query.
pub trait HistoryItemsReporter: Send + Sync {
    /// Query has started.
    fn started(&self, session_id: &str, query: &HistoryItemsQuery);
    /// An item row is reported.
    fn item(&self, row: &HistoryItemRow);
    /// Query is complete.
    fn finish(&self, summary: &HistoryItemsSummary);
}

// ─────────────────────────────────────────────────────────────────────────────
// Builder
// ─────────────────────────────────────────────────────────────────────────────

/// Creates typed phase reporters.
///
/// Each `*_reporter` method returns an owned value whose `Drop`
/// implementation guarantees any progress indicator is cleared when
/// the phase ends.
pub trait ReporterBuilder: Send + Sync {
    // ── import pipeline ───────────────────────────────────────────────────
    type Scan: ScanReporter;
    type Copy: CopyReporter;
    type Hash: HashReporter;
    type Insert: InsertReporter;

    fn scan_reporter(&self, source: &Path) -> Self::Scan;
    fn copy_reporter(&self, source: &Path, vault_root: &Path, total: u64) -> Self::Copy;
    fn hash_reporter(&self, source: &Path, total: u64) -> Self::Hash;
    fn insert_reporter(&self, source: &Path, total: u64) -> Self::Insert;

    // ── add command ───────────────────────────────────────────────────────
    type AddSummary: AddSummaryReporter;

    fn add_summary_reporter(&self, vault_root: &Path) -> Self::AddSummary;

    // ── recheck command ───────────────────────────────────────────────────
    type Recheck: RecheckReporter;

    fn recheck_reporter(&self, total: u64) -> Self::Recheck;

    // ── update command ────────────────────────────────────────────────────
    type UpdateApply: UpdateApplyReporter;

    fn update_hash_reporter(&self, source: &Path, total: u64) -> Self::Hash;
    fn update_apply_reporter(&self, total: u64) -> Self::UpdateApply;

    // ── verify command ────────────────────────────────────────────────────
    type Verify: VerifyReporter;

    fn verify_reporter(&self, total: u64) -> Self::Verify;

    // ── history command ───────────────────────────────────────────────────
    type HistorySessions: HistorySessionsReporter;
    type HistoryItems: HistoryItemsReporter;

    fn history_sessions_reporter(&self, query: &HistorySessionsQuery) -> Self::HistorySessions;
    fn history_items_reporter(&self, session_id: &str, query: &HistoryItemsQuery) -> Self::HistoryItems;
}

// ─────────────────────────────────────────────────────────────────────────────
// Noop implementations
// ─────────────────────────────────────────────────────────────────────────────

/// No-op reporter — silently discards all events.
///
/// Used by [`NoopReporterBuilder`] and by builders that only implement a
/// subset of phases (e.g. `PipeReporterBuilder` uses `Noop` for the
/// Copy / Hash / Insert reporters it does not need).
#[derive(Debug, Clone, Copy, Default)]
pub struct Noop;

impl ScanReporter for Noop {
    fn item(&self, _: &Path, _: u64, _: i64, _: ItemStatus, _: Option<&str>) {}
    fn preflight(&self, _: usize, _: usize, _: usize, _: usize, _: usize, _: &Path) {}
    fn finish(&self) {}
}

impl CopyReporter for Noop {
    fn item_started(&self, _: &Path, _: &Path, _: u64) {}
    fn item_progress(&self, _: &Path, _: u64, _: u64) {}
    fn item_finished(&self, _: &Path, _: &Path, _: &CopyItemResult) {}
    fn finish(&self) {}
}

impl HashReporter for Noop {
    fn item_started(&self, _: &Path, _: u64) {}
    fn item_finished(&self, _: &Path, _: Option<&str>, _: u64) {}
    fn finish(&self) {}
}

impl InsertReporter for Noop {
    fn progress(&self, _: u64, _: u64) {}
    fn finish(&self) {}
    fn summary(&self, _: usize, _: usize, _: usize, _: usize, _: Option<&Path>) {}
}

impl AddSummaryReporter for Noop {
    fn preflight(&self, _: usize, _: usize, _: usize) {}
    fn only_moved(&self, _: &[(PathBuf, String)], _: &Path) {}
    fn summary(&self, _: usize, _: usize, _: usize, _: usize) {}
    fn moved_hint(&self, _: &[(PathBuf, String)], _: &Path) {}
    fn finish(&self) {}
}

impl RecheckReporter for Noop {
    fn started(&self, _: usize, _: &str, _: &Path) {}
    fn item_started(&self, _: &Path, _: &Path) {}
    fn item_finished(&self, _: &Path, _: &Path, _: &crate::import::RecheckStatus) {}
    fn finish(&self) {}
    fn summary(
        &self,
        _: usize,
        _: usize,
        _: usize,
        _: usize,
        _: usize,
        _: usize,
        _: usize,
        _: usize,
        _: &Path,
    ) {
    }
}

impl UpdateApplyReporter for Noop {
    fn progress(&self, _: u64, _: u64) {}
    fn error(&self, _: &str, _: &str) {}
    fn finish(&self) {}
    fn summary(&self, _: usize, _: usize, _: usize, _: usize, _: usize) {}
    fn nothing_to_update(&self) {}
    fn dry_run_missing(&self, _: usize) {}
}

impl VerifyReporter for Noop {
    fn started(&self, _: u64) {}
    fn item_started(&self, _: &Path) {}
    fn item_finished(&self, _: &Path, _: &crate::verify::VerifyResult) {}
    fn finish(&self) {}
    fn summary(&self, _: &crate::verify::VerifySummary) {}
}

impl HistorySessionsReporter for Noop {
    fn started(&self, _: &HistorySessionsQuery) {}
    fn item(&self, _: &HistorySessionRow) {}
    fn finish(&self, _: &HistorySessionsSummary) {}
}

impl HistoryItemsReporter for Noop {
    fn started(&self, _: &str, _: &HistoryItemsQuery) {}
    fn item(&self, _: &HistoryItemRow) {}
    fn finish(&self, _: &HistoryItemsSummary) {}
}

/// No-op builder — all phases use [`Noop`].
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopReporterBuilder;

impl ReporterBuilder for NoopReporterBuilder {
    type Scan = Noop;
    type Copy = Noop;
    type Hash = Noop;
    type Insert = Noop;
    type AddSummary = Noop;
    type Recheck = Noop;
    type UpdateApply = Noop;
    type Verify = Noop;
    type HistorySessions = Noop;
    type HistoryItems = Noop;

    fn scan_reporter(&self, _: &Path) -> Noop {
        Noop
    }
    fn copy_reporter(&self, _: &Path, _: &Path, _: u64) -> Noop {
        Noop
    }
    fn hash_reporter(&self, _: &Path, _: u64) -> Noop {
        Noop
    }
    fn insert_reporter(&self, _: &Path, _: u64) -> Noop {
        Noop
    }
    fn add_summary_reporter(&self, _: &Path) -> Noop {
        Noop
    }
    fn recheck_reporter(&self, _: u64) -> Noop {
        Noop
    }
    fn update_hash_reporter(&self, _: &Path, _: u64) -> Noop {
        Noop
    }
    fn update_apply_reporter(&self, _: u64) -> Noop {
        Noop
    }
    fn verify_reporter(&self, _: u64) -> Noop {
        Noop
    }
    fn history_sessions_reporter(&self, _: &HistorySessionsQuery) -> Noop {
        Noop
    }
    fn history_items_reporter(&self, _: &str, _: &HistoryItemsQuery) -> Noop {
        Noop
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Interactor
// ─────────────────────────────────────────────────────────────────────────────

/// Trait for interactive user prompts (confirmation dialogs, etc.).
pub trait Interactor: Send + Sync {
    fn confirm(&self, message: &str) -> bool;
}

/// No-op interactor that always confirms without prompting.
#[derive(Debug, Clone, Copy, Default)]
pub struct YesInteractor;

impl Interactor for YesInteractor {
    fn confirm(&self, _message: &str) -> bool {
        true
    }
}
