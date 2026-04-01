//! Standalone recheck command implementation.
//!
//! `svault recheck <source>` scans a source directory (or MTP device),
//! compares every file that hits the CRC32C cache against the vault,
//! and writes a report. No files are imported and no vault files are
//! modified — the user decides which side is correct after reviewing.

use std::fs;
use std::io::Read;
use std::path::Path;

use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::config::{HashAlgorithm, ImportConfig};
use crate::db::Db;
use crate::hash::{crc32c_region, sha256_file, xxh3_128_file};
use crate::vfs::{DirEntry, VfsBackend, VfsError};

use super::types::{FileStatus, ImportSummary, ScanEntry};
use super::utils::session_id_now;

/// Options for the standalone `recheck` command.
pub struct RecheckOptions {
    pub source: std::path::PathBuf,
    pub vault_root: std::path::PathBuf,
    pub hash: HashAlgorithm,
    pub import_config: ImportConfig,
}

/// Options for VFS-based recheck (MTP, etc.).
pub struct VfsRecheckOptions<'a> {
    pub src_backend: &'a dyn VfsBackend,
    pub src_path: &'a Path,
    pub vault_root: &'a Path,
    pub hash: HashAlgorithm,
    pub import_config: ImportConfig,
    pub source_name: String,
    pub crc_buffer_size: usize,
}

#[derive(Debug)]
enum RecheckStatus {
    Ok,
    Mismatch,
    Error(String),
}

#[derive(Debug)]
struct RecheckResult {
    src_path: std::path::PathBuf,
    vault_path: Option<std::path::PathBuf>,
    status: RecheckStatus,
}

/// Run standalone recheck for a local source directory.
pub fn run_recheck(opts: RecheckOptions, db: &Db) -> anyhow::Result<ImportSummary> {
    let session_id = session_id_now();

    let exts: Vec<&str> = opts
        .import_config
        .allowed_extensions
        .iter()
        .map(|s| s.as_str())
        .collect();

    use crate::vfs::system::SystemFs;
    let src_fs = SystemFs::open(&opts.source)?;
    let dir_entries = src_fs.walk(Path::new(""), &exts)?;
    let total = dir_entries.len();

    if total == 0 {
        eprintln!("{} No files found in source directory", style("Warning:").yellow().bold());
        return Ok(ImportSummary {
            total: 0,
            ..Default::default()
        });
    }

    // Stage B: CRC32C fingerprint
    let scan_bar = ProgressBar::new(total as u64);
    scan_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.blue} [{bar:40}] {pos}/{len}")
            .unwrap()
            .progress_chars("=> "),
    );
    scan_bar.set_prefix("Scanning");

    let crcs: Vec<(DirEntry, Result<u32, String>)> = dir_entries
        .into_par_iter()
        .map(|e| {
            let abs = opts.source.join(&e.path);
            let result = crc32c_region(&abs, 0, 65536).map_err(|err| err.to_string());
            scan_bar.inc(1);
            (e, result)
        })
        .collect();
    scan_bar.finish_and_clear();

    eprintln!(
        "{} {} files in {}",
        style("Scanning").bold().cyan(),
        style(total).cyan(),
        style(opts.source.display()).dim()
    );

    let scan_entries: Vec<ScanEntry> = crcs
        .into_iter()
        .map(|(e, crc_result)| {
            let abs = opts.source.join(e.path);
            let crc = match crc_result {
                Err(err) => {
                    return ScanEntry {
                        src_path: abs,
                        size: e.size,
                        mtime_ms: e.mtime_ms,
                        crc32c: 0,
                        status: FileStatus::Failed(err),
                    };
                }
                Ok(v) => v,
            };
            let cached = db.lookup_by_crc32c(e.size as i64, crc).unwrap_or(None);
            let status = if cached.is_some() {
                FileStatus::LikelyCacheDuplicate
            } else {
                FileStatus::LikelyNew
            };
            ScanEntry {
                src_path: abs,
                size: e.size,
                mtime_ms: e.mtime_ms,
                crc32c: crc,
                status,
            }
        })
        .collect();

    let likely_dup = scan_entries
        .iter()
        .filter(|e| e.status == FileStatus::LikelyCacheDuplicate)
        .count();
    let failed_b = scan_entries
        .iter()
        .filter(|e| matches!(e.status, FileStatus::Failed(_)))
        .count();

    eprintln!();
    eprintln!("{}", style("Pre-flight:").bold());
    eprintln!(
        "  {}  {}",
        style(format!("Likely duplicate: {:>6}", likely_dup)).yellow(),
        style("already in vault (cache hit)").dim()
    );
    if failed_b > 0 {
        eprintln!(
            "  {}",
            style(format!("Errors:           {:>6}", failed_b)).red()
        );
    }

    if likely_dup == 0 {
        eprintln!();
        eprintln!("No cache hits found — nothing to recheck.");
        return Ok(ImportSummary {
            total,
            duplicate: 0,
            failed: failed_b,
            all_cache_hit: false,
            ..Default::default()
        });
    }

    // Perform hash-based recheck
    let check_entries: Vec<&ScanEntry> = scan_entries
        .iter()
        .filter(|e| e.status == FileStatus::LikelyCacheDuplicate)
        .collect();

    let bar = ProgressBar::new(check_entries.len() as u64);
    bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.cyan} [{bar:40}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("=> "),
    );
    bar.set_prefix("Recheck  ");

    let src_hashes: Vec<(std::path::PathBuf, u64, u32, Result<Vec<u8>, String>)> = check_entries
        .into_par_iter()
        .map(|e| {
            let filename = e
                .src_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            bar.set_message(filename);
            let hash = match &opts.hash {
                HashAlgorithm::Xxh3_128 => {
                    xxh3_128_file(&e.src_path).map(|h| h.to_bytes().to_vec())
                }
                HashAlgorithm::Sha256 => sha256_file(&e.src_path).map(|h| h.to_bytes().to_vec()),
            };
            bar.inc(1);
            (e.src_path.clone(), e.size, e.crc32c, hash.map_err(|e| e.to_string()))
        })
        .collect();

    bar.finish_and_clear();

    // Compare against vault (single-threaded, DB is !Sync)
    let mut results: Vec<RecheckResult> = Vec::with_capacity(src_hashes.len());
    for (src_path, size, crc32c, src_hash) in src_hashes {
        let src_hash = match src_hash {
            Ok(h) => h,
            Err(err) => {
                results.push(RecheckResult {
                    src_path,
                    vault_path: None,
                    status: RecheckStatus::Error(format!("source hash failed: {err}")),
                });
                continue;
            }
        };

        let db_row = match db.lookup_by_crc32c(size as i64, crc32c) {
            Ok(Some(row)) => row,
            _ => {
                results.push(RecheckResult {
                    src_path,
                    vault_path: None,
                    status: RecheckStatus::Error("no matching vault entry".into()),
                });
                continue;
            }
        };

        let vault_path = opts.vault_root.join(&db_row.path);

        // Always compute the vault file's current hash from disk — do NOT trust
        // the hash stored in the database. The whole point of recheck is to
        // detect whether the vault file has been corrupted since import.
        let vault_hash = match &opts.hash {
            HashAlgorithm::Xxh3_128 => {
                xxh3_128_file(&vault_path).map(|h| h.to_bytes().to_vec())
            }
            HashAlgorithm::Sha256 => sha256_file(&vault_path).map(|h| h.to_bytes().to_vec()),
        };

        let status = match vault_hash {
            Ok(h) if h == src_hash => RecheckStatus::Ok,
            Ok(_) => RecheckStatus::Mismatch,
            Err(err) => RecheckStatus::Error(format!("vault hash failed: {err}")),
        };

        results.push(RecheckResult {
            src_path,
            vault_path: Some(vault_path),
            status,
        });
    }

    write_report(&opts.source.display().to_string(), &session_id, &opts.hash, &results, &opts.vault_root)
}

/// Compute a full-file hash from a generic reader.
fn hash_from_reader<R: Read>(mut reader: R, algo: &HashAlgorithm) -> std::io::Result<Vec<u8>> {
    const BUF: usize = 4 * 1024 * 1024;
    let mut buf = vec![0u8; BUF];
    match algo {
        HashAlgorithm::Xxh3_128 => {
            use xxhash_rust::xxh3::Xxh3;
            let mut hasher = Xxh3::new();
            loop {
                let n = reader.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            let digest = hasher.digest128();
            let low = digest as u64;
            let high = (digest >> 64) as u64;
            let mut b = [0u8; 16];
            b[..8].copy_from_slice(&low.to_le_bytes());
            b[8..].copy_from_slice(&high.to_le_bytes());
            Ok(b.to_vec())
        }
        HashAlgorithm::Sha256 => {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            loop {
                let n = reader.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            let result = hasher.finalize();
            Ok(result.to_vec())
        }
    }
}

/// Compute CRC32 from VFS backend file.
fn crc32c_from_backend(
    backend: &dyn VfsBackend,
    entry: &DirEntry,
    buffer_size: usize,
) -> Result<u32, VfsError> {
    let mut reader = backend.open_read(&entry.path)?;
    let mut buffer = vec![0u8; buffer_size];
    let n = reader.read(&mut buffer).map_err(VfsError::Io)?;
    buffer.truncate(n);
    Ok(crc32fast::hash(&buffer))
}

/// Run standalone recheck from a VFS backend (MTP, etc.).
pub fn run_vfs_recheck(opts: VfsRecheckOptions, db: &Db) -> anyhow::Result<ImportSummary> {
    let session_id = session_id_now();

    let exts: Vec<&str> = opts
        .import_config
        .allowed_extensions
        .iter()
        .map(|s| s.as_str())
        .collect();

    let dir_entries = opts.src_backend.walk(opts.src_path, &exts)?;
    let total = dir_entries.len();

    if total == 0 {
        eprintln!(
            "{} No files found in source",
            style("Warning:").yellow().bold()
        );
        return Ok(ImportSummary {
            total: 0,
            ..Default::default()
        });
    }

    let parallel = opts.src_backend.is_parallel_capable();

    let scan_bar = ProgressBar::new(total as u64);
    scan_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.blue} [{bar:40}] {pos}/{len}")
            .unwrap()
            .progress_chars("=> "),
    );
    scan_bar.set_prefix("Scanning");

    let crcs: Vec<(DirEntry, Result<u32, String>)> = if parallel {
        dir_entries
            .into_par_iter()
            .map(|e| {
                let result = crc32c_from_backend(opts.src_backend, &e, opts.crc_buffer_size)
                    .map_err(|e| e.to_string());
                scan_bar.inc(1);
                (e, result)
            })
            .collect()
    } else {
        dir_entries
            .into_iter()
            .map(|e| {
                let result = crc32c_from_backend(opts.src_backend, &e, opts.crc_buffer_size)
                    .map_err(|e| e.to_string());
                scan_bar.inc(1);
                (e, result)
            })
            .collect()
    };
    scan_bar.finish_and_clear();

    eprintln!(
        "{} {} files from {}",
        style("Scanning").bold().cyan(),
        style(total).cyan(),
        style(&opts.source_name).dim()
    );

    let scan_entries: Vec<ScanEntry> = crcs
        .into_iter()
        .map(|(e, crc_result)| {
            let crc = match crc_result {
                Err(err) => {
                    return ScanEntry {
                        src_path: e.path.clone(),
                        size: e.size,
                        mtime_ms: e.mtime_ms,
                        crc32c: 0,
                        status: FileStatus::Failed(err),
                    };
                }
                Ok(v) => v,
            };
            let cached = db.lookup_by_crc32c(e.size as i64, crc).unwrap_or(None);
            let status = if cached.is_some() {
                FileStatus::LikelyCacheDuplicate
            } else {
                FileStatus::LikelyNew
            };
            ScanEntry {
                src_path: e.path,
                size: e.size,
                mtime_ms: e.mtime_ms,
                crc32c: crc,
                status,
            }
        })
        .collect();

    let likely_dup = scan_entries
        .iter()
        .filter(|e| e.status == FileStatus::LikelyCacheDuplicate)
        .count();
    let failed_b = scan_entries
        .iter()
        .filter(|e| matches!(e.status, FileStatus::Failed(_)))
        .count();

    eprintln!();
    eprintln!("{}", style("Pre-flight:").bold());
    eprintln!(
        "  {}  {}",
        style(format!("Likely duplicate: {:>6}", likely_dup)).yellow(),
        style("already in vault (cache hit)").dim()
    );
    if failed_b > 0 {
        eprintln!(
            "  {}",
            style(format!("Errors:           {:>6}", failed_b)).red()
        );
    }

    if likely_dup == 0 {
        eprintln!();
        eprintln!("No cache hits found — nothing to recheck.");
        return Ok(ImportSummary {
            total,
            duplicate: 0,
            failed: failed_b,
            all_cache_hit: false,
            ..Default::default()
        });
    }

    let check_entries: Vec<&ScanEntry> = scan_entries
        .iter()
        .filter(|e| e.status == FileStatus::LikelyCacheDuplicate)
        .collect();

    let bar = ProgressBar::new(check_entries.len() as u64);
    bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.cyan} [{bar:40}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("=> "),
    );
    bar.set_prefix("Recheck  ");

    let compute = |e: &ScanEntry| {
        let filename = e
            .src_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        bar.set_message(filename);
        let result = opts
            .src_backend
            .open_read(&e.src_path)
            .map_err(|e| e.to_string())
            .and_then(|r| hash_from_reader(r, &opts.hash).map_err(|e| e.to_string()));
        bar.inc(1);
        (e.src_path.clone(), e.size, e.crc32c, result)
    };

    let src_hashes: Vec<(std::path::PathBuf, u64, u32, Result<Vec<u8>, String>)> = if parallel {
        check_entries.into_par_iter().map(compute).collect()
    } else {
        check_entries.into_iter().map(compute).collect()
    };

    bar.finish_and_clear();

    let mut results: Vec<RecheckResult> = Vec::with_capacity(src_hashes.len());
    for (src_path, size, crc32c, src_hash) in src_hashes {
        let src_hash = match src_hash {
            Ok(h) => h,
            Err(err) => {
                results.push(RecheckResult {
                    src_path,
                    vault_path: None,
                    status: RecheckStatus::Error(format!("source hash failed: {err}")),
                });
                continue;
            }
        };

        let db_row = match db.lookup_by_crc32c(size as i64, crc32c) {
            Ok(Some(row)) => row,
            _ => {
                results.push(RecheckResult {
                    src_path,
                    vault_path: None,
                    status: RecheckStatus::Error("no matching vault entry".into()),
                });
                continue;
            }
        };

        let vault_path = opts.vault_root.join(&db_row.path);

        // Always compute the vault file's current hash from disk.
        let vault_hash = match &opts.hash {
            HashAlgorithm::Xxh3_128 => {
                xxh3_128_file(&vault_path).map(|h| h.to_bytes().to_vec())
            }
            HashAlgorithm::Sha256 => sha256_file(&vault_path).map(|h| h.to_bytes().to_vec()),
        };

        let status = match vault_hash {
            Ok(h) if h == src_hash => RecheckStatus::Ok,
            Ok(_) => RecheckStatus::Mismatch,
            Err(err) => RecheckStatus::Error(format!("vault hash failed: {err}")),
        };

        results.push(RecheckResult {
            src_path,
            vault_path: Some(vault_path),
            status,
        });
    }

    write_report(
        &opts.source_name,
        &session_id,
        &opts.hash,
        &results,
        &opts.vault_root,
    )
}

fn write_report(
    source_name: &str,
    session_id: &str,
    hash: &HashAlgorithm,
    results: &[RecheckResult],
    vault_root: &Path,
) -> anyhow::Result<ImportSummary> {
    let ok_count = results
        .iter()
        .filter(|r| matches!(r.status, RecheckStatus::Ok))
        .count();
    let mismatch_count = results
        .iter()
        .filter(|r| matches!(r.status, RecheckStatus::Mismatch))
        .count();
    let error_count = results
        .iter()
        .filter(|r| matches!(r.status, RecheckStatus::Error(_)))
        .count();

    let staging_dir = vault_root.join(".svault").join("staging");
    let _ = fs::create_dir_all(&staging_dir);
    let report_path = staging_dir.join(format!("recheck-{session_id}.txt"));
    let mut report = String::new();
    report.push_str(&format!("# Recheck report: {source_name}\n"));
    report.push_str(&format!("# Session: {session_id}\n"));
    report.push_str(&format!("# Hash algorithm: {hash}\n"));
    report.push_str("#\n");
    report.push_str("# STATUS    SOURCE -> VAULT_PATH\n");
    for r in results {
        let line = match &r.status {
            RecheckStatus::Ok => format!(
                "OK          {} -> {}\n",
                r.src_path.display(),
                r.vault_path.as_ref().unwrap().display()
            ),
            RecheckStatus::Mismatch => format!(
                "MISMATCH    {} -> {}\n",
                r.src_path.display(),
                r.vault_path.as_ref().unwrap().display()
            ),
            RecheckStatus::Error(msg) => {
                format!("ERROR       {} ({})\n", r.src_path.display(), msg)
            }
        };
        report.push_str(&line);
    }
    let _ = fs::write(&report_path, report);

    eprintln!();
    eprintln!("{}", style("Recheck complete:").bold());
    eprintln!(
        "  {} {}",
        style(ok_count).green(),
        style("files match vault").dim()
    );
    if mismatch_count > 0 {
        eprintln!(
            "  {} {}",
            style(mismatch_count).red().bold(),
            style("files differ from vault").dim()
        );
    }
    if error_count > 0 {
        eprintln!(
            "  {} {}",
            style(error_count).red(),
            style("errors during check").dim()
        );
    }
    eprintln!();
    eprintln!("{} {}", style("Report:").bold(), report_path.display());
    if mismatch_count > 0 {
        eprintln!();
        eprintln!(
            "{}",
            style("Differences were found between source and vault.").yellow()
        );
        eprintln!(
            "{}",
            style("Review the report, delete the incorrect side manually, and re-import.").yellow()
        );
    }

    Ok(ImportSummary {
        total: results.len(),
        imported: 0,
        duplicate: ok_count,
        failed: mismatch_count + error_count,
        manifest_path: Some(report_path),
        all_cache_hit: true,
    })
}
