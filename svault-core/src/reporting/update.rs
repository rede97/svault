//! Reporter for the `update` command's path-apply phase.

/// Reporter for the `update` command's path-apply phase.
///
/// Applies the matched path corrections to the database (and optionally
/// marks unmatched records as missing / deleted).
pub trait UpdateApplyReporter: Send + Sync {
    fn progress(&self, completed: u64, total: u64);
    fn error(&self, message: &str, path: &str);
    fn finish(&self);
    fn summary(
        &self,
        scanned: usize,
        missing: usize,
        matched: usize,
        unmatched: usize,
        updated: usize,
    );
    fn nothing_to_update(&self);
    fn dry_run_missing(&self, count: usize);
}
