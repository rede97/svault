//! Reporter for the DB insert phase.

use std::path::Path;

/// Reporter for the DB insert phase (Stage E).
///
/// Also carries the final import summary via [`InsertReporter::summary`],
/// since insert is the last pipeline stage.
pub trait InsertReporter: Send + Sync {
    fn progress(&self, completed: u64, total: u64);
    fn finish(&self);

    /// Final import summary after all pipeline stages complete.
    fn summary(
        &self,
        total: usize,
        imported: usize,
        duplicate: usize,
        failed: usize,
        manifest_path: Option<&Path>,
    );
}
