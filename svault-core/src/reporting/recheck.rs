//! Reporter for the `recheck` command (manifest integrity verification).

use std::path::Path;

/// Reporter for the `recheck` command (manifest integrity verification).
pub trait RecheckReporter: Send + Sync {
    fn started(&self, total: usize, session_id: &str, source: &Path);
    fn item_started(&self, src_path: &Path, vault_path: &Path);
    fn item_finished(&self, src_path: &Path, vault_path: &Path, status: &crate::import::RecheckStatus);
    fn finish(&self);

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
