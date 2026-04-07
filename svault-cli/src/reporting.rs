//! CLI reporting implementation.
//!
//! Provides TerminalReporter for human-readable progress output
//! and NoopReporter for JSON mode.

use svault_core::reporting::{CoreEvent, ItemStatus, PhaseKind, Reporter};

/// Terminal reporter that renders progress to stderr.
/// 
/// Human output mode: shows progress bars and status text.
pub struct TerminalReporter;

impl TerminalReporter {
    /// Create a new terminal reporter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for TerminalReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl Reporter for TerminalReporter {
    fn emit(&self, event: CoreEvent) {
        use console::style;
        
        match event {
            CoreEvent::ItemClassified { path, status, .. } => {
                let rel_path = path.file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_else(|| path.to_string_lossy());
                
                match status {
                    ItemStatus::New => {
                        eprintln!("  {} {}", style("Found").green(), style(rel_path));
                    }
                    ItemStatus::Duplicate => {
                        eprintln!("  {} {}", style("Duplicate").yellow(), style(rel_path));
                    }
                    ItemStatus::Recover => {
                        eprintln!("  {} {}", style("Recover").cyan(), style(rel_path));
                    }
                    ItemStatus::MovedInVault => {
                        eprintln!("  {} {}", style("Moved").cyan(), style(rel_path));
                    }
                    ItemStatus::Failed => {
                        eprintln!("  {} {}", style("Error").red(), style(rel_path));
                    }
                }
            }
            CoreEvent::Error { message, path } => {
                if let Some(p) = path {
                    let rel = p.file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_else(|| p.to_string_lossy());
                    eprintln!("  {} {} - {}", style("Error").red(), style(rel), message);
                } else {
                    eprintln!("  {} {}", style("Error").red(), message);
                }
            }
            CoreEvent::Warning { message, path } => {
                if let Some(p) = path {
                    let rel = p.file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_else(|| p.to_string_lossy());
                    eprintln!("  {} {} - {}", style("Warning").yellow(), style(rel), message);
                } else {
                    eprintln!("  {} {}", style("Warning").yellow(), message);
                }
            }
            CoreEvent::PhaseStarted { phase, .. } => {
                let name = match phase {
                    PhaseKind::Scan => "Scanning",
                    PhaseKind::Copy => "Copying",
                    PhaseKind::Fingerprint => "Fingerprinting",
                    PhaseKind::DedupLookup => "Looking up",
                    PhaseKind::Verify => "Verifying",
                    PhaseKind::Insert => "Inserting",
                };
                // Phase start is handled by progress bars in import module
                // This is a hook for future more granular output
                let _ = name;
            }
            CoreEvent::PhaseFinished { phase } => {
                let _ = phase;
                // Phase end is handled by progress bars in import module
            }
            CoreEvent::RunFinished { imported, duplicate, failed, .. } => {
                eprintln!("{} {} file(s) imported",
                    style("Finished:").bold().green(),
                    style(imported).green());
                if duplicate > 0 {
                    eprintln!("         {} duplicate(s) skipped",
                        style(duplicate).yellow());
                }
                if failed > 0 {
                    eprintln!("         {} file(s) failed",
                        style(failed).red());
                }
            }
            // Other events are primarily for GUI or handled internally
            _ => {}
        }
    }
}

/// Create appropriate reporter for output format.
/// 
/// - Human mode: TerminalReporter (shows progress)
/// - JSON mode: NoopReporter (silent, final summary printed separately)
pub fn create_reporter(output: &super::cli::OutputFormat) -> Box<dyn Reporter> {
    match output {
        super::cli::OutputFormat::Human => Box::new(TerminalReporter::new()),
        super::cli::OutputFormat::Json => Box::new(svault_core::reporting::NoopReporter),
    }
}
