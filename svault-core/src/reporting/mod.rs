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
///
/// Also handles the pre-flight summary and the "nothing to import" case,
/// since both are emitted at the end of the scan phase before the reporter
/// is dropped.
pub trait ScanReporter: Send + Sync {
    /// A file was discovered during the directory walk.
    /// `abs_path` is the absolute path of the file.
    fn discovered(&self, abs_path: &Path, size: u64, mtime_ms: i64);

    /// A file was classified after a DB duplicate check.
    /// `abs_path` is the absolute path of the file.
    /// `size` is the file size in bytes.
    fn classified(&self, abs_path: &Path, size: u64, status: ItemStatus, detail: Option<&str>);

    /// Periodic progress counter — number of files visited so far.
    fn progress(&self, completed: u64);

    /// A non-fatal warning associated with an optional path.
    fn warning(&self, message: &str, abs_path: Option<&Path>);

    /// A file could not be read or hashed.
    fn error(&self, message: &str, abs_path: Option<&Path>);

    /// Pre-flight summary emitted after scan and before the user is asked
    /// to confirm.  Core provides raw counts; formatting is up to the
    /// implementation.
    fn preflight(
        &self,
        total_scanned: usize,
        new_count: usize,
        duplicate_count: usize,
        moved_count: usize,
        failed_count: usize,
        source: &Path,
    );

    /// All scanned files were already in the vault — nothing to import.
    fn nothing_to_import(&self, total: usize, duplicate: usize);

    /// The scan phase is complete.  Implementations should print any
    /// completion summary and clear progress indicators.
    fn finish(&self);
}

/// Reporter for the copy phase (Stage C: file transfer).
pub trait CopyReporter: Send + Sync {
    /// A file is about to be transferred.
    /// `src_abs` is the absolute source path.
    /// `dest_abs` is the absolute destination path.
    /// `bytes_total` is the file size in bytes.
    fn item_started(&self, src_abs: &Path, dest_abs: &Path, bytes_total: u64);

    /// A file was successfully transferred.
    /// `src_abs` is the absolute source path, `dest_abs` is the absolute destination path.
    /// `bytes_total` is the file size in bytes.
    fn item_finished(&self, src_abs: &Path, dest_abs: &Path, bytes_total: u64);

    /// A file could not be transferred.
    fn error(&self, message: &str, abs_path: Option<&Path>);

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

    /// Parallel-safe progress update.
    fn progress(&self, completed: u64, total: u64);

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

    /// Parallel-safe progress update.
    fn progress(&self, completed: u64, total: u64);

    /// A file was successfully verified.
    fn verified(&self, path: &Path);

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
    fn discovered(&self, _: &Path, _: u64, _: i64) {}
    fn classified(&self, _: &Path, _: u64, _: ItemStatus, _: Option<&str>) {}
    fn progress(&self, _: u64) {}
    fn warning(&self, _: &str, _: Option<&Path>) {}
    fn error(&self, _: &str, _: Option<&Path>) {}
    fn preflight(&self, _: usize, _: usize, _: usize, _: usize, _: usize, _: &Path) {}
    fn nothing_to_import(&self, _: usize, _: usize) {}
    fn finish(&self) {}
}

impl CopyReporter for Noop {
    fn item_started(&self, _: &Path, _: &Path, _: u64) {}
    fn item_finished(&self, _: &Path, _: &Path, _: u64) {}
    fn error(&self, _: &str, _: Option<&Path>) {}
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
    fn progress(&self, _: u64, _: u64) {}
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
    fn progress(&self, _: u64, _: u64) {}
    fn verified(&self, _: &Path) {}
    fn finish(&self) {}
    fn summary(&self, _: &crate::verify::VerifySummary) {}
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
