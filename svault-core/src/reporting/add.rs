//! Reporter for the `add` command's summary phase.

use std::path::{Path, PathBuf};

/// Reporter for the `add` command's summary phase.
///
/// The scan and hash phases reuse [`ScanReporter`] and [`HashReporter`].
/// This trait handles the add-specific pre-flight summary and the
/// "vault-internal move detected" hints.
pub trait AddSummaryReporter: Send + Sync {
    fn preflight(&self, new_count: usize, duplicate_count: usize, moved_count: usize);
    fn only_moved(&self, moved_files: &[(PathBuf, String)], vault_root: &Path);
    fn summary(&self, total: usize, added: usize, duplicate: usize, failed: usize);
    fn moved_hint(&self, moved_files: &[(PathBuf, String)], vault_root: &Path);
    fn finish(&self);
}
