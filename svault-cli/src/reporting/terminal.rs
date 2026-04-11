//! Terminal-based human-readable reporter with indicatif MultiProgress.
//!
//! Provides typed reporters for each pipeline phase and command:
//!
//! | Struct                        | Trait                | Phase/Command       |
//! |-------------------------------|----------------------|---------------------|
//! | `TerminalScanReporter`        | `ScanReporter`       | walk + lookup       |
//! | `TerminalCopyReporter`        | `CopyReporter`       | file transfer       |
//! | `TerminalHashReporter`        | `HashReporter`       | XXH3 / SHA-256      |
//! | `TerminalInsertReporter`      | `InsertReporter`     | DB + summary        |
//! | `TerminalAddSummaryReporter`  | `AddSummaryReporter` | add command         |
//! | `TerminalRecheckReporter`     | `RecheckReporter`    | recheck command     |
//! | `TerminalUpdateHashReporter`  | `UpdateHashReporter` | update (hash phase) |
//! | `TerminalUpdateApplyReporter` | `UpdateApplyReporter`| update (apply phase)|
//! | `TerminalVerifyReporter`      | `VerifyReporter`     | verify command      |
//! | `TerminalBackgroundHashReporter` | `BackgroundHashReporter` | background-hash |
//!
//! Each reporter owns exactly one `ProgressBar` added to the shared
//! `MultiProgress`.  `Drop` calls `finish_and_clear()` as a safety net
//! so bars are always cleaned up even on early return or panic.
//! Explicit `finish()` calls print the completion summary line first.
//!
//! ## Output Threading
//!
//! All output is batched into single `String` buffers before calling
//! `pb.println()` once. This prevents interleaved output in multi-threaded
//! contexts (e.g., when multiple Rayon threads report status simultaneously).

use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use svault_core::reporting::{
    AddSummaryReporter, BackgroundHashReporter, CopyReporter, HashReporter, InsertReporter,
    Interactor, ItemStatus, MatchConfidence, RecheckReporter, ReporterBuilder, ScanReporter,
    UpdateApplyReporter, UpdateHashReporter, VerifyReporter,
};

/// Braille pattern spinner characters for progress bars.
///
/// Unicode block characters that animate in sequence to indicate activity.
const TICK_CHARS: &str = "⠁⠂⠄⡀⢀⠠⠐⠈ ";

// ─────────────────────────────────────────────────────────────────────────────
// Scan reporter
// ─────────────────────────────────────────────────────────────────────────────

/// Counters for tracking file classification results during scan phase.
///
/// Used internally by [`TerminalScanReporter`] to accumulate counts of
/// files by their [`ItemStatus`] for the final summary.
#[derive(Debug, Default)]
struct ScanCounters {
    /// Number of new files (will be imported).
    new: u64,
    /// Number of duplicate files (already in vault).
    duplicate: u64,
    /// Number of recoverable files (marked missing, now found).
    recover: u64,
    /// Number of files moved within the vault.
    moved: u64,
    /// Number of files with errors during scan.
    failed: u64,
}

/// Terminal reporter for the scan phase.
///
/// Shows a spinner with a live file count.  On `finish()` prints a one-line
/// completion summary.  The `preflight` and `nothing_to_import` methods
/// print structured output via `pb.println` so they never interfere with
/// the spinner.
pub struct TerminalScanReporter {
    pb: ProgressBar,
    counters: Mutex<ScanCounters>,
}

impl TerminalScanReporter {
    /// Print a line through the progress bar.
    ///
    /// This ensures output is synchronized with the spinner's draw cycle,
    /// preventing screen corruption.
    fn println<S: AsRef<str>>(&self, s: S) {
        self.pb.println(s);
    }
}

impl ScanReporter for TerminalScanReporter {
    fn discovered(&self, _path: &Path, _size: u64, _mtime_ms: i64) {
        // Counted implicitly via progress(); no per-file line for discoveries.
    }

    fn classified(&self, path: &Path, status: ItemStatus, _detail: Option<&str>) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());

        {
            let mut c = self.counters.lock().unwrap();
            match status {
                ItemStatus::New => c.new += 1,
                ItemStatus::Duplicate => c.duplicate += 1,
                ItemStatus::MovedInVault => c.moved += 1,
                ItemStatus::Recover => c.recover += 1,
                ItemStatus::Failed => c.failed += 1,
            }
        }

        match status {
            ItemStatus::New => {
                self.println(format!(
                    "  {} {}",
                    style("Found").green(),
                    style(name).dim()
                ));
            }
            ItemStatus::Duplicate => {
                self.println(format!(
                    "  {} {}",
                    style("Duplicate").yellow(),
                    style(name).dim()
                ));
            }
            ItemStatus::MovedInVault => {
                self.println(format!("  {} {}", style("Moved").cyan(), style(name).dim()));
            }
            ItemStatus::Recover => {
                self.println(format!(
                    "  {} {}",
                    style("Recover").cyan(),
                    style(name).dim()
                ));
            }
            ItemStatus::Failed => {
                self.println(format!("  {} {}", style("Error").red(), style(name).dim()));
            }
        }
    }

    fn progress(&self, completed: u64) {
        self.pb.set_position(completed);
    }

    fn warning(&self, message: &str, path: Option<&Path>) {
        let msg = path
            .map(|p| format!("{}: {}", p.display(), message))
            .unwrap_or_else(|| message.to_string());
        self.println(format!("{} {}", style("Warning:").yellow().bold(), msg));
    }

    fn error(&self, message: &str, path: Option<&Path>) {
        let msg = path
            .map(|p| format!("{}: {}", p.display(), message))
            .unwrap_or_else(|| message.to_string());
        self.println(format!("{} {}", style("Error:").red().bold(), msg));
    }

    fn preflight(
        &self,
        total_scanned: usize,
        new_count: usize,
        duplicate_count: usize,
        moved_count: usize,
        failed_count: usize,
        source: &Path,
    ) {
        let mut output = String::new();
        output.push_str(&format!(
            "{} Scanned {} files from {}\n",
            style("Finished:").bold().green(),
            style(total_scanned).green(),
            style(source.display()).dim()
        ));
        output.push('\n');
        output.push_str(&format!("{}\n", style("Pre-flight:").bold()));
        output.push_str(&format!(
            "  {}  {}\n",
            style(format!("Likely new:       {:>6}", new_count)).green(),
            style("will be imported").dim()
        ));
        output.push_str(&format!(
            "  {}  {}\n",
            style(format!("Likely duplicate: {:>6}", duplicate_count)).yellow(),
            style("already in vault (cache hit)").dim()
        ));
        if moved_count > 0 {
            output.push_str(&format!(
                "  {}  {}\n",
                style(format!("Moved in vault:   {:>6}", moved_count)).cyan(),
                style("path will be updated").dim()
            ));
        }
        if failed_count > 0 {
            output.push_str(&format!(
                "  {}\n",
                style(format!("Errors:           {:>6}", failed_count)).red()
            ));
        }
        self.println(output);
    }

    fn nothing_to_import(&self, total: usize, _duplicate: usize) {
        self.println(format!(
            "All {} files matched cache (no new files detected).",
            style(total).cyan()
        ));
    }

    fn finish(&self) {
        let c = self.counters.lock().unwrap();
        let summary = format!(
            "✓ Scan complete ({} files; new {}, duplicate {}, recover {}, moved {}, failed {})",
            self.pb.position(),
            c.new,
            c.duplicate,
            c.recover,
            c.moved,
            c.failed
        );
        self.println(&summary);
        self.pb.finish_and_clear();
    }
}

impl Drop for TerminalScanReporter {
    fn drop(&mut self) {
        // Safety net: clears the bar if finish() was not called explicitly.
        self.pb.finish_and_clear();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Copy reporter
// ─────────────────────────────────────────────────────────────────────────────

/// Terminal reporter for the copy phase.
pub struct TerminalCopyReporter {
    pb: ProgressBar,
    total: u64,
}

impl TerminalCopyReporter {
    /// Print a line through the progress bar for synchronized output.
    fn println<S: AsRef<str>>(&self, s: S) {
        self.pb.println(s);
    }
}

impl CopyReporter for TerminalCopyReporter {
    fn item_started(&self, path: &Path, _bytes_total: Option<u64>) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        self.pb.set_message(name);
    }

    fn item_finished(&self, _path: &Path) {}

    fn progress(&self, completed: u64, _total: u64) {
        self.pb.set_position(completed);
    }

    fn error(&self, message: &str, path: Option<&Path>) {
        let msg = path
            .map(|p| format!("{}: {}", p.display(), message))
            .unwrap_or_else(|| message.to_string());
        self.println(format!("{} {}", style("Error:").red().bold(), msg));
    }

    fn finish(&self) {
        let summary = format!("✓ Copy complete ({}/{})", self.pb.position(), self.total);
        self.println(&summary);
        self.pb.finish_and_clear();
    }
}

impl Drop for TerminalCopyReporter {
    fn drop(&mut self) {
        self.pb.finish_and_clear();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Hash reporter
// ─────────────────────────────────────────────────────────────────────────────

/// Terminal reporter for the hash phase.
pub struct TerminalHashReporter {
    pb: ProgressBar,
    total: u64,
}

impl TerminalHashReporter {
    /// Print a line through the progress bar for synchronized output.
    fn println<S: AsRef<str>>(&self, s: S) {
        self.pb.println(s);
    }
}

impl HashReporter for TerminalHashReporter {
    fn progress(&self, completed: u64, _total: u64) {
        self.pb.set_position(completed);
    }

    fn finish(&self) {
        let summary = format!(
            "✓ Fingerprint complete ({}/{})",
            self.pb.position(),
            self.total
        );
        self.println(&summary);
        self.pb.finish_and_clear();
    }
}

impl Drop for TerminalHashReporter {
    fn drop(&mut self) {
        self.pb.finish_and_clear();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Insert reporter
// ─────────────────────────────────────────────────────────────────────────────

/// Terminal reporter for the DB insert phase.
///
/// `finish()` clears the progress bar.  `summary()` then prints the final
/// human-readable import summary to stdout (below any cleared bars).
pub struct TerminalInsertReporter {
    pb: ProgressBar,
    total: u64,
}

impl TerminalInsertReporter {
    /// Print a line through the progress bar for synchronized output.
    fn println<S: AsRef<str>>(&self, s: S) {
        self.pb.println(s);
    }
}

impl InsertReporter for TerminalInsertReporter {
    fn progress(&self, completed: u64, _total: u64) {
        self.pb.set_position(completed);
    }

    fn finish(&self) {
        let summary = format!("✓ Insert complete ({}/{})", self.pb.position(), self.total);
        self.println(&summary);
        self.pb.finish_and_clear();
    }

    fn summary(
        &self,
        total: usize,
        imported: usize,
        duplicate: usize,
        failed: usize,
        manifest_path: Option<&Path>,
    ) {
        let mut output = String::new();
        output.push('\n');
        output.push_str(&format!(
            "{}\n",
            style("Import operation completed").green().bold()
        ));
        output.push_str(&format!("  Total files processed: {}\n", total));
        if imported > 0 {
            output.push_str(&format!(
                "  {}\n",
                style(format!("New files imported:  {}", imported)).green()
            ));
        }
        if duplicate > 0 {
            output.push_str(&format!(
                "  {}\n",
                style(format!("Duplicates skipped:  {}", duplicate)).yellow()
            ));
        }
        if failed > 0 {
            output.push_str(&format!(
                "  {}\n",
                style(format!("Failed:              {}", failed)).red()
            ));
        }
        if let Some(p) = manifest_path {
            output.push_str(&format!("  Manifest: {}\n", style(p.display()).dim()));
        }
        self.println(output);
    }
}

impl Drop for TerminalInsertReporter {
    fn drop(&mut self) {
        self.pb.finish_and_clear();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Verify reporter
// ─────────────────────────────────────────────────────────────────────────────

/// Terminal reporter for the `verify` command.
///
/// Displays a progress bar for vault verification and prints per-file
/// status lines. Outputs a final summary with counts of OK, missing,
/// size/hash mismatches, and I/O errors.
pub struct TerminalVerifyReporter {
    pb: ProgressBar,
}

impl TerminalVerifyReporter {
    /// Print a line through the progress bar for synchronized output.
    fn println<S: AsRef<str>>(&self, s: S) {
        self.pb.println(s);
    }
}

impl VerifyReporter for TerminalVerifyReporter {
    fn started(&self, _total: u64) {
        self.pb.set_message("Verifying...");
    }

    fn progress(&self, completed: u64, _total: u64) {
        self.pb.set_position(completed);
    }

    fn verified(&self, path: &Path) {
        if let Some(name) = path.file_name() {
            self.pb.println(format!(
                "  {} {}",
                style("Verified").green(),
                style(name.to_string_lossy()).dim()
            ));
        }
    }

    fn finish(&self) {
        self.pb.finish_and_clear();
    }

    fn summary(&self, summary: &svault_core::verify::VerifySummary) {
        let mut output = String::new();
        output.push('\n');
        output.push_str(&format!("{}\n", style("Verify complete").green().bold()));
        output.push_str(&format!("  Total: {}\n", summary.total));
        output.push_str(&format!("  OK: {}\n", style(summary.ok).green()));
        if summary.missing > 0 {
            output.push_str(&format!("  Missing: {}\n", style(summary.missing).red()));
        }
        if summary.size_mismatch > 0 {
            output.push_str(&format!(
                "  Size mismatch: {}\n",
                style(summary.size_mismatch).red()
            ));
        }
        if summary.hash_mismatch > 0 {
            output.push_str(&format!(
                "  Hash mismatch: {}\n",
                style(summary.hash_mismatch).red()
            ));
        }
        if summary.io_error > 0 {
            output.push_str(&format!("  IO errors: {}\n", style(summary.io_error).red()));
        }
        self.println(output);
    }
}

impl Drop for TerminalVerifyReporter {
    fn drop(&mut self) {
        self.pb.finish_and_clear();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Background hash reporter
// ─────────────────────────────────────────────────────────────────────────────

/// Terminal reporter for the `background-hash` command.
///
/// Displays progress for computing XXH3-128 and SHA-256 hashes for files
/// that were imported before full hashing was implemented. Prints per-file
/// status and a final summary with processed/failed counts.
pub struct TerminalBackgroundHashReporter {
    pb: ProgressBar,
}

impl TerminalBackgroundHashReporter {
    /// Print a line through the progress bar for synchronized output.
    fn println<S: AsRef<str>>(&self, s: S) {
        self.pb.println(s);
    }
}

impl BackgroundHashReporter for TerminalBackgroundHashReporter {
    fn started(&self, _total: u64) {
        self.pb.set_message("Hashing...");
    }

    fn progress(&self, completed: u64, _total: u64, _current_path: &Path) {
        self.pb.set_position(completed);
    }

    fn hashed(&self, path: &Path) {
        if let Some(name) = path.file_name() {
            self.pb.println(format!(
                "  {} {}",
                style("Hashing").green(),
                style(name.to_string_lossy()).dim()
            ));
        }
    }

    fn error(&self, path: &Path, message: &str) {
        if let Some(name) = path.file_name() {
            self.pb.println(format!(
                "  {} {}: {}",
                style("Error").red(),
                style(name.to_string_lossy()).dim(),
                message
            ));
        }
    }

    fn finish(&self) {
        self.pb.finish_and_clear();
    }

    fn summary(&self, processed: usize, failed: usize) {
        let mut output = String::new();
        output.push('\n');
        output.push_str(&format!(
            "{}\n",
            style("Background hash complete").green().bold()
        ));
        output.push_str(&format!("  Processed: {}\n", style(processed).green()));
        if failed > 0 {
            output.push_str(&format!("  Failed: {}\n", style(failed).red()));
        }
        self.println(output);
    }
}

impl Drop for TerminalBackgroundHashReporter {
    fn drop(&mut self) {
        self.pb.finish_and_clear();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Builder
// ─────────────────────────────────────────────────────────────────────────────

/// Builder that creates terminal reporters sharing one `MultiProgress`.
///
/// The `MultiProgress` is cleared on `Drop`, so all remaining bars are
/// cleaned up when the builder goes out of scope.
pub struct TerminalReporterBuilder {
    /// The shared `MultiProgress` instance used by all reporters created from this builder.
    pub multi_progress: MultiProgress,
}

impl TerminalReporterBuilder {
    /// Create a new builder with a fresh `MultiProgress` instance.
    ///
    /// All reporters created from this builder will share the same
    /// `MultiProgress`, ensuring coordinated progress bar rendering.
    pub fn new() -> Self {
        Self {
            multi_progress: MultiProgress::new(),
        }
    }

    /// Suspend the `MultiProgress` display while `f` runs.
    ///
    /// Used by [`SuspendingInteractor`] to prevent the progress bars from
    /// fighting with the confirmation prompt on stderr.
    pub fn suspend<F: FnOnce() -> R, R>(&self, f: F) -> R {
        self.multi_progress.suspend(f)
    }

    /// Create and register a spinner in MultiProgress, then configure it.
    ///
    /// IMPORTANT: Registration must happen before any style/message changes to
    /// avoid terminal redraw corruption in multi-threaded scenarios.
    fn add_managed_spinner<F>(&self, configure: F) -> ProgressBar
    where
        F: FnOnce(&ProgressBar),
    {
        let pb = self.multi_progress.add(ProgressBar::new_spinner());
        configure(&pb);
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    }

    /// Create and register a progress bar in MultiProgress, then configure it.
    ///
    /// IMPORTANT: Registration must happen before any style/message changes to
    /// avoid terminal redraw corruption in multi-threaded scenarios.
    fn add_managed_bar<F>(&self, total: u64, configure: F) -> ProgressBar
    where
        F: FnOnce(&ProgressBar),
    {
        let pb = self.multi_progress.add(ProgressBar::new(total));
        configure(&pb);
        // Note: No enable_steady_tick here - progress bars update via set_position()
        pb
    }

    /// Create and register a hidden progress bar for non-visual output.
    ///
    /// Used for phases that need to print output without showing a progress bar,
    /// such as summary-only reporters.
    fn add_managed_hidden<F>(&self, configure: F) -> ProgressBar
    where
        F: FnOnce(&ProgressBar),
    {
        let pb = self.multi_progress.add(ProgressBar::hidden());
        configure(&pb);
        pb
    }
}

impl Drop for TerminalReporterBuilder {
    fn drop(&mut self) {
        let _ = self.multi_progress.clear();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Add summary reporter
// ─────────────────────────────────────────────────────────────────────────────

/// Terminal reporter for the `add` command's summary phase.
///
/// Uses a hidden ProgressBar as the output carrier.
pub struct TerminalAddSummaryReporter {
    pb: ProgressBar,
}

impl TerminalAddSummaryReporter {
    /// Print a line through the progress bar for synchronized output.
    fn println<S: AsRef<str>>(&self, s: S) {
        self.pb.println(s);
    }
}

impl AddSummaryReporter for TerminalAddSummaryReporter {
    fn preflight(&self, new_count: usize, duplicate_count: usize, moved_count: usize) {
        let mut output = String::new();
        output.push('\n');
        output.push_str(&format!("{}\n", style("Pre-flight:").bold()));
        output.push_str(&format!(
            "  {}  {}\n",
            style(format!("Likely new:       {:>6}", new_count)).green(),
            style("will be added").dim()
        ));
        if duplicate_count > 0 {
            output.push_str(&format!(
                "  {}  {}\n",
                style(format!("Likely duplicate: {:>6}", duplicate_count)).yellow(),
                style("already in vault").dim()
            ));
        }
        if moved_count > 0 {
            output.push_str(&format!(
                "  {}  {}\n",
                style(format!("Moved:            {:>6}", moved_count)).cyan(),
                style("vault-internal move detected").dim()
            ));
        }
        self.println(output);
    }

    fn only_moved(
        &self,
        moved_files: &[(std::path::PathBuf, String)],
        vault_root: &std::path::Path,
    ) {
        let mut output = String::new();
        output.push('\n');
        output.push_str(&format!("{}\n", style("Note:").bold().cyan()));
        output.push_str(&format!(
            "  {} file(s) appear to have been moved within the vault.\n",
            style(moved_files.len()).cyan()
        ));
        output.push_str(&format!(
            "  Use {} to update their paths:\n",
            style("svault update").bold()
        ));
        for (current, old) in moved_files.iter().take(3) {
            let rel = current.strip_prefix(vault_root).unwrap_or(current);
            output.push_str(&format!(
                "    {} → {}\n",
                style(old).dim(),
                style(rel.display()).cyan()
            ));
        }
        if moved_files.len() > 3 {
            output.push_str(&format!("    ... and {} more\n", moved_files.len() - 3));
        }
        self.println(output);
    }

    fn summary(&self, _total: usize, added: usize, duplicate: usize, failed: usize) {
        let mut output = String::new();
        output.push_str(&format!(
            "{} {} file(s) added\n",
            style("Finished:").bold().green(),
            style(added).green()
        ));
        if duplicate > 0 {
            output.push_str(&format!(
                "         {} duplicate(s) skipped\n",
                style(duplicate).yellow()
            ));
        }
        if failed > 0 {
            output.push_str(&format!(
                "         {} file(s) failed\n",
                style(failed).red()
            ));
        }
        self.println(output);
    }

    fn moved_hint(
        &self,
        moved_files: &[(std::path::PathBuf, String)],
        vault_root: &std::path::Path,
    ) {
        let mut output = String::new();
        output.push('\n');
        output.push_str(&format!("{}\n", style("Note:").bold().cyan()));
        output.push_str(&format!(
            "  {} file(s) appear to have been moved within the vault.\n",
            style(moved_files.len()).cyan()
        ));
        output.push_str(&format!(
            "  Use {} to update their paths:\n",
            style("svault update").bold()
        ));
        for (current, old) in moved_files.iter().take(3) {
            let rel = current.strip_prefix(vault_root).unwrap_or(current);
            output.push_str(&format!(
                "    {} → {}\n",
                style(old).dim(),
                style(rel.display()).cyan()
            ));
        }
        if moved_files.len() > 3 {
            output.push_str(&format!("    ... and {} more\n", moved_files.len() - 3));
        }
        self.println(output);
    }

    fn finish(&self) {
        self.pb.finish_and_clear();
    }
}

impl Drop for TerminalAddSummaryReporter {
    fn drop(&mut self) {
        self.pb.finish_and_clear();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Recheck reporter
// ─────────────────────────────────────────────────────────────────────────────

/// Terminal reporter for the `recheck` command.
pub struct TerminalRecheckReporter {
    pb: ProgressBar,
    total: u64,
}

impl TerminalRecheckReporter {
    /// Print a line through the progress bar for synchronized output.
    fn println<S: AsRef<str>>(&self, s: S) {
        self.pb.println(s);
    }
}

impl RecheckReporter for TerminalRecheckReporter {
    fn started(&self, total: usize, session_id: &str, source: &std::path::Path) {
        let mut output = String::new();
        output.push_str(&format!(
            "{} Rechecking {} files from session {}\n",
            style("Recheck:").bold().cyan(),
            style(total).cyan(),
            style(session_id)
        ));
        output.push_str(&format!("  Source: {}\n", style(source.display())));
        output.push('\n');
        output.push_str(&format!(
            "{} {}\n",
            style("Caution:").yellow().bold(),
            style("Recheck assumes the source device has not changed since import.").yellow()
        ));
        output.push_str(&format!(
            "         {}\n",
            style("If you took new photos or modified files, filenames may be reused with different content.")
        ));
        output.push_str(&format!(
            "         {}\n",
            style("Please review the report carefully before deleting anything.")
        ));
        self.println(output);
    }

    fn progress(&self, completed: u64, _total: u64) {
        self.pb.set_position(completed);
    }

    fn finish(&self) {
        let summary = format!("✓ Recheck complete ({}/{})", self.pb.position(), self.total);
        self.println(&summary);
        self.pb.finish_and_clear();
    }

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
        report_path: &std::path::Path,
    ) {
        let mut output = String::new();
        output.push_str(&format!("{}\n", style("Results:").bold().underlined()));
        output.push_str(&format!("  {} OK\n", style(format!("{:>4}", ok)).green()));
        if source_modified > 0 {
            output.push_str(&format!(
                "  {} Source modified\n",
                style(format!("{:>4}", source_modified)).yellow()
            ));
        }
        if vault_corrupted > 0 {
            output.push_str(&format!(
                "  {} Vault corrupted\n",
                style(format!("{:>4}", vault_corrupted)).red()
            ));
        }
        if both_diverged > 0 {
            output.push_str(&format!(
                "  {} Both diverged\n",
                style(format!("{:>4}", both_diverged)).red()
            ));
        }
        if source_deleted > 0 {
            output.push_str(&format!(
                "  {} Source deleted\n",
                style(format!("{:>4}", source_deleted)).yellow()
            ));
        }
        if vault_deleted > 0 {
            output.push_str(&format!(
                "  {} Vault deleted\n",
                style(format!("{:>4}", vault_deleted)).red()
            ));
        }
        if errors > 0 {
            output.push_str(&format!(
                "  {} Errors\n",
                style(format!("{:>4}", errors)).red()
            ));
        }
        if sha256_verified > 0 {
            output.push_str(&format!(
                "  ({} files verified with SHA-256)\n",
                sha256_verified
            ));
        }
        output.push('\n');
        output.push_str(&format!(
            "{} Report written to {}\n",
            style("Report:").bold(),
            style(report_path.display()).underlined()
        ));
        self.println(output);
    }
}

impl Drop for TerminalRecheckReporter {
    fn drop(&mut self) {
        self.pb.finish_and_clear();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Update hash reporter
// ─────────────────────────────────────────────────────────────────────────────

/// Terminal reporter for the `update` command's hash-and-match phase.
pub struct TerminalUpdateHashReporter {
    pb: ProgressBar,
    total: u64,
    matches: Mutex<Vec<(String, String, MatchConfidence)>>,
}

impl TerminalUpdateHashReporter {
    /// Print a line through the progress bar for synchronized output.
    fn println<S: AsRef<str>>(&self, s: S) {
        self.pb.println(s);
    }
}

impl UpdateHashReporter for TerminalUpdateHashReporter {
    fn progress(&self, completed: u64, _total: u64) {
        self.pb.set_position(completed);
    }

    fn matched(&self, old_path: &str, new_path: &str, confidence: MatchConfidence) {
        self.matches
            .lock()
            .unwrap()
            .push((old_path.to_string(), new_path.to_string(), confidence));
    }

    fn finish(&self) {
        let mut output = String::new();
        output.push_str(&format!(
            "✓ Hash scan complete ({}/{})\n",
            self.pb.position(),
            self.total
        ));

        // Print matches through pb.println before clearing
        let matches = self.matches.lock().unwrap();
        output.push('\n');
        output.push_str(&format!("{}\n", style("Matches found:").bold()));
        if matches.is_empty() {
            output.push_str(&format!("  {} No relocated files detected.\n", style("-")));
        } else {
            for (old, new, conf) in matches.iter() {
                let icon = match conf {
                    MatchConfidence::Definitive => style("✓").green(),
                    MatchConfidence::Fast => style("~").yellow(),
                };
                output.push_str(&format!(
                    "  {} {}  {} → {}\n",
                    icon,
                    style(old),
                    style("→").dim(),
                    style(new).green()
                ));
            }
            let definitive = matches
                .iter()
                .filter(|(_, _, c)| *c == MatchConfidence::Definitive)
                .count();
            let fast = matches.len() - definitive;
            if definitive > 0 {
                output.push_str(&format!(
                    "    {} {} definitive (SHA-256)\n",
                    style("✓").green(),
                    definitive
                ));
            }
            if fast > 0 {
                output.push_str(&format!(
                    "    {} {} fast (XXH3-128 only)\n",
                    style("~").yellow(),
                    fast
                ));
            }
        }
        self.println(output);
        self.pb.finish_and_clear();
    }
}

impl Drop for TerminalUpdateHashReporter {
    fn drop(&mut self) {
        self.pb.finish_and_clear();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Update apply reporter
// ─────────────────────────────────────────────────────────────────────────────

/// Terminal reporter for the `update` command's path-apply phase.
pub struct TerminalUpdateApplyReporter {
    pb: ProgressBar,
    total: u64,
}

impl TerminalUpdateApplyReporter {
    /// Print a line through the progress bar for synchronized output.
    fn println<S: AsRef<str>>(&self, s: S) {
        self.pb.println(s);
    }
}

impl UpdateApplyReporter for TerminalUpdateApplyReporter {
    fn progress(&self, completed: u64, _total: u64) {
        self.pb.set_position(completed);
    }

    fn error(&self, message: &str, path: &str) {
        self.println(format!(
            "{} Failed to update {}: {}",
            style("Error:").red().bold(),
            style(path),
            message
        ));
    }

    fn finish(&self) {
        let summary = format!("✓ Update complete ({}/{})", self.pb.position(), self.total);
        self.println(&summary);
        self.pb.finish_and_clear();
    }

    fn summary(
        &self,
        scanned: usize,
        missing: usize,
        matched: usize,
        unmatched: usize,
        updated: usize,
    ) {
        let mut output = String::new();
        output.push('\n');
        output.push_str(&format!("{}\n", style("Summary:").bold()));
        output.push_str(&format!("  Scanned: {} file(s) on disk\n", scanned));
        output.push_str(&format!("  Missing: {} file(s) from DB\n", missing));
        output.push_str(&format!(
            "  Matched: {} file(s) relocated\n",
            style(matched).green()
        ));
        if unmatched > 0 {
            output.push_str(&format!(
                "  Unmatched: {} file(s) not found\n",
                style(unmatched).yellow()
            ));
            output.push('\n');
            output.push_str(&format!(
                "Cleaned: {} file(s) marked as missing\n",
                unmatched
            ));
        }
        if updated > 0 {
            output.push_str(&format!(
                "  Updated: {} file(s) path corrected\n",
                style(updated).green().bold()
            ));
        }
        self.println(output);
    }

    fn nothing_to_update(&self) {
        self.println("All tracked files exist. Nothing to reconcile.");
    }

    fn dry_run_missing(&self, count: usize) {
        self.println(format!("Files to mark as missing: {}", count));
    }
}

impl Drop for TerminalUpdateApplyReporter {
    fn drop(&mut self) {
        self.pb.finish_and_clear();
    }
}

impl ReporterBuilder for TerminalReporterBuilder {
    type Scan = TerminalScanReporter;
    type Copy = TerminalCopyReporter;
    type Hash = TerminalHashReporter;
    type Insert = TerminalInsertReporter;
    type AddSummary = TerminalAddSummaryReporter;
    type Recheck = TerminalRecheckReporter;
    type UpdateHash = TerminalUpdateHashReporter;
    type UpdateApply = TerminalUpdateApplyReporter;
    type Verify = TerminalVerifyReporter;
    type BackgroundHash = TerminalBackgroundHashReporter;

    fn scan_reporter(&self, _source: &Path) -> TerminalScanReporter {
        let pb = self.add_managed_spinner(|pb| {
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.cyan} {msg} {pos:>7} files")
                    .unwrap()
                    .tick_chars(TICK_CHARS),
            );
            pb.set_message("Scanning...");
        });
        TerminalScanReporter {
            pb,
            counters: Mutex::new(ScanCounters::default()),
        }
    }

    fn copy_reporter(&self, _source: &Path, total: u64) -> TerminalCopyReporter {
        let pb = self.add_managed_bar(total, |pb| {
            pb.set_style(
                ProgressStyle::default_bar()
                    .template(
                        "{spinner:.green} {msg} [{bar:40.green/blue}] {pos}/{len} ({percent}%)",
                    )
                    .unwrap()
                    .progress_chars("=> "),
            );
            pb.set_message("Copying");
        });
        TerminalCopyReporter { pb, total }
    }

    fn hash_reporter(&self, _source: &Path, total: u64) -> TerminalHashReporter {
        let pb = self.add_managed_bar(total, |pb| {
            pb.set_style(
                ProgressStyle::default_bar()
                    .template(
                        "{spinner:.yellow} {msg} [{bar:40.yellow/blue}] {pos}/{len} ({percent}%)",
                    )
                    .unwrap()
                    .progress_chars("=> "),
            );
            pb.set_message("Hashing");
        });
        TerminalHashReporter { pb, total }
    }

    fn insert_reporter(&self, _source: &Path, total: u64) -> TerminalInsertReporter {
        let pb = self.add_managed_bar(total, |pb| {
            pb.set_style(
                ProgressStyle::default_bar()
                    .template(
                        "{spinner:.magenta} {msg} [{bar:40.magenta/blue}] {pos}/{len} ({percent}%)",
                    )
                    .unwrap()
                    .progress_chars("=> "),
            );
            pb.set_message("Inserting");
        });
        TerminalInsertReporter { pb, total }
    }

    fn add_summary_reporter(&self, _vault_root: &Path) -> TerminalAddSummaryReporter {
        // Hidden bar acts as a synchronized text output carrier.
        let pb = self.add_managed_hidden(|pb| {
            pb.set_style(ProgressStyle::default_bar().template("").unwrap());
        });
        TerminalAddSummaryReporter { pb }
    }

    fn recheck_reporter(&self, total: u64) -> TerminalRecheckReporter {
        let pb = self.add_managed_bar(total, |pb| {
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{prefix:.bold.cyan} [{bar:40}] {pos}/{len}  {msg}")
                    .unwrap()
                    .progress_chars("=> "),
            );
            pb.set_prefix("Checking ");
        });
        TerminalRecheckReporter { pb, total }
    }

    fn update_hash_reporter(&self, total: u64) -> TerminalUpdateHashReporter {
        let pb = self.add_managed_bar(total, |pb| {
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{prefix:.bold.cyan} [{bar:40}] {pos}/{len}  {msg}")
                    .unwrap()
                    .progress_chars("=> "),
            );
            pb.set_prefix("Hashing  ");
        });
        TerminalUpdateHashReporter {
            pb,
            total,
            matches: Mutex::new(Vec::new()),
        }
    }

    fn update_apply_reporter(&self, total: u64) -> TerminalUpdateApplyReporter {
        let pb = self.add_managed_bar(total, |pb| {
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{prefix:.bold.cyan} [{bar:40}] {pos}/{len}  {msg}")
                    .unwrap()
                    .progress_chars("=> "),
            );
            pb.set_prefix("Updating ");
        });
        TerminalUpdateApplyReporter { pb, total }
    }

    fn verify_reporter(&self, total: u64) -> TerminalVerifyReporter {
        let pb = self.add_managed_bar(total, |pb| {
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{prefix:.bold.green} [{bar:40}] {pos}/{len}  {msg}")
                    .unwrap()
                    .progress_chars("=> "),
            );
            pb.set_prefix("Verifying");
        });
        TerminalVerifyReporter { pb }
    }

    fn background_hash_reporter(&self, total: u64) -> TerminalBackgroundHashReporter {
        let pb = self.add_managed_bar(total, |pb| {
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{prefix:.bold.green} [{bar:40}] {pos}/{len}  {msg}")
                    .unwrap()
                    .progress_chars("=> "),
            );
            pb.set_prefix("Hashing");
        });
        TerminalBackgroundHashReporter { pb }
    }
}

impl Default for TerminalReporterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Interactor
// ─────────────────────────────────────────────────────────────────────────────

/// Terminal interactor that suspends the `MultiProgress` while prompting.
///
/// This prevents the progress-bar redraw loop from overwriting the
/// confirmation prompt text on stderr.
///
/// Holds a cloned `MultiProgress` directly rather than the full reporter builder
/// to reduce coupling.
pub struct SuspendingInteractor {
    multi_progress: MultiProgress,
}

impl SuspendingInteractor {
    /// Create a new interactor that suspends the given `MultiProgress` during prompts.
    pub fn new(multi_progress: MultiProgress) -> Self {
        Self { multi_progress }
    }
}

impl Interactor for SuspendingInteractor {
    fn confirm(&self, prompt: &str) -> bool {
        self.multi_progress.suspend(|| {
            eprint!("{} [y/N] ", prompt);
            std::io::stderr().flush().unwrap();

            let mut input = String::new();
            if std::io::stdin().read_line(&mut input).is_err() {
                return false;
            }
            eprintln!();
            matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_hidden_progress_bar_is_registered_and_hidden() {
        let builder = TerminalReporterBuilder::new();
        let pb = builder.add_managed_hidden(|pb| {
            pb.set_style(ProgressStyle::default_bar().template("").unwrap());
        });
        assert!(pb.is_hidden());
    }
}
