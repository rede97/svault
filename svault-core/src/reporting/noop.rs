//! No-op reporter implementations.

use std::path::{Path, PathBuf};

use super::{
    ScanReporter, CopyReporter, HashReporter, InsertReporter,
    AddSummaryReporter, RecheckReporter, VerifyReporter, UpdateApplyReporter,
    HistorySessionsReporter, HistoryItemsReporter,
    CloneReporter, SyncDiffReporter, SyncTransferReporter,
    HistorySessionsQuery, HistoryItemsQuery,
    HistorySessionRow, HistoryItemRow,
    HistorySessionsSummary, HistoryItemsSummary,
    ItemStatus, CopyItemResult, ReporterBuilder,
};

/// No-op reporter — silently discards all events.
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
        &self, _: usize, _: usize, _: usize, _: usize, _: usize, _: usize, _: usize, _: usize, _: &Path,
    ) {}
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

impl CloneReporter for Noop {
    fn started(&self, _: usize) {}
    fn diff_computed(&self, _: usize, _: usize, _: u64) {}
    fn nothing_to_clone(&self) {}
    fn finish(&self) {}
}

impl SyncDiffReporter for Noop {
    fn started(&self, _: usize, _: usize) {}
    fn diff_computed(&self, _: usize, _: usize, _: u64) {}
    fn nothing_to_sync(&self) {}
    fn finish(&self) {}
}

impl SyncTransferReporter for Noop {
    fn started(&self, _: u64, _: u64) {}
    fn item_started(&self, _: &Path, _: u64) {}
    fn item_progress(&self, _: &Path, _: u64, _: u64) {}
    fn item_finished(&self, _: &Path, _: &CopyItemResult) {}
    fn finish(&self) {}
    fn summary(&self, _: usize, _: usize, _: usize, _: u64) {}
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
    type Clone = Noop;
    type SyncDiff = Noop;
    type SyncTransfer = Noop;

    fn scan_reporter(&self, _: &Path) -> Noop { Noop }
    fn copy_reporter(&self, _: &Path, _: &Path, _: u64) -> Noop { Noop }
    fn hash_reporter(&self, _: &Path, _: u64) -> Noop { Noop }
    fn insert_reporter(&self, _: &Path, _: u64) -> Noop { Noop }
    fn add_summary_reporter(&self, _: &Path) -> Noop { Noop }
    fn recheck_reporter(&self, _: u64) -> Noop { Noop }
    fn update_hash_reporter(&self, _: &Path, _: u64) -> Noop { Noop }
    fn update_apply_reporter(&self, _: u64) -> Noop { Noop }
    fn verify_reporter(&self, _: u64) -> Noop { Noop }
    fn history_sessions_reporter(&self, _: &HistorySessionsQuery) -> Noop { Noop }
    fn history_items_reporter(&self, _: &str, _: &HistoryItemsQuery) -> Noop { Noop }
    fn clone_reporter(&self) -> Noop { Noop }
    fn sync_diff_reporter(&self) -> Noop { Noop }
    fn sync_transfer_reporter(&self, _: &Path, _: &Path, _: u64) -> Noop { Noop }
}
