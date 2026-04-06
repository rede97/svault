//! `svault add` — register files already inside the vault.
//!
//! Scans a directory within the vault, computes hashes for any files not
//! already tracked in the database, and inserts them as `file.imported` events.
//!
//! Uses the same three-stage deduplication as `import`:
//! 1. CRC32C fast filter (format-specific regions)
//! 2. RAW ID precise matching (for RAW files with EXIF)
//! 3. Strong hash confirmation (XXH3-128 or SHA-256)

use std::path::Path;

use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::config::{Config, HashAlgorithm};
use crate::db::Db;
use crate::hash::{sha256_file, xxh3_128_file};
use crate::media::crc::compute_checksum;
use crate::media::raw_id::{extract_raw_id_if_raw, get_fingerprint_string, is_raw_file};
use crate::media::MediaFormat;
use crate::vfs::system::SystemFs;
use crate::vfs::VfsBackend;

/// Summary of an `add` operation.
#[derive(Debug, Default)]
pub struct AddSummary {
    pub total: usize,
    pub added: usize,
    pub duplicate: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Options for `svault add`.
pub struct AddOptions {
    pub path: std::path::PathBuf,
    pub vault_root: std::path::PathBuf,
    pub hash: HashAlgorithm,
}

/// Per-file scan result from Stage B.
#[derive(Debug)]
struct ScanResult {
    path: std::path::PathBuf,
    size: u64,
    mtime_ms: i64,
    crc32c: u32,
    raw_unique_id: Option<String>,
    status: ScanStatus,
}

#[derive(Debug)]
enum ScanStatus {
    LikelyNew,
    LikelyDuplicate,
    Failed(String),
}

/// Intermediate result from parallel CRC computation
struct CrcResult {
    path: std::path::PathBuf,
    size: u64,
    mtime_ms: i64,
    crc: Result<u32, String>,
    raw_unique_id: Option<String>,
}

/// Run `add` on a directory inside the vault.
pub fn run_add(opts: AddOptions, db: &Db) -> anyhow::Result<AddSummary> {
    let config = Config::load(&opts.vault_root)?;
    let exts: Vec<&str> = config
        .import
        .allowed_extensions
        .iter()
        .map(|s| s.as_str())
        .collect();

    let fs = SystemFs::open(&opts.path)?;
    let entries = fs.walk(Path::new(""), &exts)?;
    let total = entries.len();

    if total == 0 {
        eprintln!("{} No files found in {}", style("Warning:").yellow().bold(), opts.path.display());
        return Ok(AddSummary::default());
    }

    eprintln!(
        "{} Scanning {} files in {}",
        style("Adding:").bold().cyan(),
        style(total).cyan(),
        style(opts.path.display())
    );

    // ------------------------------------------------------------------
    // Stage B1: CRC32C fingerprint (parallel)
    // ------------------------------------------------------------------
    let scan_bar = ProgressBar::new(total as u64);
    scan_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.cyan} [{bar:40}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("=> "),
    );
    scan_bar.set_prefix("Scanning");

    let crc_results: Vec<CrcResult> = entries
        .into_par_iter()
        .map(|e| {
            let filename = e.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            scan_bar.set_message(filename);

            // Compute format-specific CRC32C
            let format = MediaFormat::from_path(&e.path)
                .unwrap_or(MediaFormat::Unknown(""));
            let crc = compute_checksum(&e.path, &format)
                .map_err(|err| err.to_string());

            // For RAW files, extract unique ID for precise duplicate detection
            let ext = e.path
                .extension()
                .and_then(|ex| ex.to_str())
                .unwrap_or("");
            let raw_unique_id = if is_raw_file(ext) {
                extract_raw_id_if_raw(&e.path)
                    .and_then(|raw_id| get_fingerprint_string(&raw_id))
            } else {
                None
            };

            scan_bar.inc(1);
            CrcResult {
                path: e.path,
                size: e.size,
                mtime_ms: e.mtime_ms,
                crc,
                raw_unique_id,
            }
        })
        .collect();

    scan_bar.finish_and_clear();

    // ------------------------------------------------------------------
    // Stage B2: DB lookup (single-threaded)
    // ------------------------------------------------------------------
    let scan_results: Vec<ScanResult> = crc_results
        .into_iter()
        .map(|r| {
            let crc = match r.crc {
                Err(err) => {
                    return ScanResult {
                        path: r.path,
                        size: r.size,
                        mtime_ms: r.mtime_ms,
                        crc32c: 0,
                        raw_unique_id: None,
                        status: ScanStatus::Failed(err),
                    };
                }
                Ok(v) => v,
            };

            // Get file extension for format-specific lookup
            let ext = r.path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            // DB lookup with CRC32C + RAW ID
            let cached = db.lookup_by_crc32c(
                r.size as i64,
                crc,
                ext,
                r.raw_unique_id.as_deref()
            ).unwrap_or(None);

            let status = if let Some(ref row) = cached {
                // For RAW files with unique IDs, if the IDs don't match, it's not a duplicate
                let is_same_raw_id = match (&r.raw_unique_id, &row.raw_unique_id) {
                    (Some(new_id), Some(existing_id)) => new_id == existing_id,
                    // If we can't compare IDs, fall back to CRC-only behavior
                    _ => true,
                };

                let vault_path = opts.vault_root.join(&row.path);
                if vault_path.exists() && is_same_raw_id {
                    ScanStatus::LikelyDuplicate
                } else {
                    ScanStatus::LikelyNew
                }
            } else {
                ScanStatus::LikelyNew
            };

            ScanResult {
                path: r.path,
                size: r.size,
                mtime_ms: r.mtime_ms,
                crc32c: crc,
                raw_unique_id: r.raw_unique_id,
                status,
            }
        })
        .collect();

    // Report scan results
    let likely_new: Vec<&ScanResult> = scan_results
        .iter()
        .filter(|r| matches!(r.status, ScanStatus::LikelyNew))
        .collect();
    let likely_dup = scan_results
        .iter()
        .filter(|r| matches!(r.status, ScanStatus::LikelyDuplicate))
        .count();
    let failed_b = scan_results
        .iter()
        .filter(|r| matches!(r.status, ScanStatus::Failed(_)))
        .count();

    eprintln!();
    eprintln!("{}", style("Pre-flight:").bold());
    eprintln!(
        "  {}  {}",
        style(format!("Likely new:       {:>6}", likely_new.len())).green(),
        style("will be added")
    );
    if likely_dup > 0 {
        eprintln!(
            "  {}  {}",
            style(format!("Likely duplicate: {:>6}", likely_dup)).yellow(),
            style("already in vault (cache hit)")
        );
    }
    if failed_b > 0 {
        eprintln!(
            "  {}",
            style(format!("Errors:           {:>6}", failed_b)).red()
        );
    }

    // ------------------------------------------------------------------
    // Stage D: strong hash + three-layer dedup (parallel)
    // ------------------------------------------------------------------
    let hash_bar = ProgressBar::new(likely_new.len() as u64);
    hash_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.yellow} [{bar:40}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("=> "),
    );
    hash_bar.set_prefix("Hashing  ");

    let hash_results: Vec<HashResult> = likely_new
        .into_par_iter()
        .filter_map(|r| {
            let filename = r.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            hash_bar.set_message(filename.clone());

            let hash_bytes = match opts.hash {
                HashAlgorithm::Xxh3_128 => match xxh3_128_file(&r.path) {
                    Ok(d) => d.to_bytes().to_vec(),
                    Err(e) => {
                        hash_bar.inc(1);
                        return Some(HashResult {
                            path: r.path.clone(),
                            size: r.size,
                            mtime_ms: r.mtime_ms,
                            crc32c: r.crc32c,
                            raw_unique_id: r.raw_unique_id.clone(),
                            hash_bytes: vec![],
                            is_duplicate: false,
                            dup_reason: Some(format!("hash error: {e}")),
                        });
                    }
                },
                HashAlgorithm::Sha256 => match sha256_file(&r.path) {
                    Ok(d) => d.to_bytes().to_vec(),
                    Err(e) => {
                        hash_bar.inc(1);
                        return Some(HashResult {
                            path: r.path.clone(),
                            size: r.size,
                            mtime_ms: r.mtime_ms,
                            crc32c: r.crc32c,
                            raw_unique_id: r.raw_unique_id.clone(),
                            hash_bytes: vec![],
                            is_duplicate: false,
                            dup_reason: Some(format!("hash error: {e}")),
                        });
                    }
                },
            };

            hash_bar.inc(1);
            Some(HashResult {
                path: r.path.clone(),
                size: r.size,
                mtime_ms: r.mtime_ms,
                crc32c: r.crc32c,
                raw_unique_id: r.raw_unique_id.clone(),
                hash_bytes,
                is_duplicate: false,
                dup_reason: None,
            })
        })
        .collect();

    hash_bar.finish_and_clear();

    // Pass D2: sequential DB lookup + DashMap dedup
    use dashmap::DashMap;
    let seen: DashMap<Vec<u8>, std::path::PathBuf> = DashMap::new();

    let hash_results: Vec<HashResult> = hash_results
        .into_iter()
        .map(|mut r| {
            if r.dup_reason.is_some() {
                return r;
            }

            // Check hash duplicate in DB
            let existing = db.lookup_by_hash(&r.hash_bytes, &opts.hash).unwrap_or(None);
            if let Some(ref row) = existing {
                let vault_path = opts.vault_root.join(&row.path);
                // Allow re-adding the same path even if the hash is already in the DB
                if vault_path != r.path && vault_path.exists() {
                    r.is_duplicate = true;
                    r.dup_reason = Some("db".to_string());
                    return r;
                }
            }

            // Check batch duplicate
            use dashmap::mapref::entry::Entry;
            match seen.entry(r.hash_bytes.clone()) {
                Entry::Vacant(v) => {
                    v.insert(r.path.clone());
                }
                Entry::Occupied(_) => {
                    r.is_duplicate = true;
                    r.dup_reason = Some("batch".to_string());
                    return r;
                }
            }
            r
        })
        .collect();

    // ------------------------------------------------------------------
    // Stage E: batch DB write
    // ------------------------------------------------------------------
    let mut summary = AddSummary {
        total,
        skipped: likely_dup,
        ..Default::default()
    };
    let now_ms = crate::import::utils::unix_now_ms();

    for r in &hash_results {
        let rel_path = r.path.strip_prefix(&opts.vault_root).unwrap_or(&r.path);
        let rel_str = rel_path.to_string_lossy().into_owned();

        // Skip if already tracked by path
        if let Ok(Some(_)) = db.get_file_by_path(&rel_str) {
            summary.skipped += 1;
            continue;
        }

        if let Some(reason) = &r.dup_reason {
            if reason != "hash error" {
                summary.duplicate += 1;
                eprintln!(
                    "  {} {} ({})",
                    style("Duplicate").yellow(),
                    style(rel_path.display()),
                    reason
                );
            } else {
                summary.failed += 1;
                eprintln!(
                    "  {} {} - {}",
                    style("Error").red(),
                    style(rel_path.display()),
                    reason
                );
            }
            continue;
        }

        let (xxh3, sha256) = match &opts.hash {
            HashAlgorithm::Xxh3_128 => (Some(r.hash_bytes.as_slice()), None),
            HashAlgorithm::Sha256 => (None, Some(r.hash_bytes.as_slice())),
        };

        let payload = serde_json::json!({
            "path": rel_str,
            "size": r.size,
            "mtime": r.mtime_ms,
        })
        .to_string();

        if let Err(e) = db.append_event(
            "file.imported",
            "file",
            0,
            &payload,
            |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO files \
                     (path, size, mtime, crc32c, raw_unique_id, xxh3_128, sha256, status, imported_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'imported', ?8)",
                    rusqlite::params![
                        rel_str,
                        r.size as i64,
                        r.mtime_ms,
                        r.crc32c as i64,
                        r.raw_unique_id.as_deref(),
                        xxh3,
                        sha256,
                        now_ms,
                    ],
                )?;
                Ok(())
            },
        ) {
            eprintln!("  {} {} - {}", style("Error").red(), style(rel_path.display()), e);
            summary.failed += 1;
            continue;
        }

        summary.added += 1;
        eprintln!("  {} {}", style("Added").green(), style(rel_path.display()));
    }

    eprintln!();
    eprintln!(
        "{} {} file(s) added",
        style("Finished:").bold().green(),
        style(summary.added).green()
    );
    if summary.duplicate > 0 {
        eprintln!(
            "         {} duplicate(s) skipped",
            style(summary.duplicate).yellow()
        );
    }
    if summary.skipped > 0 {
        eprintln!(
            "         {} already tracked",
            style(summary.skipped)
        );
    }
    if summary.failed > 0 {
        eprintln!(
            "         {} file(s) failed",
            style(summary.failed).red()
        );
    }

    Ok(summary)
}

#[derive(Debug)]
#[allow(dead_code)]
struct HashResult {
    path: std::path::PathBuf,
    size: u64,
    mtime_ms: i64,
    crc32c: u32,
    raw_unique_id: Option<String>,
    hash_bytes: Vec<u8>,
    is_duplicate: bool,
    dup_reason: Option<String>,
}
