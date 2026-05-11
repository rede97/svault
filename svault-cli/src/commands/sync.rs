//! `svault sync` — sync files from another vault into this vault.

use std::path::PathBuf;

use crate::cli::OutputFormat;
use crate::reporting::{JsonReporterBuilder, TerminalReporterBuilder};
use svault_core::config::TransferStrategyArg;
use svault_core::context::VaultContext;
use svault_core::sync;

/// Run sync from a source vault into the current vault.
pub fn run(
    output: OutputFormat,
    source: PathBuf,
    strategy: Vec<TransferStrategyArg>,
) -> anyhow::Result<()> {
    let target_ctx = VaultContext::open_cwd()?;
    let source_ctx = VaultContext::open_at(source)?;

    let strategies: Vec<svault_core::fs::TransferStrategy> = strategy
        .iter()
        .map(|s| s.to_transfer_strategy())
        .collect();

    let summary = match output {
        OutputFormat::Human => {
            let builder = TerminalReporterBuilder::new();
            sync::run_sync(&source_ctx, &target_ctx, &strategies, &builder)?
        }
        OutputFormat::Json => {
            let builder = JsonReporterBuilder::new();
            sync::run_sync(&source_ctx, &target_ctx, &strategies, &builder)?
        }
    };

    // Print summary (human-friendly format)
    match output {
        OutputFormat::Human => {
            if summary.transferred == 0 && summary.skipped == 0 && summary.failed == 0 {
                println!("Nothing to sync — target is up to date.");
            } else {
                println!();
                println!("Sync complete:");
                println!("  Transferred: {:>6}", summary.transferred);
                println!("  Skipped:     {:>6}", summary.skipped);
                println!("  Failed:      {:>6}", summary.failed);
                if summary.total_bytes > 0 {
                    println!("  Total size:  {:>6}", crate::commands::format_bytes(summary.total_bytes));
                }
            }
        }
        OutputFormat::Json => {
            let json = serde_json::json!({
                "event": "sync_summary",
                "transferred": summary.transferred,
                "skipped": summary.skipped,
                "failed": summary.failed,
                "total_bytes": summary.total_bytes,
            });
            println!("{}", json);
        }
    }

    Ok(())
}
