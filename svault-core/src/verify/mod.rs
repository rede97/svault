//! Archive integrity verification.
//!
//! This module provides functionality to verify that files in the vault
//! match their stored hashes, detecting corruption or tampering.

pub mod background_hash;
pub mod hardlink_upgrade;
pub mod manifest;

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;

use crate::config::HashAlgorithm;
use crate::db::{Db, FileRow};
use crate::event::{Event, EventSink, Phase, PhaseContext, Summary};
use crate::hash::{sha256_file, xxh3_128_file};

/// Result of a single file verification.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum VerifyResult {
    /// File verified successfully, hash matches.
    Ok,
    /// File not found on disk.
    Missing,
    /// File size mismatch (indicates corruption or modification).
    SizeMismatch { expected: u64, actual: u64 },
    /// Hash mismatch (indicates corruption or tampering).
    HashMismatch { algo: HashAlgorithm },
    /// Error reading file.
    IoError { message: String },
    /// Hash not computed yet (lazy hashing).
    HashNotAvailable,
}

impl VerifyResult {
    /// Returns true if verification passed.
    pub fn is_ok(&self) -> bool {
        matches!(self, VerifyResult::Ok)
    }

    /// Returns true if this is a failure that needs attention.
    pub fn is_failed(&self) -> bool {
        !matches!(self, VerifyResult::Ok | VerifyResult::HashNotAvailable)
    }
}

/// Summary of a verify run.
#[derive(Debug, Clone, Default, Serialize)]
pub struct VerifySummary {
    pub total: usize,
    pub ok: usize,
    pub missing: usize,
    pub size_mismatch: usize,
    pub hash_mismatch: usize,
    pub io_error: usize,
    pub hash_not_available: usize,
}

/// Verify a single file.
///
/// Verification strategy:
/// - If file has SHA-256 in DB, use it for definitive verification
/// - Otherwise, use XXH3-128
pub fn verify_file(vault_root: &Path, file: &FileRow) -> VerifyResult {
    let full_path = vault_root.join(&file.path);

    // Check file exists
    let metadata = match std::fs::metadata(&full_path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return VerifyResult::Missing;
        }
        Err(e) => {
            return VerifyResult::IoError {
                message: e.to_string(),
            };
        }
    };

    // Check size matches (fast check)
    let actual_size = metadata.len();
    if actual_size != file.size as u64 {
        return VerifyResult::SizeMismatch {
            expected: file.size as u64,
            actual: actual_size,
        };
    }

    // Priority: SHA-256 (definitive) > XXH3-128 (fast)
    if let Some(stored_sha256) = &file.sha256 {
        // Definitive verification with SHA-256
        let computed = match sha256_file(&full_path) {
            Ok(h) => h,
            Err(e) => {
                return VerifyResult::IoError {
                    message: e.to_string(),
                };
            }
        };

        if computed.to_bytes() != stored_sha256.as_slice() {
            return VerifyResult::HashMismatch {
                algo: HashAlgorithm::Sha256,
            };
        }
        return VerifyResult::Ok;
    }

    // Fallback to XXH3-128
    if let Some(stored_xxh3) = &file.xxh3_128 {
        let computed = match xxh3_128_file(&full_path) {
            Ok(h) => h,
            Err(e) => {
                return VerifyResult::IoError {
                    message: e.to_string(),
                };
            }
        };

        if computed.to_bytes() != stored_xxh3.as_slice() {
            return VerifyResult::HashMismatch {
                algo: HashAlgorithm::Xxh3_128,
            };
        }
        return VerifyResult::Ok;
    }

    // No hash available
    VerifyResult::HashNotAvailable
}

/// Verify all files in the vault.
pub fn verify_all(
    vault_root: &Path,
    db: &Db,
    sink: &dyn EventSink,
) -> anyhow::Result<(Vec<(String, VerifyResult)>, VerifySummary)> {
    let files = db.get_all_files()?;
    verify_files(vault_root, files, sink)
}

/// Verify a specific file by path.
pub fn verify_single(
    vault_root: &Path,
    db: &Db,
    file_path: &str,
) -> anyhow::Result<Option<VerifyResult>> {
    match db.get_file_by_path(file_path)? {
        Some(file) => Ok(Some(verify_file(vault_root, &file))),
        None => Ok(None),
    }
}

/// Verify files imported in the last N seconds.
pub fn verify_recent(
    vault_root: &Path,
    db: &Db,
    seconds: u64,
    sink: &dyn EventSink,
) -> anyhow::Result<(Vec<(String, VerifyResult)>, VerifySummary)> {
    let files = db.get_recent_files(seconds)?;
    verify_files(vault_root, files, sink)
}

/// Shared verification driver: hashes the given DB rows in parallel, emits
/// [`Event::VerifyItem`] per file, and finishes with [`Summary::Verify`].
fn verify_files(
    vault_root: &Path,
    files: Vec<FileRow>,
    sink: &dyn EventSink,
) -> anyhow::Result<(Vec<(String, VerifyResult)>, VerifySummary)> {
    let total = files.len();

    if total == 0 {
        return Ok((Vec::new(), VerifySummary::default()));
    }

    sink.emit(&Event::PhaseStarted {
        phase: Phase::Verify,
        total: Some(total as u64),
        context: PhaseContext::vault(vault_root.to_path_buf()),
    });

    let vault_root = vault_root.to_path_buf();

    let results: Vec<(String, VerifyResult)> = files
        .into_par_iter()
        .map(|file| {
            let result = verify_file(&vault_root, &file);
            sink.emit(&Event::VerifyItem {
                path: PathBuf::from(&file.path),
                result: result.clone(),
            });
            (file.path, result)
        })
        .collect();

    sink.emit(&Event::PhaseFinished {
        phase: Phase::Verify,
    });

    let mut summary = VerifySummary::default();
    for (_, result) in &results {
        match result {
            VerifyResult::Ok => summary.ok += 1,
            VerifyResult::Missing => summary.missing += 1,
            VerifyResult::SizeMismatch { .. } => summary.size_mismatch += 1,
            VerifyResult::HashMismatch { .. } => summary.hash_mismatch += 1,
            VerifyResult::IoError { .. } => summary.io_error += 1,
            VerifyResult::HashNotAvailable => summary.hash_not_available += 1,
        }
    }
    summary.total = results.len();

    sink.emit(&Event::Summary(Summary::Verify(Box::new(summary.clone()))));

    Ok((results, summary))
}
