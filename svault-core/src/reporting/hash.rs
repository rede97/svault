//! Reporter for the hash phase (XXH3-128 / SHA-256).

use std::path::Path;

use super::types::MatchConfidence;

/// Reporter for the hash phase (Stage D: XXH3-128 / SHA-256 computation).
/// Also used for the `update` command's hash-and-match phase.
pub trait HashReporter: Send + Sync {
    fn item_started(&self, abs_path: &Path, bytes_total: u64);
    fn item_finished(&self, abs_path: &Path, error: Option<&str>, bytes_total: u64);

    /// A relocate match was found (for `update` command).
    fn matched(&self, _old_path: &str, _new_path: &str, _confidence: MatchConfidence) {}

    fn finish(&self);
}
