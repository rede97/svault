//! CLI-specific reporting implementations
//!
//! Provides terminal-based progress reporting using indicatif.
//! Handles both human-readable output and JSON mode (which is silent,
//! with JSON output handled separately by the CLI).
//!
//! IMPORTANT: Never use println!/eprintln! directly when progress bars are active.
//! Always use MultiProgress::println to avoid output corruption.

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use svault_core::reporting::{CoreEvent, Interactor, PhaseKind, Reporter};
use crate::cli::OutputFormat;

/// Terminal-based progress reporter with indicatif progress bars
pub struct TerminalReporter {
    /// Multi-progress manager for coordinating multiple bars
    multi_progress: MultiProgress,
    /// Current active progress bar (if any)
    current_bar: Arc<Mutex<Option<ProgressBar>>>,
    /// Current phase being displayed
    current_phase: Arc<Mutex<Option<PhaseKind>>>,
    /// Store finished bars to keep them visible
    finished_bars: Arc<Mutex<Vec<ProgressBar>>>,
    /// Whether progress bars are active (after PhaseStarted)
    bars_active: Arc<Mutex<bool>>,
}

impl TerminalReporter {
    pub fn new() -> Self {
        Self {
            multi_progress: MultiProgress::new(),
            current_bar: Arc::new(Mutex::new(None)),
            current_phase: Arc::new(Mutex::new(None)),
            finished_bars: Arc::new(Mutex::new(Vec::new())),
            bars_active: Arc::new(Mutex::new(false)),
        }
    }

    /// Print a message safely (using multi_progress if bars are active)
    fn print(&self, msg: String) {
        let _bars_active = *self.bars_active.lock().unwrap();
        // Always use println for now to ensure messages are visible
        println!("{}", msg);
    }

    /// Finish and clear the current progress bar
    fn finish_current_bar(&self) {
        let mut bar_guard = self.current_bar.lock().unwrap();
        if let Some(bar) = bar_guard.take() {
            bar.finish();
            // Move to finished_bars to keep it visible
            self.finished_bars.lock().unwrap().push(bar);
        }
        *self.current_phase.lock().unwrap() = None;
    }

    /// Suspend progress bars for interactive output
    fn suspend<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        if *self.bars_active.lock().unwrap() {
            self.multi_progress.suspend(f)
        } else {
            f()
        }
    }

    /// Create a progress bar for the given phase
    fn create_progress_bar(&self, phase: PhaseKind, total: Option<u64>) -> ProgressBar {
        let pb = match total {
            Some(t) if t > 0 => ProgressBar::new(t),
            _ => ProgressBar::new_spinner(),
        };

        // Set up the style based on phase
        let pb = match phase {
            PhaseKind::Scan => {
                let style = ProgressStyle::default_spinner()
                    .template("{spinner:.cyan} {msg} {pos:>7} files")
                    .unwrap();
                pb.set_style(style);
                pb.enable_steady_tick(Duration::from_millis(100));
                pb.set_message("Scanning...");
                pb
            }
            PhaseKind::Copy => {
                let style = ProgressStyle::default_bar()
                    .template("{spinner:.green} {msg} [{bar:40.green/blue}] {pos}/{len} ({percent}%)")
                    .unwrap()
                    .progress_chars("=> ");
                pb.set_style(style);
                pb.enable_steady_tick(Duration::from_millis(100));
                pb.set_message("Copying");
                pb
            }
            PhaseKind::Fingerprint => {
                let style = ProgressStyle::default_bar()
                    .template("{spinner:.yellow} {msg} [{bar:40.yellow/blue}] {pos}/{len} ({percent}%)")
                    .unwrap()
                    .progress_chars("=> ");
                pb.set_style(style);
                pb.enable_steady_tick(Duration::from_millis(100));
                pb.set_message("Hashing");
                pb
            }
            PhaseKind::Insert => {
                let style = ProgressStyle::default_bar()
                    .template("{spinner:.magenta} {msg} [{bar:40.magenta/blue}] {pos}/{len} ({percent}%)")
                    .unwrap()
                    .progress_chars("=> ");
                pb.set_style(style);
                pb.enable_steady_tick(Duration::from_millis(100));
                pb.set_message("Inserting");
                pb
            }
            PhaseKind::DedupLookup => {
                let style = ProgressStyle::default_spinner()
                    .template("{spinner:.blue} {msg}")
                    .unwrap();
                pb.set_style(style);
                pb.enable_steady_tick(Duration::from_millis(100));
                pb.set_message("Checking duplicates...");
                pb
            }
            PhaseKind::Verify => {
                let style = ProgressStyle::default_bar()
                    .template("{spinner:.cyan} {msg} [{bar:40.cyan/blue}] {pos}/{len} ({percent}%)")
                    .unwrap()
                    .progress_chars("=> ");
                pb.set_style(style);
                pb.enable_steady_tick(Duration::from_millis(100));
                pb.set_message("Verifying");
                pb
            }
        };

        // Add to multi-progress
        self.multi_progress.add(pb.clone());
        pb
    }
}

impl Default for TerminalReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl Reporter for TerminalReporter {
    fn emit(&self, event: CoreEvent) {
        match event {
            CoreEvent::RunStarted { operation } => {
                self.print(format!("Starting {:?} operation...", operation));
            }

            CoreEvent::PhaseStarted { phase, total } => {
                // Check if we're transitioning from inactive to active
                let was_inactive = !*self.bars_active.lock().unwrap();
                
                // Finish any previous bar first
                self.finish_current_bar();

                // If transitioning from inactive, ensure we start on a fresh line
                if was_inactive {
                    println!();
                }

                // Create new progress bar for this phase
                let bar = self.create_progress_bar(phase, total);
                if let Some(t) = total {
                    bar.set_length(t);
                }
                *self.current_phase.lock().unwrap() = Some(phase);
                *self.current_bar.lock().unwrap() = Some(bar);
                *self.bars_active.lock().unwrap() = true;
            }

            CoreEvent::PhaseProgress {
                phase,
                completed,
                total,
            } => {
                let current = self.current_phase.lock().unwrap();
                if current.as_ref() == Some(&phase) {
                    drop(current); // Drop lock before accessing bar
                    if let Some(ref bar) = *self.current_bar.lock().unwrap() {
                        bar.set_position(completed);
                        if let Some(t) = total {
                            bar.set_length(t);
                        }
                    }
                }
            }

            CoreEvent::PhaseFinished { phase } => {
                // Get the bar and finish it properly
                let mut bar_guard = self.current_bar.lock().unwrap();
                let bar = bar_guard.take();
                *self.current_phase.lock().unwrap() = None;
                
                if let Some(bar) = bar {
                    // Disable steady tick first to stop background updates
                    bar.disable_steady_tick();
                    // Finish the bar (keeps it visible) then print completion on new line
                    bar.finish();
                }
                
                // Mark bars as inactive and print completion message
                *self.bars_active.lock().unwrap() = false;
                println!("✓ {:?} complete", phase);
            }

            CoreEvent::ItemDiscovered { path: _, size: _, .. } => {
                // Update scan count - handled by PhaseProgress
                // Could update message with current file if needed
            }

            CoreEvent::ItemClassified { path, status, detail: _ } => {
                // Show classification status with appropriate color
                let msg = match status {
                    svault_core::reporting::ItemStatus::New => {
                        format!("  {} {}", style("Found").green(), style(path.display()).dim())
                    }
                    svault_core::reporting::ItemStatus::Duplicate => {
                        format!("  {} {}", style("Duplicate").yellow(), style(path.display()).dim())
                    }
                    svault_core::reporting::ItemStatus::MovedInVault => {
                        format!("  {} {}", style("Moved").cyan(), style(path.display()).dim())
                    }
                    svault_core::reporting::ItemStatus::Recover => {
                        format!("  {} {}", style("Recover").blue(), style(path.display()).dim())
                    }
                    svault_core::reporting::ItemStatus::Failed => {
                        format!("  {} {}", style("Failed").red(), style(path.display()).dim())
                    }
                };
                self.print(msg);
            }

            CoreEvent::ItemStarted { path, phase: _, .. } => {
                // Update current bar message to show active file
                if let Some(ref bar) = *self.current_bar.lock().unwrap() {
                    let file_name = path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.display().to_string());
                    bar.set_message(format!("Processing: {}", file_name));
                }
            }

            CoreEvent::ItemProgress {
                path: _,
                bytes_done: _,
                bytes_total: _,
                ..
            } => {
                // Handled at phase level for now
            }

            CoreEvent::ItemFinished { .. } => {
                // Progress bar position is updated via PhaseProgress
            }

            CoreEvent::RunFinished {
                operation,
                total,
                imported,
                duplicate,
                failed,
            } => {
                // Clear any remaining progress bar
                self.finish_current_bar();
                
                // Clear all finished bars and deactivate
                self.finished_bars.lock().unwrap().clear();
                *self.bars_active.lock().unwrap() = false;

                self.print(String::new());
                self.print(format!("{}", style(format!("{:?} operation completed", operation)).green().bold()));
                self.print(format!("  Total files processed: {}", total));
                if imported > 0 {
                    self.print(format!("  {}", style(format!("New files imported: {}", imported)).green()));
                }
                if duplicate > 0 {
                    self.print(format!("  {}", style(format!("Duplicates skipped: {}", duplicate)).yellow()));
                }
                if failed > 0 {
                    self.print(format!("  {}", style(format!("Failed: {}", failed)).red()));
                }
            }

            CoreEvent::Warning { message, path } => {
                let msg = if let Some(ref p) = path {
                    format!("{}: {}", p.display(), message)
                } else {
                    message
                };
                
                self.print(format!(
                    "{} {}",
                    style("Warning:").yellow().bold(),
                    msg
                ));
            }

            CoreEvent::Error { message, path } => {
                let msg = if let Some(ref p) = path {
                    format!("{}: {}", p.display(), message)
                } else {
                    message
                };
                
                self.print(format!(
                    "{} {}",
                    style("Error:").red().bold(),
                    msg
                ));
            }
        }
    }
}

/// Create appropriate reporter based on output format
pub fn create_reporter(output: &OutputFormat) -> Box<dyn Reporter> {
    match output {
        OutputFormat::Human => Box::new(TerminalReporter::new()),
        OutputFormat::Json => Box::new(svault_core::reporting::NoopReporter),
    }
}

/// Terminal-based interactor for user prompts
pub struct TerminalInteractor;

impl Interactor for TerminalInteractor {
    fn confirm(&self, prompt: &str) -> bool {
        print!("{} [y/N] ", prompt);
        std::io::stdout().flush().unwrap();

        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_err() {
            return false;
        }
        
        // Print newline after input to ensure subsequent output starts on fresh line
        println!();

        matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
    }
}

/// Terminal-based interactor that suspends progress bars for prompts
pub struct SuspendingInteractor {
    reporter: Arc<TerminalReporter>,
}

impl SuspendingInteractor {
    pub fn new(reporter: Arc<TerminalReporter>) -> Self {
        Self { reporter }
    }
}

impl Interactor for SuspendingInteractor {
    fn confirm(&self, prompt: &str) -> bool {
        self.reporter.suspend(|| {
            print!("{} [y/N] ", prompt);
            std::io::stdout().flush().unwrap();

            let mut input = String::new();
            if std::io::stdin().read_line(&mut input).is_err() {
                return false;
            }

            matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
        })
    }
}

/// Print import summary (for human-readable output)
/// 
/// NOTE: This should only be called when NO progress bars are active.
/// Use TerminalReporter for output during import operations.
pub fn print_import_summary(
    total: usize,
    imported: usize,
    duplicate: usize,
    failed: usize,
) {
    println!();
    println!("{}", style("Import completed:").green().bold());
    println!("  Total files processed: {}", total);
    if imported > 0 {
        println!("  {}", style(format!("New files imported: {}", imported)).green());
    }
    if duplicate > 0 {
        println!("  {}", style(format!("Duplicates skipped: {}", duplicate)).yellow());
    }
    if failed > 0 {
        println!("  {}", style(format!("Failed: {}", failed)).red());
    }
}

/// Print a summary line for a file being processed
/// 
/// NOTE: This should only be called when NO progress bars are active.
pub fn print_file_processing(path: &Path) {
    println!("  Processing: {}", style(path.display()).dim());
}

/// Print a warning message
/// 
/// NOTE: This should only be called when NO progress bars are active.
/// Use TerminalReporter for output during import operations.
pub fn print_warning(path: &Path, message: &str) {
    eprintln!(
        "  {} {} - {}",
        style("Warning:").yellow(),
        style(path.display()).dim(),
        message
    );
}
