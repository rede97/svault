//! Import pipeline (Stages A–E).
//!
//! This module is split into sub-modules:
//! - `types`: ImportOptions, FileStatus, ScanEntry, ImportSummary
//! - `exif`: EXIF metadata extraction (date, device)
//! - `path`: Path template resolution ($year, $mon, etc.)
//! - `staging`: Pending/staging files and manifest writing
//! - `utils`: Time utilities
//! - `vfs_import`: VFS-based import (supports MTP, local, etc.)

pub mod types;
pub mod exif;
pub mod path;
pub mod staging;
pub mod utils;
pub mod vfs_import;

pub use types::{ImportOptions, FileStatus, ScanEntry, ImportSummary};

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use console::style;
use dashmap::DashMap;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::config::HashAlgorithm;
use crate::db::Db;
use crate::hash::{crc32c_region, xxh3_128_file, sha256_file};
use crate::vfs::system::SystemFs;
use crate::vfs::VfsBackend;

use exif::read_exif_date_device;
use path::resolve_dest_path;
use staging::{write_pending, write_staging, write_manifest};
use utils::{unix_now_ms, session_id_now};

/// Run the full import pipeline (Stages A–E).
pub fn run(opts: ImportOptions, db: &Db) -> anyhow::Result<ImportSummary> {
    let session_id = session_id_now();

    // ------------------------------------------------------------------
    // Stage A: directory scan
    // ------------------------------------------------------------------
    let exts: Vec<&str> = opts.import_config.allowed_extensions
        .iter().map(|s| s.as_str()).collect();

    let src_fs = SystemFs::open(&opts.source)
        .map_err(|e| anyhow::anyhow!("cannot open source: {e}"))?;
    let dir_entries = src_fs.walk(Path::new(""), &exts)
        .map_err(|e| anyhow::anyhow!("scan failed: {e}"))?;
    let total = dir_entries.len();

    if total == 0 {
        eprintln!("{} No files found in source directory", style("Warning:").yellow().bold());
        return Ok(ImportSummary { total: 0, ..Default::default() });
    }

    // ------------------------------------------------------------------
    // Stage B: CRC32C fingerprint
    // ------------------------------------------------------------------
    let scan_bar = ProgressBar::new(total as u64);
    scan_bar.set_style(
        ProgressStyle::with_template(
            "{prefix:.bold.blue} [{bar:40}] {pos}/{len}",
        )
        .unwrap()
        .progress_chars("=> "),
    );
    scan_bar.set_prefix("Scanning");

    let crcs: Vec<(crate::vfs::DirEntry, Result<u32, String>)> = dir_entries
        .into_par_iter()
        .map(|e| {
            let abs = opts.source.join(&e.path);
            let result = crc32c_region(&abs, 0, 65536)
                .map_err(|err| err.to_string());
            scan_bar.inc(1);
            (e, result)
        })
        .collect();
    scan_bar.finish_and_clear();

    // Step B2: DB lookup and real-time display (single-threaded)
    eprintln!("{} {} files in {}", 
        style("Scanning").bold().cyan(),
        style(total).cyan(),
        style(opts.source.display()).dim());
    
    let scan_entries: Vec<ScanEntry> = crcs
        .into_iter()
        .map(|(e, crc_result)| {
            let abs = opts.source.join(&e.path);
            let rel_path = e.path.strip_prefix(&opts.source)
                .unwrap_or(&e.path)
                .display()
                .to_string();
            
            let crc = match crc_result {
                Err(err) => {
                    eprintln!("  {} {}", 
                        style("Error").red(), 
                        style(&rel_path).dim());
                    return ScanEntry {
                        src_path: abs, size: e.size, mtime_ms: e.mtime_ms,
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
                    eprintln!("  {} {}", 
                        style("Found").green(), 
                        style(&rel_path).dim());
                }
                FileStatus::LikelyCacheDuplicate if opts.show_dup => {
                    eprintln!("  {} {}", 
                        style("Duplicate").yellow(), 
                        style(&rel_path).dim());
                }
                _ => {}
            }
            
            ScanEntry {
                src_path: abs, size: e.size, mtime_ms: e.mtime_ms, crc32c: crc,
                status,
            }
        })
        .collect();

    let likely_new: Vec<&ScanEntry> = scan_entries.iter()
        .filter(|e| e.status == FileStatus::LikelyNew).collect();
    let likely_dup = scan_entries.iter()
        .filter(|e| e.status == FileStatus::LikelyCacheDuplicate).count();
    let failed_b = scan_entries.iter()
        .filter(|e| matches!(e.status, FileStatus::Failed(_))).count();

    // Pre-flight summary
    eprintln!();
    eprintln!("{}", style("Pre-flight:").bold());
    eprintln!("  {}  {}",
        style(format!("Likely new:       {:>6}", likely_new.len())).green(),
        style("will be imported").dim());
    eprintln!("  {}  {}",
        style(format!("Likely duplicate: {:>6}", likely_dup)).yellow(),
        style("already in vault (cache hit)").dim());
    if failed_b > 0 {
        eprintln!("  {}",
            style(format!("Errors:           {:>6}", failed_b)).red());
    }

    // Early exit: all cache hits
    if likely_new.is_empty() {
        eprintln!();
        eprintln!("All {} files matched cache (no new files detected).", total);
        eprintln!("To verify duplicates, re-run with:");
        eprintln!("  {} EXIF binary comparison (recommended)", style("-R exif ").cyan());
        eprintln!("  {} full-file hash comparison", style("-R hash ").cyan());
        return Ok(ImportSummary {
            total, duplicate: likely_dup, failed: failed_b,
            all_cache_hit: true, ..Default::default()
        });
    }

    // ------------------------------------------------------------------
    // Write staging file + interactive confirmation
    // ------------------------------------------------------------------
    let staging_dir = opts.vault_root.join(".svault").join("staging");
    fs::create_dir_all(&staging_dir)?;
    let staging_path = staging_dir.join(format!("import-{session_id}.txt"));
    write_staging(&staging_path, &opts.source, &session_id, &scan_entries)?;
    eprintln!();
    eprintln!("{} {}",
        style("Staging list:").bold(),
        style(staging_path.display()).dim());

    if !opts.yes && !opts.dry_run {
        eprint!("{}", style("Proceed with import? [y/N] ").bold());
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)
            .map_err(|e| anyhow::anyhow!("failed to read stdin: {e}"))?;
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

    let pending_path = opts.vault_root
        .join(".svault")
        .join(format!("import-{session_id}.pending"));
    write_pending(&pending_path, &opts.source, &session_id, &scan_entries)?;

    if opts.dry_run {
        eprintln!("\n(dry-run: no files copied)");
        return Ok(ImportSummary {
            total,
            duplicate: likely_dup,
            failed: failed_b,
            ..Default::default()
        });
    }

    // ------------------------------------------------------------------
    // Stage C: copy likely_new files
    // ------------------------------------------------------------------
    let vault_archive = opts.vault_root.clone();
    let dst_fs = SystemFs::open(&vault_archive)
        .map_err(|e| anyhow::anyhow!("cannot open vault: {e}"))?;

    // Pre-resolve destination paths in serial to avoid race conditions
    // when multiple files map to the same destination
    let mut prepared_entries: Vec<(std::path::PathBuf, std::path::PathBuf, u64, i64, u32)> = Vec::new();
    let mut assigned_dests: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    

    
    for e in &likely_new {
        let rel = e.src_path.strip_prefix(&opts.source).unwrap_or(&e.src_path);
        let (taken_ms, device) = read_exif_date_device(&e.src_path, e.mtime_ms);
        let dest_rel = resolve_dest_path(
            &opts.import_config.path_template,
            rel,
            taken_ms,
            &device,
        );
        let dest_abs = vault_archive.join(&dest_rel);
        
        // Handle filename conflicts - check both filesystem and already-assigned destinations
        let unique_dest = resolve_unique_dest_path_serial(
            &dst_fs,
            &dest_abs,
            &opts.import_config.rename_template,
            &assigned_dests,
        );
        

        assigned_dests.insert(unique_dest.clone());
        prepared_entries.push((e.src_path.clone(), unique_dest, e.size, e.mtime_ms, e.crc32c));
    }

    let copy_errors: Arc<Mutex<HashMap<std::path::PathBuf, String>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let copy_bar = ProgressBar::new(prepared_entries.len() as u64);
    copy_bar.set_style(
        ProgressStyle::with_template(
            "{prefix:.bold.green} [{bar:40}] {pos}/{len}  {msg}",
        )
        .unwrap()
        .progress_chars("=> "),
    );
    copy_bar.set_prefix("Copying  ");

    let copied: Vec<(std::path::PathBuf, std::path::PathBuf, u64, i64, u32)> = prepared_entries
        .into_par_iter()
        .filter_map(|(src_path, dest_abs, size, mtime_ms, crc32c)| {
            let filename = src_path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            
            let rel = src_path.strip_prefix(&opts.source).unwrap_or(&src_path);

            if let Some(parent) = dest_abs.parent() {
                if let Err(err) = fs::create_dir_all(parent) {
                    copy_errors.lock().unwrap()
                        .insert(src_path.clone(), err.to_string());
                    copy_bar.inc(1);
                    return None;
                }
            }

            match src_fs.copy_to(rel, &dst_fs, &dest_abs) {
                Ok(_) => {
                    // Show destination path relative to vault root
                    // Use progress bar's println for thread-safe output ordering
                    let vault_rel = dest_abs.strip_prefix(&opts.vault_root)
                        .unwrap_or(&dest_abs)
                        .display()
                        .to_string();
                    copy_bar.println(format!("  {} {}",
                        style("Added").green(),
                        style(vault_rel).dim()));
                    copy_bar.set_message(filename);
                    copy_bar.inc(1);
                    Some((src_path, dest_abs, size, mtime_ms, crc32c))
                }
                Err(err) => {
                    copy_errors.lock().unwrap()
                        .insert(src_path, err.to_string());
                    copy_bar.inc(1);
                    None
                }
            }
        })
        .collect();
    copy_bar.finish_and_clear();

    let copy_err_count = copy_errors.lock().unwrap().len();
    let copied_len = copied.len();
    


    // ------------------------------------------------------------------
    // Stage D: strong hash + three-layer dedup
    // ------------------------------------------------------------------
    #[derive(Debug)]
    #[allow(dead_code)]
    struct HashResult {
        src: std::path::PathBuf,
        dest: std::path::PathBuf,
        size: u64,
        mtime_ms: i64,
        crc32c: u32,
        hash_bytes: Vec<u8>,
        is_duplicate: bool,
        dup_reason: Option<String>,
    }

    let hash_bar = ProgressBar::new(copied_len as u64);
    hash_bar.set_style(
        ProgressStyle::with_template(
            "{prefix:.bold.yellow} [{bar:40}] {pos}/{len}  {msg}",
        )
        .unwrap()
        .progress_chars("=> "),
    );
    hash_bar.set_prefix("Hashing  ");

    let hashed: Vec<HashResult> = copied
        .into_par_iter()
        .map(|(src, dest, size, mtime_ms, crc32c)| {
            let filename = dest.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            hash_bar.set_message(filename.clone());
            let hash_bytes = match &opts.hash {
                HashAlgorithm::Xxh3_128 => {
                    match xxh3_128_file(&dest) {
                        Ok(d) => d.to_bytes().to_vec(),
                        Err(e) => {
                            hash_bar.inc(1);
                            return HashResult {
                                src, dest, size, mtime_ms, crc32c,
                                hash_bytes: vec![],
                                is_duplicate: false,
                                dup_reason: Some(format!("hash error: {e}")),
                            };
                        }
                    }
                }
                HashAlgorithm::Sha256 => {
                    match sha256_file(&dest) {
                        Ok(d) => d.to_bytes().to_vec(),
                        Err(e) => {
                            hash_bar.inc(1);
                            return HashResult {
                                src, dest, size, mtime_ms, crc32c,
                                hash_bytes: vec![],
                                is_duplicate: false,
                                dup_reason: Some(format!("hash error: {e}")),
                            };
                        }
                    }
                }
            };
            hash_bar.inc(1);
            HashResult { src, dest, size, mtime_ms, crc32c, hash_bytes, is_duplicate: false, dup_reason: None }
        })
        .collect();
    hash_bar.finish_and_clear();

    // Pass D2: sequential DB lookup + DashMap dedup
    let seen: DashMap<Vec<u8>, std::path::PathBuf> = DashMap::new();
    
    let hash_results: Vec<HashResult> = hashed
        .into_iter()
        .map(|mut r| {
            if r.dup_reason.is_some() {
                return r;
            }
            let existing = db.lookup_by_hash(&r.hash_bytes, &opts.hash).unwrap_or(None);
            if existing.is_some() {
                r.is_duplicate = true;
                r.dup_reason = Some("db".to_string());
                return r;
            }
            use dashmap::mapref::entry::Entry;
            match seen.entry(r.hash_bytes.clone()) {
                Entry::Vacant(v) => { v.insert(r.dest.clone()); }
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
    // Stage E: batch DB write + manifest
    // ------------------------------------------------------------------
    let now_ms = unix_now_ms();
    let mut imported_count = 0usize;
    let mut dup_count = likely_dup;
    let mut fail_count = failed_b + copy_err_count;

    for r in &hash_results {
        if let Some(reason) = &r.dup_reason {
            if reason != "hash error" {
                dup_count += 1;
                let _ = fs::remove_file(&r.dest);
            } else {
                fail_count += 1;
            }
            continue;
        }

        let path_str = r.dest.to_string_lossy().into_owned();
        let (xxh3, sha256) = match &opts.hash {
            HashAlgorithm::Xxh3_128 => (Some(r.hash_bytes.as_slice()), None),
            HashAlgorithm::Sha256 => (None, Some(r.hash_bytes.as_slice())),
        };
        let payload = serde_json::json!({
            "path": path_str,
            "size": r.size,
            "mtime": r.mtime_ms,
        }).to_string();

        let result = db.append_event(
            "file.imported", "file", 0, &payload,
            |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO files \
                     (path, size, mtime, crc32c_val, xxh3_128, sha256, status, imported_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'imported', ?7)",
                    rusqlite::params![
                        path_str,
                        r.size as i64,
                        r.mtime_ms,
                        r.crc32c as i64,
                        xxh3,
                        sha256,
                        now_ms,
                    ],
                )?;
                Ok(())
            },
        );

        match result {
            Ok(()) => imported_count += 1,
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("UNIQUE constraint") {
                    dup_count += 1;
                    let _ = fs::remove_file(&r.dest);
                } else {
                    fail_count += 1;
                    eprintln!("  error inserting {}: {msg}", r.dest.display());
                }
            }
        }
    }

    // Write manifest
    let _manifest_path = write_manifest(
        &opts.vault_root, &session_id, &scan_entries, &hash_results
            .iter().filter(|r| r.dup_reason.is_none()).map(|r| r.dest.clone()).collect::<Vec<_>>(),
    )?;

    // Delete .pending
    let _ = fs::remove_file(&pending_path);

    // Print summary
    eprintln!("{} {} file(s) imported", 
        style("Finished:").bold().green(),
        style(imported_count).green());
    
    if dup_count > 0 {
        eprintln!("         {} duplicate(s) skipped", 
            style(dup_count).yellow());
    }
    if fail_count > 0 {
        eprintln!("         {} file(s) failed", 
            style(fail_count).red());
    }

    Ok(ImportSummary {
        total,
        imported: imported_count,
        duplicate: dup_count,
        failed: fail_count,
        manifest_path: Some(_manifest_path),
        all_cache_hit: false,
    })
}

/// Resolve unique destination path, checking both filesystem and in-memory assigned destinations.
/// Used during serial preparation phase to avoid conflicts between files in the same batch.
fn resolve_unique_dest_path_serial(
    dst_fs: &SystemFs,
    dest_path: &Path,
    rename_template: &str,
    assigned: &std::collections::HashSet<std::path::PathBuf>,
) -> std::path::PathBuf {
    use std::path::Path;
    
    // Check filesystem first
    match dst_fs.exists(dest_path) {
        Ok(true) => {
            // Destination exists on filesystem - need to find unique name
            return resolve_dest_conflict(dst_fs, dest_path, rename_template, assigned);
        }
        Ok(false) => {
            // Destination doesn't exist on filesystem, check assigned set
            if !assigned.contains(dest_path) {
                return dest_path.to_path_buf();
            }
            // Destination is already assigned in this batch - need to find unique name
        }
        Err(_) => return dest_path.to_path_buf(), // On error, try original path
    }

    // Destination conflicts with assigned set - generate unique name
    resolve_dest_conflict(dst_fs, dest_path, rename_template, assigned)
}

/// Helper to resolve destination conflicts against both filesystem and assigned set.
fn resolve_dest_conflict(
    dst_fs: &SystemFs,
    dest_path: &Path,
    rename_template: &str,
    assigned: &std::collections::HashSet<std::path::PathBuf>,
) -> std::path::PathBuf {
    use std::path::Path;
    
    let parent = dest_path.parent().unwrap_or(Path::new(""));
    let filename = dest_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    
    // Split into stem and extension
    let (stem, ext) = if let Some(pos) = filename.rfind('.') {
        (&filename[..pos], &filename[pos..]) // ext includes the dot
    } else {
        (&filename[..], "")
    };

    // Try incrementing counter until we find a free name
    for n in 1..=9999 {
        let new_filename = rename_template
            .replace("$filename", stem)
            .replace("$ext", ext.trim_start_matches('.'))
            .replace("$n", &n.to_string());
        
        let new_dest = parent.join(&new_filename);
        
        // Check both filesystem and assigned set
        let fs_exists = match dst_fs.exists(&new_dest) {
            Ok(true) => true,
            Ok(false) => false,
            Err(_) => false, // On error, assume doesn't exist
        };
        
        if !fs_exists && !assigned.contains(&new_dest) {
            return new_dest;
        }
    }

    // Fallback: append timestamp if all numbers exhausted
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let fallback_name = format!("{}.{}{}", stem, timestamp, ext);
    parent.join(fallback_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_status_equality() {
        assert_eq!(FileStatus::LikelyNew, FileStatus::LikelyNew);
        assert_ne!(FileStatus::LikelyNew, FileStatus::LikelyCacheDuplicate);
    }
}
