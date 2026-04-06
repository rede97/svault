//! Scan command - output file status for import pipeline.

use std::path::PathBuf;

use crate::cli::OutputFormat;
use svault_core::context::VaultContext;
use svault_core::import::check_duplicate;
use svault_core::pipeline;

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
    let source_canon = std::fs::canonicalize(&source).unwrap_or_else(|_| source.clone());
    
    // Get extensions from vault config (same as import)
    let ext_strings: Vec<String> = if let Ok(ctx) = VaultContext::open(None, &std::env::current_dir()?) {
        ctx.config().import.allowed_extensions.clone()
    } else {
        vec![]
    };
    let exts: Vec<&str> = ext_strings.iter().map(|s| s.as_str()).collect();

    // Open vault context for duplicate checking (same as import)
    let vault_ctx = VaultContext::open(None, &source_canon).ok();
    let vault_canon = vault_ctx.as_ref().and_then(|ctx| {
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
    let mut entries: Vec<(PathBuf, FileScanStatus)> = Vec::new();
    let mut has_error = false;
    
    for result in crc_rx {
        // Skip vault paths (same logic as import)
        if let Some(ref vault_root) = vault_canon {
            if result.file.path.ancestors().any(|p| p == vault_root) {
                continue;
            }
        }
        
        // Get relative path
        let rel_path = result.file.path.strip_prefix(&source_canon)
            .unwrap_or(&result.file.path)
            .to_path_buf();
        
        // Handle CRC errors
        let crc = match result.crc {
            Ok(c) => c,
            Err(_) => {
                has_error = true;
                entries.push((rel_path, FileScanStatus::Fail));
                continue;
            }
        };
        
        // Build CrcEntry (same as import)
        let crc_entry = pipeline::types::CrcEntry {
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
        
        // Check duplicate using shared function (same as import)
        let status = if let Some(ref ctx) = vault_ctx {
            match check_duplicate(&crc_entry, ctx.db(), ctx.vault_root(), None) {
                pipeline::CheckResult::New => FileScanStatus::New,
                pipeline::CheckResult::Duplicate => FileScanStatus::Dup,
                pipeline::CheckResult::Moved { .. } => FileScanStatus::Dup,
                pipeline::CheckResult::Recover { .. } => FileScanStatus::New,
            }
        } else {
            FileScanStatus::New
        };
        
        entries.push((rel_path, status));
    }
    
    // ========================================================================
    // Output in pipeable format
    // ========================================================================
    if !entries.is_empty() {
        let source_display = source_canon.display().to_string();
        let mut parts = vec![format!("SCAN:{}", source_display)];
        
        for (rel_path, status) in &entries {
            let prefix = match status {
                FileScanStatus::New => "new",
                FileScanStatus::Dup => "dup",
                FileScanStatus::Fail => "fail",
            };
            
            // Only include dup if show_dup is enabled
            if *status == FileScanStatus::Dup && !show_dup {
                continue;
            }
            
            let rel_str = rel_path.display().to_string();
            // Escape spaces and colons in paths for safe parsing
            let escaped = rel_str.replace(' ', "\\ ").replace(':', "\\:");
            parts.push(format!("{}:{}", prefix, escaped));
        }
        
        println!("{}", parts.join(" "));
    }
    
    if has_error {
        std::process::exit(1);
    }
    
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileScanStatus {
    New,
    Dup,
    Fail,
}
