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

use crate::config::HashAlgorithm;
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
}

/// Options for the standalone `recheck` command.
pub struct RecheckOptions {
    pub vault_root: std::path::PathBuf,
    pub manifest: ImportManifest,
    pub hash: HashAlgorithm,
}

/// Run recheck against an import manifest.
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

            // Compute source hash if file exists
            let src_hash = if record.src_path.exists() {
                match compute_hash(&record.src_path, &opts.hash) {
                    Ok(h) => Some(h),
                    Err(e) => {
                        bar.inc(1);
                        return RecheckResult {
                            src_path: record.src_path,
                            vault_path: vault_abs,
                            status: RecheckStatus::Error(format!("source read error: {e}")),
                        };
                    }
                }
            } else {
                None
            };

            // Compute vault hash if file exists
            let vault_hash = if vault_abs.exists() {
                match compute_hash(&vault_abs, &opts.hash) {
                    Ok(h) => Some(h),
                    Err(e) => {
                        bar.inc(1);
                        return RecheckResult {
                            src_path: record.src_path,
                            vault_path: vault_abs,
                            status: RecheckStatus::Error(format!("vault read error: {e}")),
                        };
                    }
                }
            } else {
                None
            };

            let expected_hash = manifest_hash(&record, &opts.hash);

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

    for r in &results {
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

    eprintln!();
    eprintln!("{}", style("Summary:").bold());
    eprintln!(
        "  {} {}",
        style(format!("OK:               {:>6}", ok)).green(),
        style("source and vault match manifest")
    );
    if source_modified > 0 {
        eprintln!(
            "  {} {}",
            style(format!("Source modified:  {:>6}", source_modified)).yellow(),
            style("vault is intact")
        );
    }
    if vault_corrupted > 0 {
        eprintln!(
            "  {} {}",
            style(format!("Vault corrupted:  {:>6}", vault_corrupted)).red(),
            style("source is intact")
        );
    }
    if both_diverged > 0 {
        eprintln!(
            "  {} {}",
            style(format!("Both diverged:    {:>6}", both_diverged)).red().bold(),
            style("neither matches manifest")
        );
    }
    if source_deleted > 0 {
        eprintln!(
            "  {} {}",
            style(format!("Source deleted:   {:>6}", source_deleted)).yellow(),
            style("source file missing")
        );
    }
    if vault_deleted > 0 {
        eprintln!(
            "  {} {}",
            style(format!("Vault deleted:    {:>6}", vault_deleted)).red(),
            style("vault file missing")
        );
    }
    if errors > 0 {
        eprintln!(
            "  {}",
            style(format!("Errors:           {:>6}", errors)).red()
        );
    }

    // Write detailed report
    write_report(
        &manifest.source_root.display().to_string(),
        &session_id,
        &opts.hash,
        &results,
        &opts.vault_root,
    )
}

/// Compute full-file hash using the configured algorithm.
/// Compute full-file hash using the configured algorithm.
///
/// # Format Compatibility
///
/// The hash string format MUST match exactly what was stored in the import manifest
/// (see `pipeline::insert::bytes_to_hex`). This is critical for recheck to work correctly.
///
/// For XXH3-128, we use `to_bytes().iter().map(|b| format!("{:02x}", b))` which produces
/// a little-endian byte array hex string. This differs from `format!("{:x}", hash)` which
/// uses the Display trait and produces a different byte order (high 64 bits first).
///
/// # Example
///
/// ```text
/// XXH3-128 hash of "hello"
/// - to_bytes() hex:   "f12ea78b328f5c8a0268e0971539ea4f" (little-endian bytes)
/// - Display trait:    "4fea391597e068028a5c8f328ba72ef1" (high:low u64 format)
///
/// Manifest stores: "f12ea78b328f5c8a0268e0971539ea4f"
/// So we must use to_bytes() format here for comparison to work.
/// ```
fn compute_hash(path: &Path, algo: &HashAlgorithm) -> std::io::Result<String> {
    match algo {
        HashAlgorithm::Xxh3_128 => {
            let hash = xxh3_128_file(path)?;
            // Match the format used in manifest: hex of little-endian bytes
            Ok(hash.to_bytes().iter().map(|b| format!("{:02x}", b)).collect())
        }
        HashAlgorithm::Sha256 => {
            let hash = sha256_file(path)?;
            Ok(hash.to_hex())
        }
    }
}

/// Retrieve the expected hash from the manifest record.
fn manifest_hash(record: &crate::verify::manifest::ImportRecord, algo: &HashAlgorithm) -> Option<String> {
    match algo {
        HashAlgorithm::Xxh3_128 => record.xxh3_128.clone(),
        HashAlgorithm::Sha256 => record.sha256.clone(),
    }
}

/// Write the recheck report to `.svault/staging/`.
fn write_report(
    source_name: &str,
    session_id: &str,
    hash_algo: &HashAlgorithm,
    results: &[RecheckResult],
    vault_root: &Path,
) -> anyhow::Result<()> {
    let staging_dir = vault_root.join(".svault").join("staging");
    fs::create_dir_all(&staging_dir)?;
    let report_path = staging_dir.join(format!("recheck-{session_id}.txt"));

    let mut buf = String::new();
    buf.push_str("# Recheck Report\n");
    buf.push_str(&format!("# Session: {session_id}\n"));
    buf.push_str(&format!("# Source: {source_name}\n"));
    buf.push_str(&format!("# Hash: {hash_algo:?}\n"));
    buf.push_str("#\n");
    buf.push_str("# CAUTION: Recheck assumes the source device has not changed since import.\n");
    buf.push_str("# If you took new photos or modified files, filenames may be reused with different content.\n");
    buf.push_str("# Review this report carefully before deleting any files.\n");
    buf.push('\n');

    for r in results {
        let status_str = match &r.status {
            RecheckStatus::Ok => "OK",
            RecheckStatus::SourceModified => "SOURCE_MODIFIED",
            RecheckStatus::VaultCorrupted => "VAULT_CORRUPTED",
            RecheckStatus::BothDiverged => "BOTH_DIVERGED",
            RecheckStatus::SourceDeleted => "SOURCE_DELETED",
            RecheckStatus::VaultDeleted => "VAULT_DELETED",
            RecheckStatus::Error(e) => &format!("ERROR: {e}"),
        };
        buf.push_str(&format!(
            "{status_str:20} {}  ->  {}\n",
            r.src_path.display(),
            r.vault_path.display()
        ));
    }

    fs::write(&report_path, buf)?;
    eprintln!();
    eprintln!(
        "{} {}",
        style("Report:").bold(),
        style(report_path.display())
    );
    Ok(())
}
