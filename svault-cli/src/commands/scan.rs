//! Scan command - output file status for import pipeline.

use std::path::PathBuf;

use crate::cli::OutputFormat;
use svault_core::context::VaultContext;
use svault_core::import::scan::{run_scan, ScanOptions};

/// Run the scan command.
///
/// Scans a directory and outputs file status in pipeable format:
///   SCAN:<source_path> new:<rel_path> dup:<rel_path> fail:<rel_path>
///
/// Uses shared pipeline stages (scan + crc) with import command for consistency.
///
/// # Output Format
/// Single line per scan with space-separated status:file entries:
/// ```
/// SCAN:/mnt/sdcard new:DCIM/IMG_0001.jpg new:DCIM/IMG_0002.jpg dup:DCIM/IMG_0003.jpg
/// ```
///
/// # Example Usage
/// ```bash
/// # Scan and pipe to import
/// svault scan /mnt/sdcard | svault import /mnt/sdcard --files-from -
///
/// # Scan with duplicate visibility
/// svault scan /mnt/sdcard --show-dup
/// ```
pub fn run(_output: OutputFormat, source: PathBuf, show_dup: bool) -> anyhow::Result<()> {
    // Open vault context for config and duplicate checking
    let vault_ctx = VaultContext::open(None, &std::env::current_dir()?).ok();
    let db = vault_ctx.as_ref().map(|ctx| ctx.db());

    let opts = ScanOptions {
        source,
        show_dup,
        collect_results: true,
    };

    let summary = run_scan(opts, db, vault_ctx.as_ref())?;

    if summary.failed > 0 {
        std::process::exit(1);
    }

    Ok(())
}
