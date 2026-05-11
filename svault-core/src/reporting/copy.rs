//! Reporter for the copy phase (file transfer).

use std::path::Path;

use super::types::CopyItemResult;

/// Reporter for the copy phase (file transfer).
/// Also reused by the sync transfer phase.
pub trait CopyReporter: Send + Sync {
    fn item_started(&self, src_abs: &Path, dest_abs: &Path, bytes_total: u64);
    fn item_progress(&self, src_abs: &Path, bytes_copied: u64, bytes_total: u64);
    fn item_finished(&self, src_abs: &Path, dest_abs: &Path, result: &CopyItemResult);
    fn finish(&self);
}
