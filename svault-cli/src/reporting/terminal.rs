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
//! | `TerminalHashReporter`  | `HashReporter` | update (hash phase) |
//! | `TerminalUpdateApplyReporter` | `UpdateApplyReporter`| update (apply phase)|
//! | `TerminalVerifyReporter`      | `VerifyReporter`     | verify command      |
//! | `TerminalHashReporter` | `HashReporter` | background-hash |
//! | `TerminalHistorySessionsReporter` | `HistorySessionsReporter` | history sessions |
//! | `TerminalHistoryItemsReporter` | `HistoryItemsReporter` | history items |
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
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use svault_core::import::RecheckStatus;
use svault_core::reporting::{
    AddSummaryReporter, CopyItemResult, CopyReporter, HashReporter, HistoryItemsQuery,
    HistoryItemsReporter, HistoryItemsSummary, HistoryItemRow, HistorySessionsQuery,
    HistorySessionsReporter, HistorySessionsSummary, HistorySessionRow, InsertReporter, Interactor,
    ItemStatus, MatchConfidence, RecheckReporter, ReporterBuilder, ScanReporter,
    UpdateApplyReporter, VerifyReporter,
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

/// Format byte size to human-readable string (B, KiB, MiB, GiB, TiB).
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let exp = (bytes as f64).log(1024.0).min((UNITS.len() - 1) as f64) as usize;
    let value = bytes as f64 / 1024f64.powi(exp as i32);
    if exp == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", value, UNITS[exp])
    }
}

impl ScanReporter for TerminalScanReporter {
    fn item(
        &self,
        path: &Path,
        size: u64,
        _mtime_ms: i64,
        status: ItemStatus,
        error: Option<&str>,
    ) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        let size_str = format_bytes(size);

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

        // Update progress bar
        self.pb.inc(1);

        // All status labels use cyan.bold, filename uses normal white, size uses dim
        let label = match status {
            ItemStatus::New => "Found",
            ItemStatus::Duplicate => "Duplicate",
            ItemStatus::MovedInVault => "Moved",
            ItemStatus::Recover => "Recover",
            ItemStatus::Failed => "Error",
        };
        let label_style = match status {
            ItemStatus::New => style(label).green().bold(),
            ItemStatus::Duplicate => style(label).yellow().bold(),
            ItemStatus::MovedInVault => style(label).color256(208).bold(),
            ItemStatus::Recover => style(label).cyan().bold(),
            ItemStatus::Failed => style(label).red().bold(),
        };

        if let Some(err) = error {
            self.println(format!(
                "  {} {} ({}) - {}",
                label_style,
                name,
                size_str,
                style(err).red()
            ));
        } else {
            self.println(format!("  {} {} ({})", label_style, name, size_str));
        }
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
        // Nothing to import - all files were duplicates, moved, or failed
        if new_count == 0 {
            let mut output = String::new();
            output.push_str(&format!(
                "{} Scanned {} files from {}\n",
                style("Finished:").bold().green(),
                style(total_scanned).green(),
                style(source.display()).color256(244),
            ));
            output.push('\n');
            if duplicate_count > 0 {
                output.push_str(&format!(
                    "All {} files matched cache (no new files detected).\n",
                    style(duplicate_count).cyan()
                ));
            } else if moved_count > 0 {
                output.push_str(&format!(
                    "Found {} moved files. Run `svault update` to fix paths.\n",
                    style(moved_count).cyan()
                ));
            } else {
                output.push_str("No files to import.\n");
            }
            self.println(output);
            return;
        }

        let mut output = String::new();
        output.push_str(&format!(
            "{} Scanned {} files from {}\n",
            style("Finished:").bold().green(),
            style(total_scanned).green(),
            style(source.display()).color256(244),
        ));
        output.push('\n');
        output.push_str(&format!("{}\n", style("Pre-flight:").bold()));
        output.push_str(&format!(
            "  {}  {}\n",
            style(format!("Likely new:       {:>6}", new_count)).green(),
            style("will be imported")
        ));
        output.push_str(&format!(
            "  {}  {}\n",
            style(format!("Likely duplicate: {:>6}", duplicate_count)).yellow(),
            style("already in vault (cache hit)")
        ));
        if moved_count > 0 {
            output.push_str(&format!(
                "  {}  {}\n",
                style(format!("Moved in vault:   {:>6}", moved_count)).cyan(),
                style("path will be updated")
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
/// Tracks currently copying files across multiple threads for display.
pub struct TerminalCopyReporter {
    pb: ProgressBar,
    total: u64,
    /// Source root path (for computing relative paths)
    source: PathBuf,
    /// Vault root path (for computing relative paths)
    vault_root: PathBuf,
    /// Currently copying file names (from multiple threads)
    active_files: Mutex<Vec<String>>,
}

impl TerminalCopyReporter {
    /// Print a line through the progress bar for synchronized output.
    fn println<S: AsRef<str>>(&self, s: S) {
        self.pb.println(s);
    }

    /// Update the progress bar message with current active files.
    /// Uses wide_msg template which auto-fills remaining space and truncates.
    fn update_message(&self) {
        let files = self.active_files.lock().unwrap();
        // Join all files with comma, wide_msg will auto-truncate if too long
        let msg = files.join(", ");
        self.pb.set_message(msg);
    }

    /// Compute relative path from source root.
    fn relative_to_source(&self, abs_path: &Path) -> String {
        abs_path
            .strip_prefix(&self.source)
            .unwrap_or(abs_path)
            .display()
            .to_string()
    }

    /// Compute relative path from vault root.
    fn relative_to_vault(&self, abs_path: &Path) -> String {
        abs_path
            .strip_prefix(&self.vault_root)
            .unwrap_or(abs_path)
            .display()
            .to_string()
    }
}

impl CopyReporter for TerminalCopyReporter {
    fn item_started(&self, src_abs: &Path, dest_abs: &Path, bytes_total: u64) {
        let name = src_abs
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| src_abs.display().to_string());
        // Compute relative paths and size
        let src_rel = self.relative_to_source(src_abs);
        let dest_rel = self.relative_to_vault(dest_abs);
        let size_str = format_bytes(bytes_total);
        // Print start message immediately
        // Copying: cyan.bold, src: normal white, size: normal white, -> dest: dim
        self.println(format!(
            "  {} {} {} -> {}",
            style("Copying").green().bold(),
            src_rel,
            format!("({})", size_str),
            style(format!("{}", dest_rel)).color256(244)
        ));
        // Add to active files for progress bar
        {
            let mut files = self.active_files.lock().unwrap();
            files.push(name);
        }
        self.update_message();
    }

    fn item_finished(&self, src_abs: &Path, _dest_abs: &Path, result: &CopyItemResult) {
        // Remove from active files and update progress
        let name = src_abs
            .file_name()
            .map(|n: &std::ffi::OsStr| n.to_string_lossy().to_string())
            .unwrap_or_else(|| src_abs.display().to_string());
        {
            let mut files = self.active_files.lock().unwrap();
            files.retain(|f| f != &name);
        }
        self.update_message();
        self.pb.inc(1);

        // Report failure if any
        if let CopyItemResult::Failed { message } = result {
            self.println(format!(
                "{} {}: {}",
                style("Error:").red().bold(),
                src_abs.display(),
                message
            ));
        }
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
    matches: Mutex<Vec<(String, String, MatchConfidence)>>,
    /// Bytes processed so far
    bytes_processed: AtomicU64,
    /// Start time for speed calculation
    start_time: Instant,
}

impl TerminalHashReporter {
    /// Print a line through the progress bar for synchronized output.
    fn println<S: AsRef<str>>(&self, s: S) {
        self.pb.println(s);
    }
}

impl HashReporter for TerminalHashReporter {
    fn item_started(&self, _abs_path: &Path, _bytes_total: u64) {}

    fn item_finished(&self, abs_path: &Path, error: Option<&str>, bytes_total: u64) {
        // Print error if any
        if let Some(err) = error {
            let name = abs_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| abs_path.display().to_string());
            self.println(format!("  {} {}: {}", style("Error").red(), name, err));
        }

        // Update bytes processed
        let processed = self
            .bytes_processed
            .fetch_add(bytes_total, Ordering::Relaxed)
            + bytes_total;

        // Calculate speed (bytes per second)
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let speed = if elapsed > 0.0 {
            processed as f64 / elapsed
        } else {
            0.0
        };

        // Update progress bar message with bytes info
        let msg = format!("{} /s", format_bytes(speed as u64));
        self.pb.set_message(msg);

        // Increment progress (file count)
        self.pb.inc(1);
    }

    fn matched(&self, old_path: &str, new_path: &str, confidence: MatchConfidence) {
        self.matches
            .lock()
            .unwrap()
            .push((old_path.to_string(), new_path.to_string(), confidence));
    }

    fn finish(&self) {
        let matches = self.matches.lock().unwrap();

        // Build output
        let mut output = String::new();
        output.push_str(&format!(
            "✓ Fingerprint complete ({}/{})\n",
            self.pb.position(),
            self.total
        ));

        // Print matches if any
        if !matches.is_empty() {
            output.push('\n');
            output.push_str(&format!("{}\n", style("Matches found:").bold()));
            for (old, new, conf) in matches.iter() {
                let label = match conf {
                    MatchConfidence::Definitive => style("[Definitive]").green(),
                    MatchConfidence::Fast => style("[Fast match]").yellow(),
                };
                output.push_str(&format!(
                    "  {} {} -> {}\n",
                    label,
                    style(old),
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
                    "    {} match(es) with SHA-256 (definitive)\n",
                    style(definitive).green().bold()
                ));
            }
            if fast > 0 {
                output.push_str(&format!(
                    "    {} match(es) with XXH3-128 only (fast)\n",
                    style(fast).yellow().bold()
                ));
            }
        }

        self.println(output);
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
            output.push_str(&format!(
                "  Manifest: {}\n",
                style(p.display()).italic().bold()
            ));
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

    fn item_started(&self, _path: &Path) {
        // Progress bar is updated in item_finished
    }

    fn item_finished(&self, path: &Path, result: &svault_core::verify::VerifyResult) {
        self.pb.inc(1);

        // Output failure details
        match result {
            svault_core::verify::VerifyResult::Ok => {
                // Success - show minimal output or nothing
                // Could optionally show verified files with verbose flag
            }
            svault_core::verify::VerifyResult::Missing => {
                self.pb.println(format!(
                    "  {} {}",
                    style("Missing").red(),
                    style(path.display()).red(),
                ));
            }
            svault_core::verify::VerifyResult::SizeMismatch { expected, actual } => {
                self.pb.println(format!(
                    "  {} {} (expected {} bytes, actual {} bytes)",
                    style("Size mismatch").red(),
                    style(path.display()),
                    expected,
                    actual,
                ));
            }
            svault_core::verify::VerifyResult::HashMismatch { algo } => {
                self.pb.println(format!(
                    "  {} {} (hash algorithm: {:?})",
                    style("Hash mismatch").red(),
                    style(path.display()),
                    algo,
                ));
            }
            svault_core::verify::VerifyResult::IoError(e) => {
                self.pb.println(format!(
                    "  {} {}: {}",
                    style("IO error").red(),
                    style(path.display()),
                    e,
                ));
            }
            svault_core::verify::VerifyResult::HashNotAvailable => {
                // Silent - hash not available is not a failure
            }
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
// History sessions reporter
// ─────────────────────────────────────────────────────────────────────────────

/// Terminal reporter for the `history sessions` command.
pub struct TerminalHistorySessionsReporter {
    pb: ProgressBar,
}

impl TerminalHistorySessionsReporter {
    /// Print a line through the progress bar for synchronized output.
    fn println<S: AsRef<str>>(&self, s: S) {
        self.pb.println(s);
    }
}

impl HistorySessionsReporter for TerminalHistorySessionsReporter {
    fn started(&self, _query: &HistorySessionsQuery) {
        // No progress bar for history - we print as we go
    }

    fn item(&self, session: &HistorySessionRow) {
        let status = if session.failed > 0 {
            format!("{} added, {} dup, {} failed", 
                style(session.added).green(),
                session.duplicate,
                style(session.failed).red())
        } else {
            format!("{} added, {} dup", 
                style(session.added).green(),
                session.duplicate)
        };
        
        // Format timestamp
        let datetime = chrono::DateTime::from_timestamp_millis(session.started_at_ms)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "unknown".to_string());
        
        // Session type label
        let type_label = match session.session_type.as_str() {
            "import" => style("import").cyan(),
            "add" => style("add").green(),
            "update" => style("update").yellow(),
            "recheck" => style("recheck").blue(),
            _ => style("unknown").dim(),
        };
        
        self.println(format!(
            "  [{:9}] {} {} {} {} [{}]",
            type_label,
            style(datetime).dim(),
            style(&session.session_id[..session.session_id.len().min(8)]).cyan(),
            style(&session.source).yellow(),
            status,
            session.total_files
        ));
    }

    fn finish(&self, summary: &HistorySessionsSummary) {
        if summary.has_more {
            self.println(format!(
                "\n  {} (showing {} of {})",
                style("... more sessions available").dim(),
                summary.returned,
                summary.total
            ));
        }
    }
}

impl Drop for TerminalHistorySessionsReporter {
    fn drop(&mut self) {
        self.pb.finish_and_clear();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// History items reporter
// ─────────────────────────────────────────────────────────────────────────────

/// Terminal reporter for the `history items` command.
pub struct TerminalHistoryItemsReporter {
    pb: ProgressBar,
}

impl TerminalHistoryItemsReporter {
    /// Print a line through the progress bar for synchronized output.
    fn println<S: AsRef<str>>(&self, s: S) {
        self.pb.println(s);
    }
}

impl HistoryItemsReporter for TerminalHistoryItemsReporter {
    fn started(&self, session_id: &str, _query: &HistoryItemsQuery) {
        self.println(format!("  Session: {}", style(session_id).cyan().bold()));
    }

    fn item(&self, item: &HistoryItemRow) {
        let status_style = match item.status.as_str() {
            "added" => style("added").green(),
            "duplicate" => style("duplicate").dim(),
            "failed" => style("failed").red(),
            _ => style(item.status.as_str()),
        };
        
        self.println(format!(
            "  {} {} -> {}",
            status_style,
            style(&item.source_path).yellow(),
            style(&item.vault_path).dim()
        ));
    }

    fn finish(&self, summary: &HistoryItemsSummary) {
        if summary.has_more {
            self.println(format!(
                "\n  {} (showing {} of {} items)",
                style("... more items available").dim(),
                summary.returned,
                summary.total
            ));
        }
    }
}

impl Drop for TerminalHistoryItemsReporter {
    fn drop(&mut self) {
        self.pb.finish_and_clear();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Background hash reporter
// ─────────────────────────────────────────────────────────────────────────────

/// Terminal reporter for the `background-hash` command.
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
            style("will be added")
        ));
        if duplicate_count > 0 {
            output.push_str(&format!(
                "  {}  {}\n",
                style(format!("Likely duplicate: {:>6}", duplicate_count)).yellow(),
                style("already in vault")
            ));
        }
        if moved_count > 0 {
            output.push_str(&format!(
                "  {}  {}\n",
                style(format!("Moved:            {:>6}", moved_count)).cyan(),
                style("vault-internal move detected")
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
                "    {} -> {}\n",
                style(old).color256(244),
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
                "    {} -> {}\n",
                style(old).color256(244),
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

    fn item_started(&self, _src_path: &std::path::Path, _vault_path: &std::path::Path) {
        // Progress bar is updated in item_finished
    }

    fn item_finished(
        &self,
        src_path: &std::path::Path,
        _vault_path: &std::path::Path,
        status: &RecheckStatus,
    ) {
        self.pb.inc(1);

        // Output failure details
        match status {
            RecheckStatus::Ok => {
                // Success - silent
            }
            RecheckStatus::SourceModified => {
                self.println(format!(
                    "  {} {}",
                    style("Source modified").yellow(),
                    style(src_path.display()),
                ));
            }
            RecheckStatus::VaultCorrupted => {
                self.println(format!(
                    "  {} {}",
                    style("Vault corrupted").red(),
                    style(src_path.display()),
                ));
            }
            RecheckStatus::BothDiverged => {
                self.println(format!(
                    "  {} {}",
                    style("Both diverged").red().bold(),
                    style(src_path.display()),
                ));
            }
            RecheckStatus::SourceDeleted => {
                self.println(format!(
                    "  {} {}",
                    style("Source deleted").yellow(),
                    style(src_path.display()),
                ));
            }
            RecheckStatus::VaultDeleted => {
                self.println(format!(
                    "  {} {}",
                    style("Vault deleted").red(),
                    style(src_path.display()),
                ));
            }
            RecheckStatus::Error(e) => {
                self.println(format!(
                    "  {} {}: {}",
                    style("Error").red().bold(),
                    style(src_path.display()),
                    e,
                ));
            }
        }
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
        output.push_str(&format!("{}\n", style("Results:").bold()));
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
            style(report_path.display()).italic().bold()
        ));
        self.println(output);
    }
}

impl Drop for TerminalRecheckReporter {
    fn drop(&mut self) {
        self.pb.finish_and_clear();
    }
}

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
    type UpdateApply = TerminalUpdateApplyReporter;
    type Verify = TerminalVerifyReporter;
    type HistorySessions = TerminalHistorySessionsReporter;
    type HistoryItems = TerminalHistoryItemsReporter;

    fn scan_reporter(&self, source: &Path) -> TerminalScanReporter {
        let pb = self.add_managed_spinner(|pb| {
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template(
                        "{spinner:.cyan} {prefix:.cyan.bold} {msg} {pos:>7} files ({per_sec})",
                    )
                    .unwrap()
                    .tick_chars(TICK_CHARS),
            );
            pb.set_prefix("Scanning");
            let source_display = source.display().to_string();
            // Truncate long paths for display
            let msg = if source_display.len() > 40 {
                format!("...{}...", &source_display[source_display.len() - 37..])
            } else {
                source_display
            };
            pb.set_message(style(msg).color256(244).to_string());
        });
        TerminalScanReporter {
            pb,
            counters: Mutex::new(ScanCounters::default()),
        }
    }

    fn copy_reporter(&self, source: &Path, vault_root: &Path, total: u64) -> TerminalCopyReporter {
        let pb = self.add_managed_bar(total, |pb| {
            pb.set_style(
                ProgressStyle::default_bar()
                    .template(
                        "  {prefix:.cyan.bold} [{bar:40}] {pos}/{len} ({percent}%): {wide_msg}",
                    )
                    .unwrap()
                    .progress_chars("=> "),
            );
            pb.set_prefix("Copying");
            pb.set_message("");
        });
        TerminalCopyReporter {
            pb,
            total,
            source: source.to_path_buf(),
            vault_root: vault_root.to_path_buf(),
            active_files: Mutex::new(Vec::new()),
        }
    }

    fn hash_reporter(&self, _source: &Path, total: u64) -> TerminalHashReporter {
        let pb = self.add_managed_bar(total, |pb| {
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("  {prefix:.cyan.bold} [{bar:40}] {pos}/{len} ({percent}%) {msg}")
                    .unwrap()
                    .progress_chars("=> "),
            );
            pb.set_prefix("Hashing");
        });
        TerminalHashReporter {
            pb,
            total,
            matches: Mutex::new(Vec::new()),
            bytes_processed: AtomicU64::new(0),
            start_time: Instant::now(),
        }
    }

    fn insert_reporter(&self, _source: &Path, total: u64) -> TerminalInsertReporter {
        let pb = self.add_managed_bar(total, |pb| {
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("  {prefix:.cyan.bold} [{bar:40}] {pos}/{len} ({percent}%)")
                    .unwrap()
                    .progress_chars("=> "),
            );
            pb.set_prefix("Inserting");
            pb.set_message("");
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
                    .template("  {prefix:.cyan.bold} [{bar:40}] {pos}/{len}  {msg}")
                    .unwrap()
                    .progress_chars("=> "),
            );
            pb.set_prefix("Checking");
        });
        TerminalRecheckReporter { pb, total }
    }

    fn update_hash_reporter(&self, _source: &Path, total: u64) -> TerminalHashReporter {
        let pb = self.add_managed_bar(total, |pb| {
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("  {prefix:.cyan.bold} [{bar:40}] {pos}/{len} ({percent}%) {msg}")
                    .unwrap()
                    .progress_chars("=> "),
            );
            pb.set_prefix("Hashing");
        });
        TerminalHashReporter {
            pb,
            total,
            matches: Mutex::new(Vec::new()),
            bytes_processed: AtomicU64::new(0),
            start_time: Instant::now(),
        }
    }

    fn update_apply_reporter(&self, total: u64) -> TerminalUpdateApplyReporter {
        let pb = self.add_managed_bar(total, |pb| {
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("  {prefix:.cyan.bold} [{bar:40}] {pos}/{len}  {msg}")
                    .unwrap()
                    .progress_chars("=> "),
            );
            pb.set_prefix("Updating");
        });
        TerminalUpdateApplyReporter { pb, total }
    }

    fn verify_reporter(&self, total: u64) -> TerminalVerifyReporter {
        let pb = self.add_managed_bar(total, |pb| {
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("  {prefix:.cyan.bold} [{bar:40}] {pos}/{len}  {msg}")
                    .unwrap()
                    .progress_chars("=> "),
            );
            pb.set_prefix("Verifying");
        });
        TerminalVerifyReporter { pb }
    }

    fn history_sessions_reporter(&self, _query: &HistorySessionsQuery) -> TerminalHistorySessionsReporter {
        // Hidden bar acts as a synchronized text output carrier
        let pb = self.add_managed_hidden(|pb| {
            pb.set_style(ProgressStyle::default_bar().template("").unwrap());
        });
        TerminalHistorySessionsReporter { pb }
    }

    fn history_items_reporter(&self, _session_id: &str, _query: &HistoryItemsQuery) -> TerminalHistoryItemsReporter {
        // Hidden bar acts as a synchronized text output carrier
        let pb = self.add_managed_hidden(|pb| {
            pb.set_style(ProgressStyle::default_bar().template("").unwrap());
        });
        TerminalHistoryItemsReporter { pb }
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
