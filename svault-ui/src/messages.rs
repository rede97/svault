//! One-line styled messages for command results.
//!
//! These cover the small, non-progress outputs (init confirmation, single
//! file verification, hardlink upgrades) that don't warrant the full event
//! pipeline. Long-running operations use [`crate::TerminalSink`] instead.

use std::path::Path;

use console::style;
use svault_core::verify::VerifyResult;

/// Print a success line (green ✓) to stdout.
pub fn success(message: &str) {
    println!("{} {}", style("✓").green().bold(), message);
}

/// Print an informational line (cyan →) to stderr.
pub fn info(message: &str) {
    eprintln!("{} {}", style("→").cyan(), message);
}

/// Print a warning line (yellow ⚠) to stderr.
pub fn warn(message: &str) {
    eprintln!("{} {}", style("⚠").yellow().bold(), message);
}

/// Print a labeled action line, e.g. `Verify: Verifying all files in vault`.
pub fn action(label: &str, message: &str) {
    eprintln!("{} {}", style(format!("{}:", label)).bold().cyan(), message);
}

/// Render the result of verifying a single file.
///
/// Returns `true` when the file is intact, `false` on any failure.
pub fn verify_single_result(path: &Path, result: &VerifyResult) -> bool {
    match result {
        VerifyResult::Ok => {
            println!("{} {}", style("✓").green().bold(), path.display());
            true
        }
        VerifyResult::Missing => {
            eprintln!(
                "{} {} - File not found",
                style("✗").red().bold(),
                path.display()
            );
            false
        }
        VerifyResult::SizeMismatch { expected, actual } => {
            eprintln!(
                "{} {} - Size mismatch (expected {}, got {})",
                style("✗").red().bold(),
                path.display(),
                expected,
                actual
            );
            false
        }
        VerifyResult::HashMismatch { algo } => {
            eprintln!(
                "{} {} - Hash mismatch ({})",
                style("✗").red().bold(),
                path.display(),
                algo
            );
            false
        }
        VerifyResult::IoError { message } => {
            eprintln!(
                "{} {} - IO error: {}",
                style("✗").red().bold(),
                path.display(),
                message
            );
            false
        }
        VerifyResult::HashNotAvailable => {
            eprintln!(
                "{} {} - Hash not computed yet",
                style("!").yellow().bold(),
                path.display()
            );
            true // not a corruption
        }
    }
}
