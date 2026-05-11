//! Reporter builder trait — creates typed phase reporters.

use std::path::Path;

use super::{
    ScanReporter, CopyReporter, HashReporter, InsertReporter,
    AddSummaryReporter, RecheckReporter, VerifyReporter, UpdateApplyReporter,
    HistorySessionsReporter, HistoryItemsReporter,
    HistorySessionsQuery, HistoryItemsQuery,
    CloneReporter, SyncDiffReporter, SyncTransferReporter,
};

/// Creates typed phase reporters.
///
/// Each `*_reporter` method returns an owned value whose `Drop`
/// implementation guarantees any progress indicator is cleared when
/// the phase ends.
pub trait ReporterBuilder: Send + Sync {
    // ── import pipeline ──
    type Scan: ScanReporter;
    type Copy: CopyReporter;
    type Hash: HashReporter;
    type Insert: InsertReporter;

    fn scan_reporter(&self, source: &Path) -> Self::Scan;
    fn copy_reporter(&self, source: &Path, vault_root: &Path, total: u64) -> Self::Copy;
    fn hash_reporter(&self, source: &Path, total: u64) -> Self::Hash;
    fn insert_reporter(&self, source: &Path, total: u64) -> Self::Insert;

    // ── add command ──
    type AddSummary: AddSummaryReporter;

    fn add_summary_reporter(&self, vault_root: &Path) -> Self::AddSummary;

    // ── recheck command ──
    type Recheck: RecheckReporter;

    fn recheck_reporter(&self, total: u64) -> Self::Recheck;

    // ── update command ──
    type UpdateApply: UpdateApplyReporter;

    fn update_hash_reporter(&self, source: &Path, total: u64) -> Self::Hash;
    fn update_apply_reporter(&self, total: u64) -> Self::UpdateApply;

    // ── verify command ──
    type Verify: VerifyReporter;

    fn verify_reporter(&self, total: u64) -> Self::Verify;

    // ── history command ──
    type HistorySessions: HistorySessionsReporter;
    type HistoryItems: HistoryItemsReporter;

    fn history_sessions_reporter(&self, query: &HistorySessionsQuery) -> Self::HistorySessions;
    fn history_items_reporter(&self, session_id: &str, query: &HistoryItemsQuery) -> Self::HistoryItems;

    // ── clone ──
    type Clone: CloneReporter;

    /// Create a reporter for the clone phase (vault scan + target diff).
    fn clone_reporter(&self) -> Self::Clone;

    // ── sync ──
    type SyncDiff: SyncDiffReporter;
    type SyncTransfer: SyncTransferReporter;

    /// Create a reporter for the sync diff phase (SHA-256 set comparison).
    fn sync_diff_reporter(&self) -> Self::SyncDiff;
    /// Create a reporter for the sync transfer phase (file copy to target).
    fn sync_transfer_reporter(&self, source: &Path, target: &Path, total: u64) -> Self::SyncTransfer;
}
