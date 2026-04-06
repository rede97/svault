//! Archive integrity verification.
//!
//! This module provides functionality to verify that files in the vault
//! match their stored hashes, detecting corruption or tampering.

pub mod background_hash;
pub mod hardlink_upgrade;
pub mod manifest;

use std::path::Path;

use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::config::HashAlgorithm;
use crate::db::{Db, FileRow};
use crate::hash::{sha256_file, xxh3_128_file};

/// Result of a single file verification.
#[derive(Debug, Clone)]
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
    IoError(String),
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
#[derive(Debug, Clone, Default)]
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
pub fn verify_file(
    vault_root: &Path,
    file: &FileRow,
    algo: &HashAlgorithm,
) -> VerifyResult {
    let full_path = vault_root.join(&file.path);

    // Check file exists
    let metadata = match std::fs::metadata(&full_path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return VerifyResult::Missing;
        }
        Err(e) => return VerifyResult::IoError(e.to_string()),
    };

    // Check size matches (fast check)
    let actual_size = metadata.len();
    if actual_size != file.size as u64 {
        return VerifyResult::SizeMismatch {
            expected: file.size as u64,
            actual: actual_size,
        };
    }

    // Check hash based on algorithm
    match algo {
        HashAlgorithm::Xxh3_128 => {
            let stored_hash = match &file.xxh3_128 {
                Some(h) => h,
                None => return VerifyResult::HashNotAvailable,
            };

            let computed = match xxh3_128_file(&full_path) {
                Ok(h) => h,
                Err(e) => return VerifyResult::IoError(e.to_string()),
            };

            if computed.to_bytes() != stored_hash.as_slice() {
                return VerifyResult::HashMismatch {
                    algo: HashAlgorithm::Xxh3_128,
                };
            }
        }
        HashAlgorithm::Sha256 => {
            let stored_hash = match &file.sha256 {
                Some(h) => h,
                None => return VerifyResult::HashNotAvailable,
            };

            let computed = match sha256_file(&full_path) {
                Ok(h) => h,
                Err(e) => return VerifyResult::IoError(e.to_string()),
            };

            if computed.to_bytes() != stored_hash.as_slice() {
                return VerifyResult::HashMismatch {
                    algo: HashAlgorithm::Sha256,
                };
            }
        }
    }

    VerifyResult::Ok
}

/// Verify all files in the vault.
pub fn verify_all(
    vault_root: &Path,
    db: &Db,
    algo: &HashAlgorithm,
) -> anyhow::Result<(Vec<(String, VerifyResult)>, VerifySummary)> {
    let files = db.get_all_files()?;
    let total = files.len();

    if total == 0 {
        return Ok((Vec::new(), VerifySummary::default()));
    }

    let bar = ProgressBar::new(total as u64);
    bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.green} [{bar:40}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("=> "),
    );
    bar.set_prefix("Verifying");

    let vault_root = vault_root.to_path_buf();
    let algo = algo.clone();

    let results: Vec<(String, VerifyResult)> = files
        .into_par_iter()
        .map(|file| {
            let filename = Path::new(&file.path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            bar.set_message(filename);

            let result = verify_file(&vault_root, &file, &algo);
            if result.is_ok() {
                bar.println(format!(
                    "  {} {}",
                    style("Verified").green(),
                    style(&file.path)
                ));
            }
            bar.inc(1);
            (file.path, result)
        })
        .collect();

    bar.finish_and_clear();

    let mut summary = VerifySummary::default();
    for (_, result) in &results {
        match result {
            VerifyResult::Ok => summary.ok += 1,
            VerifyResult::Missing => summary.missing += 1,
            VerifyResult::SizeMismatch { .. } => summary.size_mismatch += 1,
            VerifyResult::HashMismatch { .. } => summary.hash_mismatch += 1,
            VerifyResult::IoError(_) => summary.io_error += 1,
            VerifyResult::HashNotAvailable => summary.hash_not_available += 1,
        }
    }
    summary.total = results.len();

    Ok((results, summary))
}

/// Verify a specific file by path.
pub fn verify_single(
    vault_root: &Path,
    db: &Db,
    file_path: &str,
    algo: &HashAlgorithm,
) -> anyhow::Result<Option<VerifyResult>> {
    match db.get_file_by_path(file_path)? {
        Some(file) => Ok(Some(verify_file(vault_root, &file, algo))),
        None => Ok(None),
    }
}

/// Verify files imported in the last N seconds.
pub fn verify_recent(
    vault_root: &Path,
    db: &Db,
    algo: &HashAlgorithm,
    seconds: u64,
) -> anyhow::Result<(Vec<(String, VerifyResult)>, VerifySummary)> {
    let files = db.get_recent_files(seconds)?;
    let total = files.len();

    if total == 0 {
        return Ok((Vec::new(), VerifySummary::default()));
    }

    let bar = ProgressBar::new(total as u64);
    bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.green} [{bar:40}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("=> "),
    );
    bar.set_prefix("Verifying");

    let vault_root = vault_root.to_path_buf();
    let algo = algo.clone();

    let results: Vec<(String, VerifyResult)> = files
        .into_par_iter()
        .map(|file| {
            let filename = Path::new(&file.path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            bar.set_message(filename);

            let result = verify_file(&vault_root, &file, &algo);
            if result.is_ok() {
                bar.println(format!(
                    "  {} {}",
                    style("Verified").green(),
                    style(&file.path)
                ));
            }
            bar.inc(1);
            (file.path, result)
        })
        .collect();

    bar.finish_and_clear();

    let mut summary = VerifySummary::default();
    for (_, result) in &results {
        match result {
            VerifyResult::Ok => summary.ok += 1,
            VerifyResult::Missing => summary.missing += 1,
            VerifyResult::SizeMismatch { .. } => summary.size_mismatch += 1,
            VerifyResult::HashMismatch { .. } => summary.hash_mismatch += 1,
            VerifyResult::IoError(_) => summary.io_error += 1,
            VerifyResult::HashNotAvailable => summary.hash_not_available += 1,
        }
    }
    summary.total = results.len();

    Ok((results, summary))
}
