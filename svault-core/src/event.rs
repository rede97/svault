//! Progress events and interaction boundary for svault-core.
//!
//! This module is the **only** channel through which core communicates
//! progress to the outside world (architecture rule R3). Core never touches
//! the terminal directly; it emits [`Event`]s to an [`EventSink`] and asks
//! for confirmation through an [`Interactor`].
//!
//! Two communication models exist in core:
//!
//! - **Push (events)** — for long-running operations (import, add, update,
//!   recheck, verify). The operation emits events; the sink renders them.
//! - **Pull (return data)** — for instant queries (status, db dump). The
//!   function returns a `serde::Serialize` data structure and the caller
//!   formats it. No events involved.
//!
//! Implementations of [`EventSink`] live in the `svault-ui` crate
//! (terminal progress bars, JSON event stream, scan pipe protocol).

use std::path::PathBuf;

use serde::Serialize;

use crate::ops::add::AddSummary;
use crate::ops::recheck::RecheckStatus;
use crate::ops::types::ImportSummary;
use crate::ops::update::UpdateSummary;
use crate::verify::{VerifyResult, VerifySummary};

// ─────────────────────────────────────────────────────────────────────────────
// Supporting enums
// ─────────────────────────────────────────────────────────────────────────────

/// Which pipeline phase an event belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Directory walk + CRC32C + duplicate lookup.
    Scan,
    /// File transfer into the vault.
    Copy,
    /// Strong hash computation (XXH3-128 / SHA-256).
    Hash,
    /// Batch database insert.
    Insert,
    /// Database path updates (`update` command).
    Apply,
    /// Manifest re-verification (`recheck` command).
    Recheck,
    /// Vault integrity verification (`verify` command).
    Verify,
    /// Vault-to-vault comparison (`sync` command).
    Compare,
}

/// Classification of a file after scanning and duplicate lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    /// Will be imported.
    New,
    /// Already in the vault; skipped.
    Duplicate,
    /// Previously marked missing; will be re-imported.
    Recover,
    /// Found elsewhere inside the vault (moved by the user).
    MovedInVault,
    /// Could not be processed (I/O error, CRC failure, …).
    Failed,
}

/// Confidence level of a path match found by `svault update`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchConfidence {
    /// Matched by SHA-256 — cryptographically definitive.
    Definitive,
    /// Matched by XXH3-128 only — fast but theoretically collidable.
    Fast,
}

// ─────────────────────────────────────────────────────────────────────────────
// Summaries
// ─────────────────────────────────────────────────────────────────────────────

/// Final summary of a completed operation (structured, serializable).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Summary {
    /// `svault import` finished.
    Import(ImportSummary),
    /// `svault add` finished.
    Add(AddSummary),
    /// `svault verify` finished.
    Verify(Box<VerifySummary>),
    /// `svault recheck` finished.
    Recheck(RecheckSummary),
    /// `svault update` finished.
    Update(UpdateSummary),
    /// `svault clone` finished.
    Clone(CloneSummary),
    /// `svault sync` finished.
    Sync(SyncSummary),
}

/// Final summary of a `clone` run.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CloneSummary {
    /// Files matching the filters.
    pub total: usize,
    /// Files successfully copied.
    pub copied: usize,
    /// Files that failed to copy.
    pub failed: usize,
    /// Total bytes copied.
    pub bytes: u64,
    /// Path of the written clone manifest.
    pub manifest_path: Option<PathBuf>,
}

/// Final summary of a `sync` run.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncSummary {
    /// Files present in both vaults.
    pub identical: usize,
    /// Files copied from source to dest.
    pub copied: usize,
    /// Files that failed to copy or insert.
    pub failed: usize,
    /// Total bytes copied.
    pub bytes: u64,
    /// Files only in the source vault (not copied: hashless).
    pub skipped: usize,
    /// Files only in the dest vault (reported, never deleted).
    pub only_dest: usize,
    /// Same content at different paths (reported only).
    pub moved: usize,
    /// Same path, different content (kept dest, reported).
    pub conflicts: usize,
    /// Conflict paths (capped by the sink when displayed).
    pub conflict_paths: Vec<String>,
    /// Path of the written sync manifest.
    pub manifest_path: Option<PathBuf>,
}

/// Tally of a `recheck` run, plus the path of the written JSON report.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RecheckSummary {
    pub ok: usize,
    pub source_modified: usize,
    pub vault_corrupted: usize,
    pub both_diverged: usize,
    pub source_deleted: usize,
    pub vault_deleted: usize,
    pub errors: usize,
    pub sha256_verified: usize,
    pub report_path: PathBuf,
}

// ─────────────────────────────────────────────────────────────────────────────
// Hints
// ─────────────────────────────────────────────────────────────────────────────

/// Advisory messages an operation wants surfaced to the user.
///
/// Hints carry raw data; wording is entirely up to the sink.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Hint {
    /// `add` found only vault-internal moves; suggest `svault update`.
    /// Pairs are `(current_vault_path, old_recorded_path)`.
    OnlyMoved {
        moved: Vec<(PathBuf, String)>,
        vault_root: PathBuf,
    },
    /// `add` imported new files but also noticed moved files.
    MovedHint {
        moved: Vec<(PathBuf, String)>,
        vault_root: PathBuf,
    },
    /// `update` found no missing files.
    NothingToUpdate,
    /// `update --dry-run`: this many records would be marked missing.
    DryRunMissing { count: usize },
    /// `import` reconciled staging leftovers from an interrupted session:
    /// `completed` pending renames were finished, `purged` incomplete
    /// residue files (created by svault inside `.svault/staging/`) removed.
    StagingReconciled { completed: usize, purged: usize },
    /// A staged file could not be renamed to its final destination after
    /// the DB commit; the rename is retried at the start of the next import.
    StagedCommitDeferred {
        staged: PathBuf,
        dest: PathBuf,
        error: String,
    },
}

/// Directory context of a running phase, used by sinks to display
/// relative paths. Which fields are set depends on the phase.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PhaseContext {
    /// Source directory being scanned/copied from (import, add, scan).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
    /// Vault root (copy destinations, verify, update, background hash).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_root: Option<PathBuf>,
}

impl PhaseContext {
    /// Context with only a source directory.
    pub fn source(source: PathBuf) -> Self {
        Self {
            source: Some(source),
            vault_root: None,
        }
    }

    /// Context with only a vault root.
    pub fn vault(vault_root: PathBuf) -> Self {
        Self {
            source: None,
            vault_root: Some(vault_root),
        }
    }

    /// Context with both a source directory and a vault root.
    pub fn both(source: PathBuf, vault_root: PathBuf) -> Self {
        Self {
            source: Some(source),
            vault_root: Some(vault_root),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Event
// ─────────────────────────────────────────────────────────────────────────────

/// A single progress event emitted by a core operation.
///
/// Events are emitted on arbitrary Rayon worker threads; sinks must be
/// thread-safe (`Send + Sync`). Each event is self-contained — a sink never
/// needs to call back into core.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// A pipeline phase has started. `context` carries the directories the
    /// phase works on, used by sinks for relative path display.
    PhaseStarted {
        phase: Phase,
        total: Option<u64>,
        context: PhaseContext,
    },
    /// A pipeline phase is complete; progress indicators should be cleared.
    PhaseFinished { phase: Phase },

    /// A file has been scanned and classified.
    ScanItem {
        path: PathBuf,
        size: u64,
        mtime_ms: i64,
        status: ItemStatus,
        error: Option<String>,
    },
    /// Pre-flight counts after scanning, before user confirmation.
    Preflight {
        source: PathBuf,
        total: usize,
        new: usize,
        duplicate: usize,
        moved: usize,
        failed: usize,
    },

    /// A file transfer has started.
    CopyStarted {
        src: PathBuf,
        dst: PathBuf,
        bytes: u64,
    },
    /// Progress within a single file transfer.
    CopyProgress {
        src: PathBuf,
        copied: u64,
        total: u64,
    },
    /// A file transfer has finished (`error = None` on success).
    CopyFinished {
        src: PathBuf,
        dst: PathBuf,
        error: Option<String>,
    },

    /// Hashing of a file has started.
    HashStarted { path: PathBuf, bytes: u64 },
    /// Hashing of a file has finished (`error = None` on success).
    HashFinished {
        path: PathBuf,
        bytes: u64,
        error: Option<String>,
    },
    /// `update` matched a missing DB record to a file on disk.
    RelocateMatched {
        old_path: String,
        new_path: String,
        confidence: MatchConfidence,
    },

    /// Counter-style progress for phases without per-file detail
    /// (insert, apply).
    Progress { phase: Phase, done: u64, total: u64 },
    /// A database update failed during the apply phase.
    ApplyError { path: String, message: String },

    /// `recheck` has started: `total` file pairs from `session_id`.
    RecheckStarted {
        total: usize,
        session_id: String,
        source: PathBuf,
    },
    /// A single source/vault pair has been re-verified.
    RecheckItem {
        src: PathBuf,
        vault: PathBuf,
        status: RecheckStatus,
    },

    /// A single vault file has been verified.
    VerifyItem { path: PathBuf, result: VerifyResult },

    /// `sync` comparison is complete; preflight counts before confirmation.
    SyncPlan {
        source_vault: PathBuf,
        identical: usize,
        to_copy: usize,
        copy_bytes: u64,
        moved: usize,
        only_dest: usize,
        conflicts: usize,
    },

    /// The operation is complete; carries the structured final summary.
    Summary(Summary),
    /// Advisory message for the user (see [`Hint`]).
    Hint(Hint),
}

// ─────────────────────────────────────────────────────────────────────────────
// Sink
// ─────────────────────────────────────────────────────────────────────────────

/// Receives progress events from core operations.
///
/// Implementations must be cheap and non-blocking — they are called from
/// Rayon worker threads in hot loops. Heavy rendering should be throttled
/// by the implementation.
pub trait EventSink: Send + Sync {
    /// Handle one event.
    fn emit(&self, event: &Event);
}

/// A sink that discards all events.
///
/// Used in tests, benchmarks, and non-interactive automation.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopSink;

impl EventSink for NoopSink {
    fn emit(&self, _event: &Event) {}
}

// ─────────────────────────────────────────────────────────────────────────────
// Interactor
// ─────────────────────────────────────────────────────────────────────────────

/// Confirmation boundary between core and the user (architecture rule R4).
///
/// Core calls [`Interactor::confirm`] whenever it needs a y/N decision.
/// The terminal implementation lives in `svault-ui`; automation uses
/// [`YesInteractor`].
pub trait Interactor: Send + Sync {
    /// Ask the user to confirm `message`; returns the user's decision.
    fn confirm(&self, message: &str) -> bool;
}

/// An interactor that always confirms without asking.
///
/// Used for `--yes`, `--output=json`, and tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct YesInteractor;

impl Interactor for YesInteractor {
    fn confirm(&self, _message: &str) -> bool {
        true
    }
}
