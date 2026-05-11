//! Reporter for the `verify` command (vault integrity check).

use std::path::Path;

/// Reporter for the `verify` command (vault integrity check).
pub trait VerifyReporter: Send + Sync {
    fn started(&self, total: u64);
    fn item_started(&self, path: &Path);
    fn item_finished(&self, path: &Path, result: &crate::verify::VerifyResult);
    fn finish(&self);
    fn summary(&self, summary: &crate::verify::VerifySummary);
}
