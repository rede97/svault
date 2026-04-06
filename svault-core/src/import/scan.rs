//! `svault scan` — scan directory and report file status for import pipeline.
//!
//! This module provides the core logic for the scan command, which outputs
//! file status in a pipeable format for the scan -> filter -> import workflow.

use std::path::{Path, PathBuf};

use indicatif::{ProgressBar, ProgressStyle};

use crate::context::VaultContext;
use crate::db::Db;
use crate::pipeline;

/// Status of a scanned file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileScanStatus {
    /// New file - will be imported.
    New,
    /// Duplicate file - already in vault.
    Dup,
    /// Failed to read or compute hash.
    Fail,
}

/// Result of scanning a single file.
#[derive(Debug, Clone)]
pub struct ScanResult {
    /// Relative path from scan root.
    pub rel_path: PathBuf,
    /// File size in bytes.
    pub size: u64,
    /// Modification time in milliseconds.
    pub mtime_ms: i64,
    /// Scan status.
    pub status: FileScanStatus,
}

/// Summary of a scan operation.
#[derive(Debug, Default)]
pub struct ScanSummary {
    /// Total files scanned.
    pub total: usize,
    /// Number of new files.
    pub new: usize,
    /// Number of duplicate files.
    pub dup: usize,
    /// Number of failed files.
    pub failed: usize,
    /// Scan results (optional, for programmatic use).
    pub results: Vec<ScanResult>,
}

/// Options for the scan command.
pub struct ScanOptions {
    /// Source directory to scan.
    pub source: PathBuf,
    /// Show duplicate files in output.
    pub show_dup: bool,
    /// Include results in summary (for programmatic use).
    pub collect_results: bool,
}

/// Run scan and return summary.
///
/// This is the core logic for the scan command. It scans the source directory,
/// computes CRC32C for each file, checks against the database for duplicates,
/// and returns a summary of the results.
///
/// Output is printed to stderr for progress and status information.
pub fn run_scan(opts: ScanOptions, db: Option<&Db>, vault_ctx: Option<&VaultContext>) -> anyhow::Result<ScanSummary> {
    let source_canon = std::fs::canonicalize(&opts.source)
        .unwrap_or_else(|_| opts.source.clone());

    // Get extensions from vault config
    let ext_strings: Vec<String> = vault_ctx.map(|ctx| {
        ctx.config().import.allowed_extensions.clone()
    }).unwrap_or_default();
    let exts: Vec<&str> = ext_strings.iter().map(|s| s.as_str()).collect();

    // Get vault root for path filtering
    let vault_canon = vault_ctx.and_then(|ctx| {
        std::fs::canonicalize(ctx.vault_root()).ok()
    });

    // ========================================================================
    // Stage A+B: Scan + CRC (shared pipeline with import command)
    // ========================================================================
    let scan_rx = pipeline::scan::scan_stream(&source_canon, &exts)?;
    let crc_rx = pipeline::crc::compute_crcs_stream(scan_rx, None);

    // ========================================================================
    // Stage C: Lookup (simplified - just classify as new/dup/fail)
    // ========================================================================
    let mut summary = ScanSummary::default();
    
    // Progress bar
    let scan_bar = ProgressBar::new_spinner();
    scan_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.cyan} {spinner} {pos} files ({per_sec})")
            .unwrap(),
    );
    scan_bar.set_prefix("Scanning");

    for result in crc_rx {
        // Skip vault paths
        if let Some(ref vault_root) = vault_canon {
            if result.file.path.ancestors().any(|p| p == vault_root) {
                continue;
            }
        }

        summary.total += 1;
        scan_bar.inc(1);

        // Get relative path
        let rel_path = result.file.path.strip_prefix(&source_canon)
            .unwrap_or(&result.file.path)
            .to_path_buf();

        // Handle CRC errors
        let crc = match result.crc {
            Ok(c) => c,
            Err(_) => {
                summary.failed += 1;
                if opts.collect_results {
                    summary.results.push(ScanResult {
                        rel_path: rel_path.clone(),
                        size: result.file.size,
                        mtime_ms: result.file.mtime_ms,
                        status: FileScanStatus::Fail,
                    });
                }
                continue;
            }
        };

        // Build CrcEntry
        let entry = pipeline::types::CrcEntry {
            file: pipeline::types::FileEntry {
                path: result.file.path,
                size: result.file.size,
                mtime_ms: result.file.mtime_ms,
            },
            src_path: None,
            crc32c: crc,
            raw_unique_id: result.raw_unique_id,
            precomputed_hash: None,
        };

        // Check duplicate using shared function
        let status = if let Some(ctx) = vault_ctx {
            match super::check_duplicate(&entry, db.unwrap(), ctx.vault_root(), None) {
                pipeline::CheckResult::New => FileScanStatus::New,
                pipeline::CheckResult::Duplicate => FileScanStatus::Dup,
                pipeline::CheckResult::Moved { .. } => FileScanStatus::Dup,
                pipeline::CheckResult::Recover { .. } => FileScanStatus::New,
            }
        } else {
            FileScanStatus::New
        };

        match status {
            FileScanStatus::New => summary.new += 1,
            FileScanStatus::Dup => summary.dup += 1,
            FileScanStatus::Fail => summary.failed += 1,
        }

        if opts.collect_results {
            summary.results.push(ScanResult {
                rel_path: rel_path.clone(),
                size: entry.file.size,
                mtime_ms: entry.file.mtime_ms,
                status,
            });
        }
    }
    scan_bar.finish_and_clear();

    // Output in pipeable format
    if summary.total > 0 {
        print_scan_output(&source_canon, &summary, opts.show_dup);
    }

    Ok(summary)
}

/// Print scan output in pipeable format.
/// Format: SCAN:<source>
///         new:<file>
///         dup:<file>
///         fail:<file>
fn print_scan_output(source: &Path, summary: &ScanSummary, show_dup: bool) {
    let source_display = source.display().to_string();
    
    // Print source line
    println!("SCAN:{}", source_display);

    // If no results collected, return early
    if summary.results.is_empty() {
        return;
    }

    // Print each file on its own line
    for result in &summary.results {
        let prefix = match result.status {
            FileScanStatus::New => "new",
            FileScanStatus::Dup => "dup",
            FileScanStatus::Fail => "fail",
        };

        // Skip dup if show_dup is disabled
        if result.status == FileScanStatus::Dup && !show_dup {
            continue;
        }

        let rel_str = result.rel_path.display().to_string();
        // Escape spaces and colons in paths for safe parsing
        let escaped = rel_str.replace(' ', "\\ ").replace(':', "\\:");
        println!("{}:{}", prefix, escaped);
    }
}

/// Format a scan result as a string (for testing or custom output).
/// Each entry is on its own line for clarity.
pub fn format_scan_line(source: &Path, results: &[(PathBuf, FileScanStatus)], show_dup: bool) -> String {
    let source_display = source.display().to_string();
    let mut lines = vec![format!("SCAN:{}", source_display)];

    for (rel_path, status) in results {
        let prefix = match status {
            FileScanStatus::New => "new",
            FileScanStatus::Dup => "dup",
            FileScanStatus::Fail => "fail",
        };

        if *status == FileScanStatus::Dup && !show_dup {
            continue;
        }

        let rel_str = rel_path.display().to_string();
        let escaped = rel_str.replace(' ', "\\ ").replace(':', "\\:");
        lines.push(format!("{}:{}", prefix, escaped));
    }

    lines.join("\n")
}
