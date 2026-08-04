use std::path::PathBuf;

use svault_core::context::VaultContext;
use svault_core::db;
use svault_core::verify::background_hash;
use svault_core::verify::{verify_all, verify_recent, verify_single};
use svault_ui::messages;

use crate::cli::OutputFormat;
use crate::commands::SinkSet;

pub fn run(
    output: OutputFormat,
    quiet: bool,
    file: Option<PathBuf>,
    recent: Option<u64>,
    upgrade_links: bool,
    background_hash: bool,
    background_hash_limit: Option<usize>,
) -> anyhow::Result<()> {
    let ctx = VaultContext::open_cwd()?;
    let sink = SinkSet::new(&output, quiet, false);

    if background_hash {
        let opts = background_hash::BackgroundHashOptions {
            vault_root: ctx.vault_root().to_path_buf(),
            limit: background_hash_limit,
            nice: false,
        };
        let summary = background_hash::run_background_hash(opts, ctx.db(), sink.as_sink())?;
        if !quiet {
            messages::info(&format!(
                "Background hash complete: {} processed, {} failed",
                summary.processed, summary.failed
            ));
        }
        // If only background-hash is requested, return early
        if !upgrade_links && recent.is_none() && file.is_none() {
            return Ok(());
        }
    }

    if upgrade_links {
        upgrade_hardlinks(ctx.vault_root(), ctx.db(), recent, file.as_ref())?;
    }

    if let Some(seconds) = recent {
        if !quiet {
            messages::action(
                "Verify",
                &format!("Verifying files imported in the last {} seconds", seconds),
            );
        }
        let (_results, summary) =
            verify_recent(ctx.vault_root(), ctx.db(), seconds, sink.as_sink())?;
        exit_on_failures(&summary)?;
        return Ok(());
    }

    if let Some(file_path) = file {
        return verify_single_file(ctx.vault_root(), ctx.db(), &file_path);
    }

    if !quiet {
        messages::action("Verify", "Verifying all files in vault");
    }
    let (_results, summary) = verify_all(ctx.vault_root(), ctx.db(), sink.as_sink())?;
    exit_on_failures(&summary)?;
    Ok(())
}

/// Return an error (exit code 1 via main) when verification found failures.
fn exit_on_failures(summary: &svault_core::verify::VerifySummary) -> anyhow::Result<()> {
    let failures =
        summary.missing + summary.size_mismatch + summary.hash_mismatch + summary.io_error;
    if failures > 0 {
        anyhow::bail!("verification failed: {} file(s) need attention", failures);
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
                    messages::warn(&format!(
                        "Failed to upgrade hardlink {}: {}",
                        full_path.display(),
                        e
                    ));
                } else {
                    messages::info(&format!("Upgraded hardlink {}", full_path.display()));
                }
            }
            Ok(false) => {}
            Err(e) => {
                messages::warn(&format!("Failed to check {}: {}", full_path.display(), e));
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
        Some(result) => {
            if messages::verify_single_result(file_path, &result) {
                Ok(())
            } else {
                anyhow::bail!("verification failed: {}", file_path.display())
            }
        }
        None => {
            anyhow::bail!("File not found in database: {}", file_path.display());
        }
    }
}
