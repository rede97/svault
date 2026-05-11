//! Reporting abstraction layer for svault-core.
//!
//! Core calls methods on typed phase reporters rather than emitting a generic
//! event enum.  CLI / GUI layers implement the traits to adapt those calls to
//! concrete rendering strategies (terminal progress bars, JSON stream,
//! pipeable text, ...).
//!
//! # Architecture
//!
//! ```text
//! ReporterBuilder
//!   ├─ scan_reporter()           → ScanReporter            (walk + CRC + lookup + preflight)
//!   ├─ copy_reporter()           → CopyReporter            (file transfer)
//!   ├─ hash_reporter()           → HashReporter            (XXH3 / SHA-256)
//!   ├─ insert_reporter()         → InsertReporter          (DB insert + final summary)
//!   ├─ add_summary_reporter()    → AddSummaryReporter      (add command summary)
//!   ├─ recheck_reporter()        → RecheckReporter         (manifest integrity check)
//!   ├─ update_hash_reporter()    → HashReporter            (update: hash-to-match phase)
//!   ├─ update_apply_reporter()   → UpdateApplyReporter     (update: path-apply phase)
//!   ├─ verify_reporter()         → VerifyReporter          (vault integrity verification)
//!   ├─ history_sessions_reporter() → HistorySessionsReporter
//!   ├─ history_items_reporter()  → HistoryItemsReporter
//!   ├─ clone_reporter()          → CloneReporter           (clone: scan + diff)
//!   ├─ sync_diff_reporter()      → SyncDiffReporter        (sync: sha256 diff)
//!   └─ sync_transfer_reporter()  → SyncTransferReporter    (sync/clone: file transfer)
//! ```
//!
//! Each reporter is obtained from the builder, used for exactly one phase,
//! then dropped.  `Drop` implementations guarantee that any progress
//! indicator is cleared even on early exit or panic.

mod types;
mod scan;
mod copy;
mod hash;
mod insert;
mod add;
mod recheck;
mod verify;
mod update;
mod history;
mod clone;
mod sync;
mod builder;
mod noop;
mod interactor;

pub use types::{CopyItemResult, ItemStatus, MatchConfidence};
pub use scan::ScanReporter;
pub use copy::CopyReporter;
pub use hash::HashReporter;
pub use insert::InsertReporter;
pub use add::AddSummaryReporter;
pub use recheck::RecheckReporter;
pub use verify::VerifyReporter;
pub use update::UpdateApplyReporter;
pub use history::{
    HistoryItemsQuery, HistoryItemsReporter, HistoryItemsSummary, HistoryItemRow,
    HistorySessionsQuery, HistorySessionsReporter, HistorySessionsSummary, HistorySessionRow,
};
pub use clone::CloneReporter;
pub use sync::{SyncDiffReporter, SyncTransferReporter};
pub use builder::ReporterBuilder;
pub use noop::{Noop, NoopReporterBuilder};
pub use interactor::{Interactor, YesInteractor};
