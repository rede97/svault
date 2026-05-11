//! Reporter trait for the `clone` command (export vault files to a plain directory).

/// Reporter for the `clone` command.
///
/// Covers the scan and diff phases: listing files in the source vault that
/// match the filter, and comparing against the target directory to skip
/// already-present files.  The transfer phase reuses
/// [`SyncTransferReporter`](super::SyncTransferReporter).
pub trait CloneReporter: Send + Sync {
    /// Clone started. `available` — files matching the filter in the source vault.
    fn started(&self, available: usize);

    /// Diff against target directory computed.
    /// `to_clone` — files to copy, `already_present` — skip, `total_bytes` to transfer.
    fn diff_computed(&self, to_clone: usize, already_present: usize, total_bytes: u64);

    /// Target already has everything matching the filter.
    fn nothing_to_clone(&self);

    /// The clone phase is complete.
    fn finish(&self);
}
