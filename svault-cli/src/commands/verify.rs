use std::path::PathBuf;
use std::sync::Arc;

use crate::cli::OutputFormat;
use crate::reporting::TerminalReporterBuilder;
use console::style;
use svault_core::context::VaultContext;
use svault_core::db;
use svault_core::verify::background_hash;
use svault_core::verify::{VerifyResult, VerifySummary, verify_all, verify_recent, verify_single};

pub fn run(
    output: OutputFormat,
    file: Option<PathBuf>,
    recent: Option<u64>,
    upgrade_links: bool,
    background_hash: bool,
    background_hash_limit: Option<usize>,
) -> anyhow::Result<()> {
    let ctx = VaultContext::open_cwd()?;

    if background_hash {
        let opts = background_hash::BackgroundHashOptions {
            vault_root: ctx.vault_root().to_path_buf(),
            limit: background_hash_limit,
            nice: false, // Auto-managed based on system load
        };
        let reporter_builder = Arc::new(TerminalReporterBuilder::new());
        let _summary =
            background_hash::run_background_hash(opts, ctx.db(), reporter_builder.as_ref())?;
        // Summary is printed by reporter
        // If only background-hash is requested (no other flags), return early
        if !upgrade_links && recent.is_none() && file.is_none() {
            return Ok(());
        }
    }

    if upgrade_links {
        upgrade_hardlinks(ctx.vault_root(), ctx.db(), recent, file.as_ref())?;
    }

    if let Some(seconds) = recent {
        eprintln!(
            "{} Verifying files imported in the last {} seconds",
            style("Verify:").bold().cyan(),
            style(seconds).cyan()
        );
        let reporter_builder = Arc::new(TerminalReporterBuilder::new());
        let (results, summary) = verify_recent(
            ctx.vault_root(),
            ctx.db(),
            seconds,
            reporter_builder.as_ref(),
        )?;
        print_verify_results(output, &results, &summary)?;
        return Ok(());
    }

    if let Some(file_path) = file {
        verify_single_file(ctx.vault_root(), ctx.db(), &file_path)?;
    } else {
        eprintln!(
            "{} Verifying all files in vault",
            style("Verify:").bold().cyan()
        );
        let reporter_builder = Arc::new(TerminalReporterBuilder::new());
        let (results, summary) = verify_all(ctx.vault_root(), ctx.db(), reporter_builder.as_ref())?;
        print_verify_results(output, &results, &summary)?;
    }

    Ok(())
}

fn upgrade_hardlinks(
    vault_root: &std::path::Path,
    db: &db::Db,
    recent: Option<u64>,
    file: Option<&PathBuf>,
) -> anyhow::Result<()> {
    let files_to_check: Vec<svault_core::db::FileRow> = if let Some(seconds) = recent {
        db.get_recent_files(seconds)?
    } else if let Some(file_path) = file {
        if let Some(f) = db.get_file_by_path(&file_path.to_string_lossy())? {
            vec![f]
        } else {
            Vec::new()
        }
    } else {
        db.get_all_files()?
    };

    for file_row in files_to_check {
        let full_path = vault_root.join(&file_row.path);
        match svault_core::verify::hardlink_upgrade::is_hardlinked(&full_path) {
            Ok(true) => {
                if let Err(e) =
                    svault_core::verify::hardlink_upgrade::upgrade_to_binary_copy(&full_path)
                {
                    eprintln!(
                        "  {} Failed to upgrade hardlink {}: {}",
                        style("⚠").yellow().bold(),
                        full_path.display(),
                        e
                    );
                } else {
                    eprintln!(
                        "  {} Upgraded hardlink {}",
                        style("→").cyan(),
                        full_path.display()
                    );
                }
            }
            Ok(false) => {}
            Err(e) => {
                eprintln!(
                    "  {} Failed to check {}: {}",
                    style("⚠").yellow().bold(),
                    full_path.display(),
                    e
                );
            }
        }
    }
    Ok(())
}

fn verify_single_file(
    vault_root: &std::path::Path,
    db: &db::Db,
    file_path: &std::path::Path,
) -> anyhow::Result<()> {
    match verify_single(vault_root, db, &file_path.to_string_lossy())? {
        Some(result) => match result {
            VerifyResult::Ok => {
                println!("{} {}", style("✓").green().bold(), file_path.display());
            }
            VerifyResult::Missing => {
                eprintln!(
                    "{} {} - File not found",
                    style("✗").red().bold(),
                    file_path.display()
                );
                std::process::exit(1);
            }
            VerifyResult::SizeMismatch { expected, actual } => {
                eprintln!(
                    "{} {} - Size mismatch (expected {}, got {})",
                    style("✗").red().bold(),
                    file_path.display(),
                    expected,
                    actual
                );
                std::process::exit(1);
            }
            VerifyResult::HashMismatch { algo } => {
                eprintln!(
                    "{} {} - Hash mismatch ({})",
                    style("✗").red().bold(),
                    file_path.display(),
                    algo
                );
                std::process::exit(1);
            }
            VerifyResult::IoError(e) => {
                eprintln!(
                    "{} {} - IO error: {}",
                    style("✗").red().bold(),
                    file_path.display(),
                    e
                );
                std::process::exit(1);
            }
            VerifyResult::HashNotAvailable => {
                eprintln!(
                    "{} {} - Hash not computed yet",
                    style("!").yellow().bold(),
                    file_path.display()
                );
            }
        },
        None => {
            anyhow::bail!("File not found in database: {}", file_path.display());
        }
    }
    Ok(())
}

pub fn print_verify_results(
    _output: OutputFormat,
    results: &[(String, VerifyResult)],
    summary: &VerifySummary,
) -> anyhow::Result<()> {
    let mut has_failures = false;
    for (path, result) in results {
        match result {
            VerifyResult::Ok => {}
            VerifyResult::Missing => {
                has_failures = true;
                eprintln!("{} {} - Missing", style("✗").red().bold(), path);
            }
            VerifyResult::SizeMismatch { expected, actual } => {
                has_failures = true;
                eprintln!(
                    "{} {} - Size mismatch (expected {}, got {})",
                    style("✗").red().bold(),
                    path,
                    expected,
                    actual
                );
            }
            VerifyResult::HashMismatch { algo } => {
                has_failures = true;
                eprintln!(
                    "{} {} - {} hash mismatch",
                    style("✗").red().bold(),
                    path,
                    algo
                );
            }
            VerifyResult::IoError(e) => {
                has_failures = true;
                eprintln!("{} {} - IO error: {}", style("✗").red().bold(), path, e);
            }
            VerifyResult::HashNotAvailable => {
                eprintln!(
                    "{} {} - Hash not available",
                    style("!").yellow().bold(),
                    path
                );
            }
        }
    }

    eprintln!();
    eprintln!("{}", style("Summary:").bold());
    eprintln!(
        "  {} {}",
        style(format!("OK:               {:>6}", summary.ok)).green(),
        style("verified successfully")
    );
    if summary.missing > 0 {
        eprintln!(
            "  {} {}",
            style(format!("Missing:          {:>6}", summary.missing)).red(),
            style("file not found on disk")
        );
    }
    if summary.size_mismatch > 0 {
        eprintln!(
            "  {} {}",
            style(format!("Size mismatch:    {:>6}", summary.size_mismatch)).red(),
            style("file size differs from database")
        );
    }
    if summary.hash_mismatch > 0 {
        eprintln!(
            "  {} {}",
            style(format!("Hash mismatch:    {:>6}", summary.hash_mismatch)).red(),
            style("hash does not match database")
        );
    }
    if summary.io_error > 0 {
        eprintln!(
            "  {} {}",
            style(format!("IO error:         {:>6}", summary.io_error)).red(),
            style("unable to read file")
        );
    }
    if summary.hash_not_available > 0 {
        eprintln!(
            "  {} {}",
            style(format!(
                "Hash pending:     {:>6}",
                summary.hash_not_available
            ))
            .yellow(),
            style("hash not yet computed")
        );
    }

    if has_failures {
        std::process::exit(1);
    }
    Ok(())
}
