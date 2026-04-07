use std::io::{self, BufRead};
use std::path::PathBuf;

use crate::cli::OutputFormat;
use crate::reporting::{create_reporter, TerminalInteractor};
use svault_core::config::SyncStrategy;
use svault_core::context::VaultContext;
use svault_core::import::{run as import_run, run_with_file_list, ImportOptions};
use svault_core::reporting::{Interactor, YesInteractor};
// Note: NoopReporter no longer needed - all import paths use real reporter

pub fn run(
    output: OutputFormat,
    dry_run: bool,
    yes: bool,
    source: PathBuf,
    files_from: Option<PathBuf>,
    target: Option<PathBuf>,
    strategy: Vec<svault_core::config::TransferStrategyArg>,
    force: bool,
    full_id: bool,
    show_dup: bool,
) -> anyhow::Result<()> {
    let source_str = source.to_string_lossy();
    
    // Handle files-from mode (including stdin)
    if let Some(files_from_path) = files_from {
        return run_files_from_import(
            output, dry_run, yes, source, files_from_path, target, strategy, force, full_id, show_dup,
        );
    }
    
    if source_str.starts_with("mtp://") {
        // MTP import via VFS
        #[cfg(feature = "mtp")]
        {
            run_mtp_import(output, dry_run, yes, &source_str, target, strategy, force, full_id, show_dup)
        }
        #[cfg(not(feature = "mtp"))]
        {
            Err(anyhow::anyhow!("MTP support not enabled. Build with --features mtp"))
        }
    } else {
        // Local filesystem import
        run_local_import(output, dry_run, yes, source, target, strategy, force, full_id, show_dup)
    }
}

/// Import from scan output format.
/// 
/// Parses output from `svault scan`: SCAN:<source> new:file1 dup:file2 ...
/// Only imports files marked as "new" (relative paths joined with source).
fn run_files_from_import(
    output: OutputFormat,
    dry_run: bool,
    yes: bool,
    source: PathBuf,
    files_from: PathBuf,
    target: Option<PathBuf>,
    strategy: Vec<svault_core::config::TransferStrategyArg>,
    force: bool,
    full_id: bool,
    show_dup: bool,
) -> anyhow::Result<()> {
    // Read scan output from stdin or file
    let input: Vec<String> = if files_from.as_os_str() == "-" {
        io::stdin()
            .lock()
            .lines()
            .filter_map(|line| line.ok())
            .filter(|line| !line.is_empty())
            .collect()
    } else {
        let file = std::fs::File::open(&files_from)
            .map_err(|e| anyhow::anyhow!("cannot open file list '{}': {}", files_from.display(), e))?;
        io::BufReader::new(file)
            .lines()
            .filter_map(|line| line.ok())
            .filter(|line| !line.is_empty())
            .collect()
    };

    // Parse scan output format and extract relative paths
    let mut rel_paths: Vec<PathBuf> = Vec::new();
    
    for line in input {
        // Parse SCAN:prefix new:file1 dup:file2 ...
        // Skip the SCAN: prefix and extract new: entries
        let parts: Vec<&str> = line.split_whitespace().collect();
        
        for part in parts {
            if part.starts_with("new:") {
                // Extract relative path after "new:"
                let rel_path = &part[4..];
                // Unescape \ -> space and \: -> :
                let unescaped = rel_path.replace("\\ ", " ").replace("\\:", ":");
                if !unescaped.is_empty() {
                    rel_paths.push(PathBuf::from(unescaped));
                }
            }
            // Skip dup: and fail: entries
        }
    }

    if rel_paths.is_empty() {
        return Err(anyhow::anyhow!("no new files to import (all files are duplicates or failed)"));
    }

    // Convert relative paths to absolute paths by joining with source
    let source_canon = std::fs::canonicalize(&source).unwrap_or_else(|_| source.clone());
    let paths: Vec<PathBuf> = rel_paths.into_iter()
        .map(|rel| source_canon.join(rel))
        .collect();

    let ctx = VaultContext::open(target, &source)?;
    
    let opts = ImportOptions {
        source: source_canon,
        vault_root: ctx.vault_root().to_path_buf(),
        strategy: SyncStrategy(strategy),
        dry_run,
        yes,
        import_config: ctx.config().import.clone(),
        force,
        full_id,
        show_dup,
        files_from: None,
    };
    
    // JSON mode without --yes is not allowed (cannot prompt without breaking stdout)
    if matches!(output, OutputFormat::Json) && !yes {
        return Err(anyhow::anyhow!("JSON mode requires --yes flag (cannot prompt without breaking structured output)"));
    }
    
    let reporter = create_reporter(&output);
    
    // Only --yes skips prompt; JSON mode alone does not auto-confirm
    let interactor: Box<dyn Interactor> = if yes {
        Box::new(YesInteractor)
    } else {
        Box::new(TerminalInteractor)
    };
    
    let summary = run_with_file_list(opts, ctx.db(), paths, reporter.as_ref(), interactor.as_ref())?;
    
    if matches!(output, OutputFormat::Json) {
        println!(
            "{}",
            serde_json::json!({
                "total": summary.total,
                "imported": summary.imported,
                "duplicate": summary.duplicate,
                "failed": summary.failed,
                "all_cache_hit": summary.all_cache_hit,
                "manifest": summary.manifest_path.map(|p| p.display().to_string()),
            })
        );
    }
    Ok(())
}

#[cfg(feature = "mtp")]
fn run_mtp_import(
    output: OutputFormat,
    dry_run: bool,
    yes: bool,
    source_str: &str,
    target: Option<PathBuf>,
    strategy: Vec<svault_core::config::TransferStrategyArg>,
    force: bool,
    full_id: bool,
    show_dup: bool,
) -> anyhow::Result<()> {
    // JSON mode without --yes is not allowed (cannot prompt without breaking stdout)
    if matches!(output, OutputFormat::Json) && !yes {
        return Err(anyhow::anyhow!("JSON mode requires --yes flag (cannot prompt without breaking structured output)"));
    }
    
    use svault_core::import::vfs_import::{run_vfs_import, VfsImportOptions};
    use svault_core::vfs::manager::VfsManager;

    let ctx = VaultContext::open(target, &std::env::current_dir()?)?;

    let manager = VfsManager::new();
    let (backend, mtp_path) = manager
        .open_url(source_str)
        .map_err(|e| anyhow::anyhow!("failed to open MTP device: {e}"))?;

    let opts = VfsImportOptions {
        src_backend: &*backend,
        src_path: &mtp_path,
        vault_root: ctx.vault_root(),
        dry_run,
        yes,
        import_config: ctx.config().import.clone(),
        source_name: source_str.to_string(),
        strategy: SyncStrategy(strategy),
        force,
        full_id,
        show_dup,
        crc_buffer_size: 64 * 1024, // 64KB for MTP (good balance)
    };

    let reporter = create_reporter(&output);
    
    // Only --yes skips prompt; JSON mode alone does not auto-confirm
    let interactor: Box<dyn Interactor> = if yes {
        Box::new(YesInteractor)
    } else {
        Box::new(TerminalInteractor)
    };
    
    let summary = run_vfs_import(opts, ctx.db(), reporter.as_ref(), interactor.as_ref())?;

    if matches!(output, OutputFormat::Json) {
        println!(
            "{}",
            serde_json::json!({
                "total": summary.total,
                "imported": summary.imported,
                "duplicate": summary.duplicate,
                "failed": summary.failed,
                "all_cache_hit": summary.all_cache_hit,
                "manifest": summary.manifest_path.map(|p| p.display().to_string()),
            })
        );
    }
    Ok(())
}

fn run_local_import(
    output: OutputFormat,
    dry_run: bool,
    yes: bool,
    source: PathBuf,
    target: Option<PathBuf>,
    strategy: Vec<svault_core::config::TransferStrategyArg>,
    force: bool,
    full_id: bool,
    show_dup: bool,
) -> anyhow::Result<()> {
    // JSON mode without --yes is not allowed (cannot prompt without breaking stdout)
    if matches!(output, OutputFormat::Json) && !yes {
        return Err(anyhow::anyhow!("JSON mode requires --yes flag (cannot prompt without breaking structured output)"));
    }
    
    let ctx = VaultContext::open(target, &source)?;
    let opts = ImportOptions {
        source,
        vault_root: ctx.vault_root().to_path_buf(),
        strategy: SyncStrategy(strategy),
        dry_run,
        yes,
        import_config: ctx.config().import.clone(),
        force,
        full_id,
        show_dup,
        files_from: None,
    };
    let reporter = create_reporter(&output);
    
    // Only --yes skips prompt; JSON mode alone does not auto-confirm
    let interactor: Box<dyn Interactor> = if yes {
        Box::new(YesInteractor)
    } else {
        Box::new(TerminalInteractor)
    };
    
    let summary = import_run(opts, ctx.db(), reporter.as_ref(), interactor.as_ref())?;
    if matches!(output, OutputFormat::Json) {
        println!(
            "{}",
            serde_json::json!({
                "total": summary.total,
                "imported": summary.imported,
                "duplicate": summary.duplicate,
                "failed": summary.failed,
                "all_cache_hit": summary.all_cache_hit,
                "manifest": summary.manifest_path.map(|p| p.display().to_string()),
            })
        );
    }
    Ok(())
}
