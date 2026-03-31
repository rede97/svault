//! Import pipeline for generic VFS backends (supports MTP, local, etc.)

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::config::{HashAlgorithm, ImportConfig, RecheckMode};
use crate::db::Db;
use crate::hash::{sha256_file, xxh3_128_file};
use crate::vfs::{DirEntry, VfsBackend, VfsError};


use super::path::resolve_dest_path;
use super::types::{FileStatus, ImportSummary, ScanEntry};
use super::utils::session_id_now;

/// Options for VFS-based import (supports MTP, local, etc.)
pub struct VfsImportOptions<'a> {
    /// Source VFS backend
    pub src_backend: &'a dyn VfsBackend,
    /// Source root path on the backend
    pub src_path: &'a Path,
    /// Vault root directory
    pub vault_root: &'a Path,
    /// Hash algorithm
    pub hash: HashAlgorithm,
    /// Recheck mode
    pub recheck: RecheckMode,
    /// Dry run
    pub dry_run: bool,
    /// Skip confirmation
    pub yes: bool,
    /// Show duplicates
    pub show_dup: bool,
    /// Import configuration
    pub import_config: ImportConfig,
    /// Source display name (for progress messages)
    pub source_name: String,
}

/// Compute CRC32 from VFS backend file (first 64KB).
fn crc32c_from_backend(backend: &dyn VfsBackend, entry: &DirEntry) -> Result<u32, VfsError> {
    let mut reader = backend.open_read(&entry.path)?;
    let mut buffer = vec![0u8; 65536];
    let n = reader.read(&mut buffer).map_err(VfsError::Io)?;
    buffer.truncate(n);
    Ok(crc32fast::hash(&buffer))
}

/// Run import from a VFS backend.
pub fn run_vfs_import(opts: VfsImportOptions, db: &Db) -> Result<ImportSummary> {
    let session_id = session_id_now();

    // Stage A: Scan source
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

    // Stage B: CRC32 fingerprint
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
            let result = crc32c_from_backend(opts.src_backend, &e).map_err(|e| e.to_string());
            scan_bar.inc(1);
            (e, result)
        })
        .collect();
    scan_bar.finish_and_clear();

    // Display
    eprintln!(
        "{} {} files from {}",
        style("Scanning").bold().cyan(),
        style(total).cyan(),
        style(&opts.source_name).dim()
    );

    let scan_entries: Vec<ScanEntry> = crcs
        .into_iter()
        .map(|(e, crc_result)| {
            let rel_path = e
                .path
                .strip_prefix(opts.src_path)
                .unwrap_or(&e.path)
                .display()
                .to_string();

            let crc = match crc_result {
                Err(err) => {
                    eprintln!("  {} {}", style("Error").red(), style(&rel_path).dim());
                    return ScanEntry {
                        src_path: e.path,
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

            match status {
                FileStatus::LikelyNew => {
                    eprintln!("  {} {}", style("Found").green(), style(&rel_path).dim());
                }
                FileStatus::LikelyCacheDuplicate if opts.show_dup => {
                    eprintln!(
                        "  {}",
                        style(format!("Duplicate: {}", &rel_path)).yellow()
                    );
                }
                _ => {}
            }

            ScanEntry {
                src_path: e.path,
                size: e.size,
                mtime_ms: e.mtime_ms,
                crc32c: crc,
                status,
            }
        })
        .collect();

    // Summary
    let likely_new: Vec<&ScanEntry> = scan_entries
        .iter()
        .filter(|e| e.status == FileStatus::LikelyNew)
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
        style(format!("Likely new:       {:>6}", likely_new.len())).green(),
        style("will be imported").dim()
    );
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

    if likely_new.is_empty() {
        eprintln!();
        eprintln!(
            "All {} files matched cache (no new files detected).",
            total
        );
        return Ok(ImportSummary {
            total,
            duplicate: likely_dup,
            failed: failed_b,
            all_cache_hit: true,
            ..Default::default()
        });
    }

    // Staging
    let staging_dir = opts.vault_root.join(".svault").join("staging");
    std::fs::create_dir_all(&staging_dir)?;
    let staging_path = staging_dir.join(format!("import-{session_id}.txt"));

    // Convert to staging format
    let entries_for_staging: Vec<(String, u64, i64, u32, FileStatus)> = scan_entries
        .iter()
        .map(|e| {
            let rel = e
                .src_path
                .strip_prefix(opts.src_path)
                .unwrap_or(&e.src_path)
                .display()
                .to_string();
            (rel, e.size, e.mtime_ms, e.crc32c, e.status.clone())
        })
        .collect();

    write_staging_vfs(&staging_path, &opts.source_name, &session_id, &entries_for_staging)?;

    eprintln!();
    eprintln!(
        "{} {}",
        style("Staging list:").bold(),
        style(staging_path.display()).dim()
    );

    if !opts.yes && !opts.dry_run {
        eprint!("{}", style("Proceed with import? [y/N] ").bold());
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
            eprintln!("{}", style("Aborted. Staging list kept at:").yellow());
            eprintln!("  {}", staging_path.display());
            return Ok(ImportSummary {
                total,
                duplicate: likely_dup,
                failed: failed_b,
                all_cache_hit: false,
                ..Default::default()
            });
        }
    }

    let pending_path = opts
        .vault_root
        .join(".svault")
        .join(format!("import-{session_id}.pending"));
    write_pending_vfs(&pending_path, &opts.source_name, &session_id, &entries_for_staging)?;

    if opts.dry_run {
        eprintln!("\n(dry-run: no files copied)");
        return Ok(ImportSummary {
            total,
            duplicate: likely_dup,
            failed: failed_b,
            ..Default::default()
        });
    }

    // Stage C: Copy files
    use crate::vfs::system::SystemFs;
    let dst_fs = SystemFs::open(opts.vault_root)
        .map_err(|e| anyhow::anyhow!("cannot open vault: {e}"))?;

    let copy_errors: Arc<Mutex<HashMap<std::path::PathBuf, String>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let copy_bar = ProgressBar::new(likely_new.len() as u64);
    copy_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.green} [{bar:40}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("=> "),
    );
    copy_bar.set_prefix("Copying  ");

    // Copy files
    let copied: Vec<(std::path::PathBuf, std::path::PathBuf, u64, i64, u32)> = likely_new
        .iter()
        .filter_map(|e| {
            let rel = e
                .src_path
                .strip_prefix(opts.src_path)
                .unwrap_or(&e.src_path);
            let filename = rel
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| rel.display().to_string());

            // Read EXIF from source via VFS
            let (taken_ms, device) = read_exif_from_vfs(opts.src_backend, &e.src_path, e.mtime_ms);
            let dest_rel = resolve_dest_path(
                &opts.import_config.path_template,
                rel,
                taken_ms,
                &device,
            );
            let dest_path = opts.vault_root.join(&dest_rel);

            copy_bar.set_message(filename.clone());

            // Create parent dirs and copy
            if let Some(parent) = dest_path.parent() {
                if let Err(err) = dst_fs.create_dir_all(parent) {
                    let mut errors = copy_errors.lock().unwrap();
                    errors.insert(e.src_path.clone(), err.to_string());
                    copy_bar.inc(1);
                    return None;
                }
            }

            // Use VFS copy_to
            match opts.src_backend.copy_to(&e.src_path, &dst_fs, &dest_path) {
                Ok(_) => {
                    copy_bar.inc(1);
                    Some((e.src_path.clone(), dest_path, e.size, taken_ms, e.crc32c))
                }
                Err(err) => {
                    let mut errors = copy_errors.lock().unwrap();
                    errors.insert(e.src_path.clone(), err.to_string());
                    copy_bar.inc(1);
                    None
                }
            }
        })
        .collect();

    copy_bar.finish_and_clear();

    // Report copy errors
    let copy_errs = copy_errors.lock().unwrap();
    if !copy_errs.is_empty() {
        eprintln!("\n{}", style("Copy errors:").red().bold());
        for (src, err) in copy_errs.iter() {
            eprintln!("  {} {} - {}", style("✗").red(), src.display(), err);
        }
    }
    drop(copy_errs);

    // Stage D: Hash verification
    let verify_bar = ProgressBar::new(copied.len() as u64);
    verify_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.cyan} [{bar:40}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("=> "),
    );
    verify_bar.set_prefix("Verifying");

    let verified: Vec<(std::path::PathBuf, std::path::PathBuf, u64, i64, u32, Vec<u8>)> = copied
        .into_par_iter()
        .filter_map(|(src, dest, size, taken_ms, crc)| {
            let filename = src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            verify_bar.set_message(filename);

            let hash_result = match opts.hash {
                HashAlgorithm::Xxh3_128 => {
                    xxh3_128_file(&dest).map(|h| h.to_bytes().to_vec())
                }
                HashAlgorithm::Sha256 => {
                    sha256_file(&dest).map(|h| h.to_bytes().to_vec())
                }
            };

            verify_bar.inc(1);
            match hash_result {
                Ok(hash) => Some((src, dest, size, taken_ms, crc, hash)),
                Err(_) => None,
            }
        })
        .collect();

    verify_bar.finish_and_clear();

    // Stage E: DB insert and manifest
    let imported_at = chrono::Utc::now().timestamp_millis();
    for (_src, dest, size, taken_ms, crc, hash) in verified.iter() {
        let relpath = dest.strip_prefix(opts.vault_root).unwrap_or(dest);
        let relpath_str = relpath.to_string_lossy();

        let (xxh3_bytes, sha256_bytes) = match opts.hash {
            HashAlgorithm::Xxh3_128 => (Some(hash.as_slice()), None),
            HashAlgorithm::Sha256 => (None, Some(hash.as_slice())),
        };

        let _ = db.insert_file_row(
            &relpath_str,
            *size as i64,
            *taken_ms,
            Some(*crc),
            xxh3_bytes,
            sha256_bytes,
            "imported",
            imported_at,
        );
    }

    let imported = verified.len();
    let manifest_path = opts
        .vault_root
        .join(".svault")
        .join(format!("import-{session_id}.manifest"));
    write_manifest_vfs(&manifest_path, &verified)?;

    // Delete pending file
    let _ = std::fs::remove_file(&pending_path);

    // Cleanup staging
    let _ = std::fs::remove_file(&staging_path);

    // Summary
    eprintln!();
    eprintln!("{}", style("Import complete:").bold().green());
    eprintln!(
        "  {} {}",
        style(imported).green().bold(),
        style("files imported").dim()
    );
    eprintln!(
        "  {} {}",
        style(likely_dup).yellow(),
        style("duplicates skipped").dim()
    );
    eprintln!(
        "  {} {}",
        style(failed_b + copy_errors.lock().unwrap().len()).red(),
        style("failed").dim()
    );
    eprintln!();
    eprintln!("{} {}", style("Manifest:").bold(), manifest_path.display());

    Ok(ImportSummary {
        total,
        imported,
        duplicate: likely_dup,
        failed: failed_b + copy_errors.lock().unwrap().len(),
        manifest_path: Some(manifest_path),
        all_cache_hit: false,
    })
}

/// Read EXIF data from VFS file.
fn read_exif_from_vfs(backend: &dyn VfsBackend, path: &Path, fallback_ms: i64) -> (i64, String) {
    match backend.open_read(path) {
        Ok(mut reader) => {
            // Read first 64KB where EXIF usually is
            let mut buf = vec![0u8; 65536];
            if let Ok(n) = reader.read(&mut buf) {
                buf.truncate(n);
                parse_exif_from_buffer(&buf, fallback_ms)
            } else {
                (fallback_ms, "Unknown".to_string())
            }
        }
        Err(_) => (fallback_ms, "Unknown".to_string()),
    }
}

/// Parse EXIF from buffer.
fn parse_exif_from_buffer(buf: &[u8], fallback_ms: i64) -> (i64, String) {
    use exif::{In, Reader, Tag, Value};

    let Ok(exif) = Reader::new().read_raw(buf.to_vec()) else {
        return (fallback_ms, "Unknown".to_string());
    };

    // Date: prefer DateTimeOriginal, fallback to DateTime
    let taken_ms = exif
        .get_field(Tag::DateTimeOriginal, In::PRIMARY)
        .or_else(|| exif.get_field(Tag::DateTime, In::PRIMARY))
        .and_then(|f| {
            if let Value::Ascii(ref vec) = f.value {
                vec.first().and_then(|b| {
                    let s = std::str::from_utf8(b).ok()?;
                    parse_exif_datetime_ms(s)
                })
            } else {
                None
            }
        })
        .unwrap_or(fallback_ms);

    // Device: "Make Model"
    let make = exif
        .get_field(Tag::Make, In::PRIMARY)
        .and_then(|f| exif_ascii_first(&f.value))
        .unwrap_or_default();
    let model = exif
        .get_field(Tag::Model, In::PRIMARY)
        .and_then(|f| exif_ascii_first(&f.value))
        .unwrap_or_default();

    let device = if make.is_empty() && model.is_empty() {
        "Unknown".to_string()
    } else {
        let raw = if make.is_empty() {
            model
        } else if model.starts_with(&make) {
            model
        } else {
            format!("{make} {model}")
        };
        raw.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim()
            .to_string()
    };

    (taken_ms, device)
}

fn exif_ascii_first(v: &exif::Value) -> Option<String> {
    if let exif::Value::Ascii(vec) = v {
        vec.first()
            .and_then(|b| std::str::from_utf8(b).ok())
            .map(|s| s.trim_end_matches('\0').trim().to_string())
    } else {
        None
    }
}

/// Parse EXIF datetime string `"YYYY:MM:DD HH:MM:SS"` → Unix milliseconds.
fn parse_exif_datetime_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let year: i64 = std::str::from_utf8(&b[0..4]).ok()?.parse().ok()?;
    let month: i64 = std::str::from_utf8(&b[5..7]).ok()?.parse().ok()?;
    let day: i64 = std::str::from_utf8(&b[8..10]).ok()?.parse().ok()?;
    let hour: i64 = std::str::from_utf8(&b[11..13]).ok()?.parse().ok()?;
    let min: i64 = std::str::from_utf8(&b[14..16]).ok()?.parse().ok()?;
    let sec: i64 = std::str::from_utf8(&b[17..19]).ok()?.parse().ok()?;
    let days = ymd_to_days(year as i32, month as u32, day as u32)?;
    let secs = days * 86400 + hour * 3600 + min * 60 + sec;
    Some(secs * 1000)
}

/// Calendar date → days since 1970-01-01.
fn ymd_to_days(y: i32, m: u32, d: u32) -> Option<i64> {
    let m = m as i32;
    let d = d as i32;
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let m_adj = if m > 2 { (m - 3) as u32 } else { (m + 9) as u32 };
    let doy = (153 * m_adj + 2) / 5 + d as u32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era as i64) * 146097 + doe as i64 - 719468)
}

/// Write staging file (VFS version).
fn write_staging_vfs(
    path: &std::path::Path,
    source: &str,
    session: &str,
    entries: &[(String, u64, i64, u32, FileStatus)],
) -> Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    writeln!(f, "# Import staging: {source}")?;
    writeln!(f, "# Session: {session}")?;
    writeln!(f, "# Status: NEW / DUP / ERR")?;
    writeln!(f)?;

    for (rel, size, mtime, crc, status) in entries {
        let status_str = match status {
            FileStatus::LikelyNew => "NEW",
            FileStatus::LikelyCacheDuplicate => "DUP",
            FileStatus::Failed(_) => "ERR",
            _ => "???",
        };
        writeln!(f, "{status_str}\t{crc:08x}\t{size}\t{mtime}\t{}", rel)?;
    }
    Ok(())
}

/// Write pending file (VFS version).
fn write_pending_vfs(
    path: &std::path::Path,
    source: &str,
    session: &str,
    entries: &[(String, u64, i64, u32, FileStatus)],
) -> Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    writeln!(f, "# Import pending: {source}")?;
    writeln!(f, "# Session: {session}")?;
    writeln!(f)?;

    for (rel, size, mtime, crc, status) in entries {
        if matches!(status, FileStatus::LikelyNew) {
            writeln!(f, "{crc:08x}\t{size}\t{mtime}\t{}", rel)?;
        }
    }
    Ok(())
}

/// Write manifest file (VFS version).
fn write_manifest_vfs(
    path: &std::path::Path,
    entries: &[(std::path::PathBuf, std::path::PathBuf, u64, i64, u32, Vec<u8>)],
) -> Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    writeln!(f, "# Import manifest")?;
    writeln!(f, "# src_path -> dest_path | size | timestamp | crc32 | hash")?;
    writeln!(f)?;

    for (src, dest, size, taken_ms, crc, hash) in entries {
        let hash_hex = hash.iter().map(|b| format!("{:02x}", b)).collect::<String>();
        writeln!(
            f,
            "{crc:08x}\t{size}\t{taken_ms}\t{hash_hex}\t{} -> {}",
            src.display(),
            dest.display()
        )?;
    }
    Ok(())
}
