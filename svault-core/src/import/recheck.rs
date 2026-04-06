//! Standalone recheck command implementation.
//!
//! `svault recheck` reads an import manifest and verifies the integrity
//! of both the source files and the vault copies against the recorded hashes.
//! It writes a report to `.svault/staging/` so the user can decide which
//! side is correct. No files are imported or modified.

use std::fs;
use std::path::Path;

use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::db::Db;
use crate::hash::{sha256_file, xxh3_128_file};
use crate::verify::manifest::ImportManifest;

use super::utils::session_id_now;

/// Result of rechecking a single file pair.
#[derive(Debug)]
pub enum RecheckStatus {
    /// Both source and vault match the manifest.
    Ok,
    /// Source file has been modified since import.
    SourceModified,
    /// Vault copy has been corrupted since import.
    VaultCorrupted,
    /// Both source and vault have diverged from the manifest.
    BothDiverged,
    /// Source file is missing.
    SourceDeleted,
    /// Vault copy is missing.
    VaultDeleted,
    /// Cannot read one of the files.
    Error(String),
}

/// Per-file recheck result.
#[derive(Debug)]
pub struct RecheckResult {
    pub src_path: std::path::PathBuf,
    pub vault_path: std::path::PathBuf,
    pub status: RecheckStatus,
    /// Whether verification used SHA-256 (definitive) or XXH3-128
    pub used_sha256: bool,
}

/// Options for the standalone `recheck` command.
pub struct RecheckOptions {
    pub vault_root: std::path::PathBuf,
    pub manifest: ImportManifest,
}

/// Run recheck against an import manifest.
///
/// Verification strategy:
/// - If manifest has SHA-256, use it for definitive verification
/// - Otherwise, use XXH3-128 (fast but less secure)
pub fn run_recheck(opts: RecheckOptions, _db: &Db) -> anyhow::Result<()> {
    let session_id = session_id_now();
    let manifest = &opts.manifest;
    let total = manifest.files.len();

    if total == 0 {
        eprintln!("{} Manifest contains no files", style("Warning:").yellow().bold());
        return Ok(());
    }

    eprintln!(
        "{} Rechecking {} files from session {}",
        style("Recheck:").bold().cyan(),
        style(total).cyan(),
        style(&manifest.session_id)
    );
    eprintln!("  Source: {}", style(manifest.source_root.display()));
    eprintln!();
    eprintln!("{} {}",
        style("Caution:").yellow().bold(),
        style("Recheck assumes the source device has not changed since import.").yellow()
    );
    eprintln!("{} {}",
        style("         "),
        style("If you took new photos or modified files, filenames may be reused with different content.")
    );
    eprintln!("{} {}",
        style("         "),
        style("Please review the report carefully before deleting anything.")
    );

    let bar = ProgressBar::new(total as u64);
    bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.cyan} [{bar:40}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("=> "),
    );
    bar.set_prefix("Checking ");

    let results: Vec<RecheckResult> = manifest
        .files
        .clone()
        .into_par_iter()
        .map(|record| {
            let filename = record
                .src_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            bar.set_message(filename);

            let vault_abs = opts.vault_root.join(&record.dest_path);

            // Determine which hash to use: prefer SHA-256 if available
            let has_sha256 = record.sha256.is_some();
            let expected_hash = if has_sha256 {
                record.sha256.clone()
            } else {
                record.xxh3_128.clone()
            };

            // Compute source hash if file exists
            let src_hash = if record.src_path.exists() {
                match compute_hash(&record.src_path, has_sha256) {
                    Ok(h) => Some(h),
                    Err(e) => {
                        bar.inc(1);
                        return RecheckResult {
                            src_path: record.src_path,
                            vault_path: vault_abs,
                            status: RecheckStatus::Error(format!("source read error: {e}")),
                            used_sha256: has_sha256,
                        };
                    }
                }
            } else {
                None
            };

            // Compute vault hash if file exists
            let vault_hash = if vault_abs.exists() {
                match compute_hash(&vault_abs, has_sha256) {
                    Ok(h) => Some(h),
                    Err(e) => {
                        bar.inc(1);
                        return RecheckResult {
                            src_path: record.src_path,
                            vault_path: vault_abs,
                            status: RecheckStatus::Error(format!("vault read error: {e}")),
                            used_sha256: has_sha256,
                        };
                    }
                }
            } else {
                None
            };

            let status = match (&src_hash, &vault_hash) {
                (None, _) => RecheckStatus::SourceDeleted,
                (_, None) => RecheckStatus::VaultDeleted,
                (Some(s), Some(v)) => {
                    let src_ok = expected_hash.as_ref() == Some(s);
                    let vault_ok = expected_hash.as_ref() == Some(v);
                    match (src_ok, vault_ok) {
                        (true, true) => RecheckStatus::Ok,
                        (true, false) => RecheckStatus::VaultCorrupted,
                        (false, true) => RecheckStatus::SourceModified,
                        (false, false) => {
                            if s == v {
                                // Both diverged to the same content — still not the original
                                RecheckStatus::BothDiverged
                            } else {
                                RecheckStatus::BothDiverged
                            }
                        }
                    }
                }
            };

            bar.inc(1);
            RecheckResult {
                src_path: record.src_path,
                vault_path: vault_abs,
                status,
                used_sha256: has_sha256,
            }
        })
        .collect();

    bar.finish_and_clear();

    // Print summary
    let mut ok = 0;
    let mut source_modified = 0;
    let mut vault_corrupted = 0;
    let mut both_diverged = 0;
    let mut source_deleted = 0;
    let mut vault_deleted = 0;
    let mut errors = 0;
    let mut sha256_verified = 0;

    for r in &results {
        if r.used_sha256 {
            sha256_verified += 1;
        }
        match &r.status {
            RecheckStatus::Ok => ok += 1,
            RecheckStatus::SourceModified => source_modified += 1,
            RecheckStatus::VaultCorrupted => vault_corrupted += 1,
            RecheckStatus::BothDiverged => both_diverged += 1,
            RecheckStatus::SourceDeleted => source_deleted += 1,
            RecheckStatus::VaultDeleted => vault_deleted += 1,
            RecheckStatus::Error(_) => errors += 1,
        }
    }

    eprintln!("{}", style("Results:").bold().underlined());
    eprintln!("  {} OK", style(format!("{:>4}", ok)).green());
    if source_modified > 0 {
        eprintln!("  {} Source modified", style(format!("{:>4}", source_modified)).yellow());
    }
    if vault_corrupted > 0 {
        eprintln!("  {} Vault corrupted", style(format!("{:>4}", vault_corrupted)).red());
    }
    if both_diverged > 0 {
        eprintln!("  {} Both diverged", style(format!("{:>4}", both_diverged)).red());
    }
    if source_deleted > 0 {
        eprintln!("  {} Source deleted", style(format!("{:>4}", source_deleted)).yellow());
    }
    if vault_deleted > 0 {
        eprintln!("  {} Vault deleted", style(format!("{:>4}", vault_deleted)).red());
    }
    if errors > 0 {
        eprintln!("  {} Errors", style(format!("{:>4}", errors)).red());
    }
    if sha256_verified > 0 {
        eprintln!("  ({} files verified with SHA-256)", sha256_verified);
    }

    // Write report
    write_report(&opts.vault_root, &session_id, &results)?;

    Ok(())
}

/// Compute hash for a file.
/// If use_sha256 is true, compute SHA-256; otherwise XXH3-128.
fn compute_hash(path: &Path, use_sha256: bool) -> std::io::Result<String> {
    if use_sha256 {
        let hash = sha256_file(path)?;
        Ok(hash.to_hex())
    } else {
        let hash = xxh3_128_file(path)?;
        // Match the format used in manifest: hex of little-endian bytes
        Ok(hash.to_bytes().iter().map(|b| format!("{:02x}", b)).collect())
    }
}

/// Write the recheck report to `.svault/staging/`.
fn write_report(
    vault_root: &Path,
    session_id: &str,
    results: &[RecheckResult],
) -> anyhow::Result<()> {
    let staging = vault_root.join(".svault").join("staging");
    fs::create_dir_all(&staging)?;

    let report_path = staging.join(format!("recheck_{}.json", session_id));

    // Build JSON report
    let mut report = serde_json::Map::new();
    report.insert("session_id".to_string(), session_id.into());
    report.insert("checked_at".to_string(), chrono::Utc::now().to_rfc3339().into());

    let items: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            let mut obj = serde_json::Map::new();
            obj.insert("src_path".to_string(), r.src_path.to_string_lossy().into_owned().into());
            obj.insert("vault_path".to_string(), r.vault_path.to_string_lossy().into_owned().into());
            obj.insert("status".to_string(), format!("{:?}", r.status).into());
            obj.insert("used_sha256".to_string(), r.used_sha256.into());
            obj.into()
        })
        .collect();
    report.insert("files".to_string(), items.into());

    let json = serde_json::to_string_pretty(&report)?;
    fs::write(&report_path, json)?;

    eprintln!();
    eprintln!("{} Report written to {}",
        style("Report:").bold(),
        style(report_path.display()).underlined()
    );

    Ok(())
}
