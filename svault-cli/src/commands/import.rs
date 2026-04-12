use std::io::{self, BufRead};
use std::path::PathBuf;
use std::sync::Arc;

use crate::cli::OutputFormat;
use crate::reporting::{JsonReporterBuilder, SuspendingInteractor, TerminalReporterBuilder};
use svault_core::config::SyncStrategy;
use svault_core::context::VaultContext;
use svault_core::import::ImportOptions;
use svault_core::reporting::YesInteractor;

/// Normalize a path by removing trailing backslashes and quotes.
/// 
/// On Windows, PowerShell may add trailing backslashes when auto-completing paths,
/// which can cause issues when the backslash escapes the closing quote.
fn normalize_path(path: &std::path::Path) -> PathBuf {
    let path_str = path.as_os_str().to_string_lossy();
    
    // Repeatedly strip trailing backslashes and quotes
    let mut cleaned = path_str.as_ref();
    loop {
        let new_cleaned = cleaned
            .trim_end_matches('\\')
            .trim_end_matches('/')
            .trim_end_matches('"')
            .trim_end_matches('\'');
        if new_cleaned == cleaned {
            break;
        }
        cleaned = new_cleaned;
    }
    
    PathBuf::from(cleaned)
}

#[allow(clippy::too_many_arguments)]
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
    // Parse file-list input (stdin or file) into Vec<PathBuf> before entering core.
    let file_list: Option<Vec<PathBuf>> = match files_from {
        None => None,
        Some(ref path) => {
            let lines: Vec<String> = if path.as_os_str() == "-" {
                io::stdin()
                    .lock()
                    .lines()
                    .map_while(Result::ok)
                    .filter(|l| !l.is_empty())
                    .collect()
            } else {
                let file = std::fs::File::open(path).map_err(|e| {
                    anyhow::anyhow!("cannot open file list '{}': {}", path.display(), e)
                })?;
                io::BufReader::new(file)
                    .lines()
                    .map_while(Result::ok)
                    .filter(|l| !l.is_empty())
                    .collect()
            };

            // Parse scan-output format: SCAN:<prefix> new:file1 dup:file2 …
            // Only "new:" entries are imported; relative paths are joined with source.
            let source_normalized = normalize_path(&source);
            let source_canon = dunce::canonicalize(&source_normalized).unwrap_or_else(|_| source_normalized.clone());
            let mut paths: Vec<PathBuf> = Vec::new();

            for line in &lines {
                let parts: Vec<&str> = line.split_whitespace().collect();
                for part in parts {
                    if let Some(rel) = part.strip_prefix("new:") {
                        let unescaped = rel.replace("\\ ", " ").replace("\\:", ":");
                        if !unescaped.is_empty() {
                            paths.push(source_canon.join(unescaped));
                        }
                    }
                }
            }

            if paths.is_empty() {
                return Err(anyhow::anyhow!(
                    "no new files to import (all files are duplicates or failed)"
                ));
            }

            Some(paths)
        }
    };

    let source_normalized = normalize_path(&source);
    let source_canon = dunce::canonicalize(&source_normalized).unwrap_or_else(|_| source_normalized.clone());
    let ctx = VaultContext::open(target, &source_canon)?;

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
        files_from: file_list,
    };

    match output {
        OutputFormat::Json => {
            // JSON mode requires --yes flag for non-interactive execution
            if !yes {
                return Err(anyhow::anyhow!(
                    "JSON output mode requires --yes flag to confirm non-interactive execution"
                ));
            }
            let reporter_builder = JsonReporterBuilder::new();
            let _summary = opts.run_import(ctx.db(), &reporter_builder, &YesInteractor)?;
        }
        OutputFormat::Human => {
            let reporter_builder = Arc::new(TerminalReporterBuilder::new());
            if yes {
                let _summary = opts.run_import(ctx.db(), reporter_builder.as_ref(), &YesInteractor)?;
            } else {
                let interactor = SuspendingInteractor::new(reporter_builder.multi_progress.clone());
                let _summary = opts.run_import(ctx.db(), reporter_builder.as_ref(), &interactor)?;
            }
        }
    }
    Ok(())
}
