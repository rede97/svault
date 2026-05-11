//! Sync engine: powers `svault sync` (vault→vault) and `svault clone` (vault→directory).
//!
//! Two-phase architecture:
//!   1. Diff — SHA-256 set comparison to determine what needs transfer
//!   2. Transfer — file copy using `fs::transfer_file_with_reporter`
//!
//! Both phases report through typed reporter traits obtained from a
//! [`ReporterBuilder`](crate::reporting::ReporterBuilder).

pub mod diff;
pub mod transfer;

use std::path::Path;

use crate::reporting::{ReporterBuilder, SyncDiffReporter, SyncTransferReporter};

pub use diff::{compute_vault_diff, SyncDiff};
pub use transfer::{transfer_to_dir, transfer_to_vault, SyncTransferSummary};

/// Run a full sync from source vault to target vault.
///
/// 1. Computes SHA-256 diff between source and target DBs.
/// 2. Transfers new files and inserts records into target DB.
/// 3. Returns early if nothing to sync.
pub fn run_sync<RB: ReporterBuilder>(
    source_ctx: &crate::context::VaultContext,
    target_ctx: &crate::context::VaultContext,
    strategies: &[crate::fs::TransferStrategy],
    builder: &RB,
) -> anyhow::Result<SyncTransferSummary> {
    let diff_reporter = builder.sync_diff_reporter();
    let diff = compute_vault_diff(source_ctx.db(), target_ctx.db(), &diff_reporter)?;

    if diff.to_copy.is_empty() {
        diff_reporter.nothing_to_sync();
        diff_reporter.finish();
        return Ok(SyncTransferSummary::default());
    }

    diff_reporter.finish();

    let transfer_reporter = builder.sync_transfer_reporter(
        source_ctx.vault_root(),
        target_ctx.vault_root(),
        diff.to_copy.len() as u64,
    );
    let summary = transfer_to_vault(
        &diff.to_copy,
        source_ctx.vault_root(),
        target_ctx.vault_root(),
        target_ctx.db(),
        strategies,
        &transfer_reporter,
    )?;
    transfer_reporter.finish();

    Ok(summary)
}

/// Run a full clone from source vault to a plain directory.
///
/// Lists all imported files from the source vault, checks target directory for
/// already-present files (by path + size), and copies missing files.
///
/// If `files` is provided, uses that pre-filtered list instead of querying the DB.
pub fn run_clone<RB: ReporterBuilder>(
    source_ctx: &crate::context::VaultContext,
    target_dir: &Path,
    strategies: &[crate::fs::TransferStrategy],
    builder: &RB,
    files: Option<&[crate::db::FileRow]>,
) -> anyhow::Result<SyncTransferSummary> {
    use crate::reporting::CloneReporter;

    let all_files: Vec<crate::db::FileRow> = match files {
        Some(f) => f.to_vec(),
        None => source_ctx
            .db()
            .get_all_files()?
            .into_iter()
            .filter(|f| f.status == "imported")
            .collect(),
    };

    let clone_reporter = builder.clone_reporter();
    clone_reporter.started(all_files.len());

    let (to_clone, already_present) = partition_files_by_target_presence(&all_files, target_dir);
    let total_bytes: u64 = to_clone.iter().map(|f| f.size as u64).sum();

    if to_clone.is_empty() {
        clone_reporter.nothing_to_clone();
        clone_reporter.finish();
        return Ok(SyncTransferSummary::default());
    }

    clone_reporter.diff_computed(to_clone.len(), already_present, total_bytes);
    clone_reporter.finish();

    let transfer_reporter = builder.sync_transfer_reporter(
        source_ctx.vault_root(),
        target_dir,
        to_clone.len() as u64,
    );
    let summary = transfer_to_dir(&to_clone, source_ctx.vault_root(), target_dir, strategies, &transfer_reporter)?;
    transfer_reporter.finish();

    Ok(summary)
}

/// Split files into (missing_on_target, already_present) by checking
/// whether a file exists at `target_dir / rel_path` with matching size.
pub fn partition_files_by_target_presence(
    files: &[crate::db::FileRow],
    target_dir: &Path,
) -> (Vec<crate::db::FileRow>, usize) {
    let mut to_clone = Vec::new();
    let mut present = 0usize;

    for f in files {
        let dst = target_dir.join(&f.path);
        if let Ok(meta) = std::fs::metadata(&dst) {
            if meta.len() as i64 == f.size {
                present += 1;
                continue;
            }
        }
        to_clone.push(f.clone());
    }

    (to_clone, present)
}
