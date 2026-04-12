//! Standalone recheck command implementation.
//!
//! `svault recheck` reads an import manifest and verifies the integrity
//! of both the source files and the vault copies against the recorded hashes.
//! It writes a report to `.svault/staging/` so the user can decide which
//! side is correct. No files are imported or modified.

use std::fs;
use std::path::Path;

use rayon::prelude::*;

use crate::db::Db;
use crate::hash::{sha256_file, xxh3_128_file};
use crate::reporting::{RecheckReporter, ReporterBuilder};
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
pub fn run_recheck<RB: ReporterBuilder>(
    opts: RecheckOptions,
    _db: &Db,
    reporter_builder: &RB,
) -> anyhow::Result<()> {
    let session_id = session_id_now();
    let manifest = &opts.manifest;
    let total = manifest.files.len();

    if total == 0 {
        // Nothing to check — reporter.not_started() or similar could be added
        // if the CLI wants to show a warning; core stays silent.
        return Ok(());
    }

    let reporter = reporter_builder.recheck_reporter(total as u64);
    reporter.started(total, &manifest.session_id, &manifest.source_root);

    let results: Vec<RecheckResult> = manifest
        .files
        .clone()
        .into_par_iter()
        .map(|record| {
            let vault_abs = opts.vault_root.join(&record.dest_path);

            // Signal start of rechecking this file pair
            reporter.item_started(&record.src_path, &vault_abs);

            let has_sha256 = record.sha256.is_some();
            let expected_hash = if has_sha256 {
                record.sha256.clone()
            } else {
                record.xxh3_128.clone()
            };

            let src_hash = if record.src_path.exists() {
                match compute_hash(&record.src_path, has_sha256) {
                    Ok(h) => Some(h),
                    Err(e) => {
                        let status = RecheckStatus::Error(format!("source read error: {e}"));
                        reporter.item_finished(&record.src_path, &vault_abs, &status);
                        return RecheckResult {
                            src_path: record.src_path,
                            vault_path: vault_abs,
                            status,
                            used_sha256: has_sha256,
                        };
                    }
                }
            } else {
                None
            };

            let vault_hash = if vault_abs.exists() {
                match compute_hash(&vault_abs, has_sha256) {
                    Ok(h) => Some(h),
                    Err(e) => {
                        let status = RecheckStatus::Error(format!("vault read error: {e}"));
                        reporter.item_finished(&record.src_path, &vault_abs, &status);
                        return RecheckResult {
                            src_path: record.src_path,
                            vault_path: vault_abs,
                            status,
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
                        (false, false) => RecheckStatus::BothDiverged,
                    }
                }
            };

            reporter.item_finished(&record.src_path, &vault_abs, &status);

            RecheckResult {
                src_path: record.src_path,
                vault_path: vault_abs,
                status,
                used_sha256: has_sha256,
            }
        })
        .collect();

    reporter.finish();

    // Tally results
    let mut ok = 0usize;
    let mut source_modified = 0usize;
    let mut vault_corrupted = 0usize;
    let mut both_diverged = 0usize;
    let mut source_deleted = 0usize;
    let mut vault_deleted = 0usize;
    let mut errors = 0usize;
    let mut sha256_verified = 0usize;

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

    // Write JSON report first, then call reporter.summary with the path
    let report_path = write_report(&opts.vault_root, &session_id, &results)?;

    reporter.summary(
        ok,
        source_modified,
        vault_corrupted,
        both_diverged,
        source_deleted,
        vault_deleted,
        errors,
        sha256_verified,
        &report_path,
    );

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
        Ok(hash
            .to_bytes()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect())
    }
}

/// Write the recheck report to `.svault/staging/` and return its path.
fn write_report(
    vault_root: &Path,
    session_id: &str,
    results: &[RecheckResult],
) -> anyhow::Result<std::path::PathBuf> {
    let staging = vault_root.join(".svault").join("staging");
    fs::create_dir_all(&staging)?;

    let report_path = staging.join(format!("recheck_{}.json", session_id));

    // Build JSON report
    let mut report = serde_json::Map::new();
    report.insert("session_id".to_string(), session_id.into());
    report.insert(
        "checked_at".to_string(),
        chrono::Utc::now().to_rfc3339().into(),
    );

    let items: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            let mut obj = serde_json::Map::new();
            obj.insert(
                "src_path".to_string(),
                r.src_path.to_string_lossy().into_owned().into(),
            );
            obj.insert(
                "vault_path".to_string(),
                r.vault_path.to_string_lossy().into_owned().into(),
            );
            obj.insert("status".to_string(), format!("{:?}", r.status).into());
            obj.insert("used_sha256".to_string(), r.used_sha256.into());
            obj.into()
        })
        .collect();
    report.insert("files".to_string(), items.into());

    let json = serde_json::to_string_pretty(&report)?;
    fs::write(&report_path, json)?;

    Ok(report_path)
}
