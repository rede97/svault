//! `svault recheck` — manifest integrity verification.
//!
//! Reads an import manifest and verifies both the original source files
//! and the vault copies against the hashes recorded at import time.
//! A report is written to `.svault/sessions/recheck/<ts-id>/report.json`
//! so the user can decide which side is correct.

use std::path::Path;

use rayon::prelude::*;
use serde::Serialize;

use crate::db::Db;
use crate::event::{Event, EventSink, Phase, RecheckSummary, Summary};
use crate::hash::{sha256_file, xxh3_128_file};
use crate::verify::manifest::ImportManifest;

use super::utils::session_id_now;

/// Result of rechecking a single file pair.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
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
    Error { message: String },
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
pub fn run_recheck(opts: RecheckOptions, _db: &Db, sink: &dyn EventSink) -> anyhow::Result<()> {
    let session_id = session_id_now();
    let manifest = &opts.manifest;
    let total = manifest.files.len();

    if total == 0 {
        return Ok(());
    }

    sink.emit(&Event::RecheckStarted {
        total,
        session_id: manifest.session_id.clone(),
        source: manifest.source_root.clone(),
    });

    let results: Vec<RecheckResult> = manifest
        .files
        .clone()
        .into_par_iter()
        .map(|record| {
            let vault_abs = record
                .dest_path
                .as_ref()
                .map(|p| opts.vault_root.join(p))
                .unwrap_or_else(|| opts.vault_root.join("unknown"));

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
                        let status = RecheckStatus::Error {
                            message: format!("source read error: {e}"),
                        };
                        sink.emit(&Event::RecheckItem {
                            src: record.src_path.clone(),
                            vault: vault_abs.clone(),
                            status: status.clone(),
                        });
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
                        let status = RecheckStatus::Error {
                            message: format!("vault read error: {e}"),
                        };
                        sink.emit(&Event::RecheckItem {
                            src: record.src_path.clone(),
                            vault: vault_abs.clone(),
                            status: status.clone(),
                        });
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

            sink.emit(&Event::RecheckItem {
                src: record.src_path.clone(),
                vault: vault_abs.clone(),
                status: status.clone(),
            });

            RecheckResult {
                src_path: record.src_path,
                vault_path: vault_abs,
                status,
                used_sha256: has_sha256,
            }
        })
        .collect();

    sink.emit(&Event::PhaseFinished {
        phase: Phase::Recheck,
    });

    // Tally results
    let mut tally = RecheckSummary::default();

    for r in &results {
        if r.used_sha256 {
            tally.sha256_verified += 1;
        }
        match &r.status {
            RecheckStatus::Ok => tally.ok += 1,
            RecheckStatus::SourceModified => tally.source_modified += 1,
            RecheckStatus::VaultCorrupted => tally.vault_corrupted += 1,
            RecheckStatus::BothDiverged => tally.both_diverged += 1,
            RecheckStatus::SourceDeleted => tally.source_deleted += 1,
            RecheckStatus::VaultDeleted => tally.vault_deleted += 1,
            RecheckStatus::Error { .. } => tally.errors += 1,
        }
    }

    // Write JSON report, then emit the summary with its path
    tally.report_path = write_report(&opts.vault_root, &session_id, &results)?;

    sink.emit(&Event::Summary(Summary::Recheck(tally)));

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

/// Write the recheck report to its session directory
/// (`sessions/recheck/<session_id>/report.json`) and return its path.
fn write_report(
    vault_root: &Path,
    session_id: &str,
    results: &[RecheckResult],
) -> anyhow::Result<std::path::PathBuf> {
    let dir = crate::session::session_dir(
        vault_root,
        crate::verify::manifest::SessionType::Recheck,
        session_id,
    );
    let report_path = dir.join(crate::session::REPORT_FILE);

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

    crate::session::write_json_atomic(&report_path, &report)?;

    Ok(report_path)
}
