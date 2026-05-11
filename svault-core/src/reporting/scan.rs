//! Reporter for the scan phase (walk + CRC + DB lookup).

use std::path::Path;

use super::types::ItemStatus;

/// Reporter for the scan phase (Stages A + B + C: walk + CRC + DB lookup).
pub trait ScanReporter: Send + Sync {
    /// A file has been scanned and classified.
    fn item(&self, path: &Path, size: u64, mtime_ms: i64, status: ItemStatus, error: Option<&str>);

    /// Pre-flight summary emitted after scan and before confirmation.
    fn preflight(
        &self,
        total_scanned: usize,
        new_count: usize,
        duplicate_count: usize,
        moved_count: usize,
        failed_count: usize,
        source: &Path,
    );

    /// The scan phase is complete.
    fn finish(&self);
}
