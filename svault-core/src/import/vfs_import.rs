//! Import pipeline for generic VFS backends (supports MTP, local, etc.)
//!
//! This module provides the full import pipeline (Stages A-E) for any VFS backend,
//! enabling imports from MTP devices, network filesystems, and local storage.
//!
//! # Concurrency Strategy
//!
//! This pipeline carefully manages concurrency based on the source backend:
//!
//! ## Stage A (Scan) and Stage B (CRC32)
//!
//! These stages use **parallel processing** (`rayon`) for all backends because:
//! - CRC32 computation is CPU-bound
//! - MTP metadata operations (`GetObjectHandles`) have low overhead
//! - The device CPU handles concurrent metadata requests reasonably well
//!
//! ## Stage C (Copy)
//!
//! This stage is **SEQUENTIAL** for MTP backends because:
//! - USB is a single shared pipe; parallel reads just queue up
//! - MTP/PTP protocols are half-duplex request-response
//! - Device-side CPU is the bottleneck, not USB bandwidth
//! - Sequential access with large buffers saturates the pipe optimally
//!
//! For local filesystems, Stage C uses parallel copy with `rayon`.
//!
//! ## Stage D (Hash Verification)
//!
//! Always parallel using `rayon` because it operates on local files in the vault.
//!
//! # Performance Comparison
//!
//! | Stage | Local FS | MTP Device | Notes |
//! |-------|----------|------------|-------|
//! | Scan  | Parallel | Parallel   | Low overhead |
//! | CRC32 | Parallel | Parallel   | CPU-bound |
//! | Copy  | Parallel | **Sequential** | USB contention |
//! | Hash  | Parallel | Parallel   | Local files |

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::config::{HashAlgorithm, ImportConfig, SyncStrategy};
use crate::db::Db;
use crate::hash::{sha256_file, xxh3_128_file};
use crate::media::{extract_raw_id_if_raw, is_raw_file};
use crate::media::raw_id::get_fingerprint_string;
use crate::vfs::{transfer::transfer_file, DirEntry, VfsBackend, VfsError};

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
    /// Dry run
    pub dry_run: bool,
    /// Skip confirmation
    pub yes: bool,
    /// Import configuration
    pub import_config: ImportConfig,
    /// Source display name (for progress messages)
    pub source_name: String,
    /// Transfer strategy
    pub strategy: SyncStrategy,
    /// Force import even if the file is a confirmed duplicate.
    pub force: bool,
    /// Show duplicate files that were skipped during import.
    pub show_dup: bool,
    /// CRC32 fingerprint buffer size in bytes (default: 64KB)
    /// Larger values = more accurate dedup but slower scan
    pub crc_buffer_size: usize,
}

impl<'a> VfsImportOptions<'a> {
    /// Create new import options with defaults
    pub fn new(src_backend: &'a dyn VfsBackend, vault_root: &'a Path) -> Self {
        Self {
            src_backend,
            src_path: Path::new(""),
            vault_root,
            hash: crate::config::HashAlgorithm::Xxh3_128,
            dry_run: false,
            yes: false,
            import_config: crate::config::ImportConfig::default(),
            source_name: String::new(),
            strategy: SyncStrategy::default(),
            force: false,
            show_dup: false,
            crc_buffer_size: 64 * 1024, // 64KB default
        }
    }
    
    /// Set CRC buffer size (for fingerprinting)
    pub fn with_crc_buffer_size(mut self, size: usize) -> Self {
        self.crc_buffer_size = size;
        self
    }
}

/// Compute CRC32 from VFS backend file.
/// Reads first `buffer_size` bytes (default 64KB) for fingerprinting.
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

/// Run import from a VFS backend.
pub fn run_vfs_import(opts: VfsImportOptions, db: &Db) -> Result<ImportSummary> {
    let session_id = session_id_now();

    // Stage A: Scan source (streaming)
    let exts: Vec<&str> = opts
        .import_config
        .allowed_extensions
        .iter()
        .map(|s| s.as_str())
        .collect();

    let mut dir_entries: Vec<_> = opts.src_backend.walk_stream(opts.src_path, &exts)?
        .into_iter()
        .filter_map(|r| r.ok())
        .collect();
    if opts.src_backend.as_system_fs().is_some() {
        let vault_canon = std::fs::canonicalize(opts.vault_root)
            .unwrap_or_else(|_| opts.vault_root.to_path_buf());
        // Ensure vault path ends with '/' for accurate prefix matching
        let vault_prefix = format!("{}/", vault_canon.to_string_lossy());
        // Convert to absolute paths for comparison
        dir_entries.retain(|e| {
            let abs_path = opts.src_path.join(&e.path);
            !abs_path.to_string_lossy().starts_with(&vault_prefix)
        });
    }
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
    // Use parallel processing only if backend supports it (local FS)
    // MTP uses single-thread to avoid USB contention
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

    // Display
    eprintln!(
        "{} {} files from {}",
        style("Scanning").bold().cyan(),
        style(total).cyan(),
        style(&opts.source_name)
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
                    eprintln!("  {} {}", style("Error").red(), style(&rel_path));
                    return ScanEntry {
                        src_path: e.path,
                        size: e.size,
                        mtime_ms: e.mtime_ms,
                        crc32c: 0,
                        status: FileStatus::Failed(err),
                        raw_unique_id: None,
                    };
                }
                Ok(v) => v,
            };

            // Get file extension for format-specific lookup
            let ext = e.path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            
            // For RAW files, extract unique ID for precise duplicate detection
            // Only works for local filesystem (SystemFs); skip for MTP/other backends
            let raw_unique_id = if is_raw_file(ext) && opts.src_backend.as_system_fs().is_some() {
                extract_raw_id_if_raw(&e.path)
                    .and_then(|raw_id| get_fingerprint_string(&raw_id))
            } else {
                None
            };
            
            let cached = db.lookup_by_crc32c(
                e.size as i64, 
                crc, 
                ext,
                raw_unique_id.as_deref()
            ).unwrap_or(None);
            
            let status = if let Some(ref row) = cached {
                // For RAW files with unique IDs, if the IDs don't match, it's not a duplicate
                let is_same_raw_id = match (&raw_unique_id, &row.raw_unique_id) {
                    (Some(new_id), Some(existing_id)) => new_id == existing_id,
                    _ => true,
                };
                
                let vault_path = opts.vault_root.join(&row.path);
                if vault_path.exists() && is_same_raw_id {
                    FileStatus::LikelyCacheDuplicate
                } else {
                    FileStatus::LikelyNew
                }
            } else {
                FileStatus::LikelyNew
            };

            if let FileStatus::LikelyNew = status {
                eprintln!("  {} {}", style("Found").green(), style(&rel_path));
            }

            ScanEntry {
                src_path: e.path,
                size: e.size,
                mtime_ms: e.mtime_ms,
                crc32c: crc,
                status,
                raw_unique_id,
            }
        })
        .collect();

    // Summary
    let likely_new: Vec<&ScanEntry> = scan_entries
        .iter()
        .filter(|e| e.status == FileStatus::LikelyNew || (opts.force && e.status == FileStatus::LikelyCacheDuplicate))
        .collect();
    let likely_dup = scan_entries
        .iter()
        .filter(|e| e.status == FileStatus::LikelyCacheDuplicate && !opts.force)
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
        style("will be imported")
    );
    eprintln!(
        "  {}  {}",
        style(format!("Likely duplicate: {:>6}", likely_dup)).yellow(),
        style("already in vault (cache hit)")
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
        eprintln!("To verify duplicates, run:",);
        eprintln!("  {} <source>", style("svault recheck").cyan());
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
        style(staging_path.display())
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

    // Stage C: Prepare destinations (serial) and copy files (parallel/serial)
    use crate::vfs::system::SystemFs;
    let dst_fs = SystemFs::open(opts.vault_root)
        .map_err(|e| anyhow::anyhow!("cannot open vault: {e}"))?;

    // Pre-resolve destination paths in serial to avoid race conditions
    // when multiple files map to the same destination.
    // We need to track assigned destinations in memory since the filesystem
    // doesn't have the files yet (we're in the preparation phase).
    let mut prepared_entries: Vec<(std::path::PathBuf, std::path::PathBuf, i64, i64, u32)> = Vec::new();
    let mut assigned_dests: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    

    
    for e in &likely_new {
        let rel = e
            .src_path
            .strip_prefix(opts.src_path)
            .unwrap_or(&e.src_path);
        
        // Read EXIF from source via VFS
        let (taken_ms, device) = read_exif_from_vfs(opts.src_backend, &e.src_path, e.mtime_ms);
        let dest_rel = resolve_dest_path(
            &opts.import_config.path_template,
            rel,
            taken_ms,
            &device,
        );
        let dest_path = opts.vault_root.join(&dest_rel);

        // Handle filename conflicts with already-assigned destinations
        // First check filesystem, then check in-memory assigned_dests
        let unique_dest = resolve_unique_dest_path_with_assigned(
            &dst_fs,
            &dest_path,
            &opts.import_config.rename_template,
            &assigned_dests,
        );
        

        assigned_dests.insert(unique_dest.clone());
        prepared_entries.push((e.src_path.clone(), unique_dest, e.size as i64, taken_ms, e.crc32c));
    }

    let copy_errors: Arc<Mutex<HashMap<std::path::PathBuf, String>>> =
        Arc::new(Mutex::new(HashMap::new()));
    
    // Flag to signal disconnection for early abort
    let disconnected: Arc<std::sync::atomic::AtomicBool> = 
        Arc::new(std::sync::atomic::AtomicBool::new(false));
    let disconnect_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    let copy_bar = ProgressBar::new(prepared_entries.len() as u64);
    copy_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.green} [{bar:40}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("=> "),
    );
    copy_bar.set_prefix("Copying  ");

    let transfer_strategies = opts.strategy.to_transfer_strategies();

    // Copy files: parallel for local FS, sequential for MTP
    let copy_op = |(src_path, dest_path, size, taken_ms, crc): (std::path::PathBuf, std::path::PathBuf, i64, i64, u32)| -> Option<(std::path::PathBuf, std::path::PathBuf, i64, i64, u32)> {
        // Check if already disconnected
        if disconnected.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        
        let filename = src_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        copy_bar.set_message(filename.clone());

        // Create parent dirs and copy
        if let Some(parent) = dest_path.parent()
            && let Err(err) = dst_fs.create_dir_all(parent)
        {
            let mut errors = copy_errors.lock().unwrap();
            errors.insert(src_path.clone(), err.to_string());
            copy_bar.inc(1);
            return None;
        }

        // Use VFS transfer engine
        match transfer_file(opts.src_backend, &src_path, &dst_fs, &dest_path, &transfer_strategies) {
            Ok(_) => {
                copy_bar.inc(1);
                Some((src_path, dest_path, size, taken_ms, crc))
            }
            Err(err) => {
                let err_str = err.to_string();
                
                // Check for MTP disconnection - this is fatal, should abort
                if err_str.contains("disconnected") || 
                   err_str.contains("Camera appears to have disconnected") ||
                   err_str.contains("LIBUSB_ERROR_NO_DEVICE") {
                    disconnected.store(true, std::sync::atomic::Ordering::Relaxed);
                    *disconnect_error.lock().unwrap() = Some(err_str);
                    copy_bar.inc(1);
                    return None;
                }
                
                // Regular error - just log it
                let mut errors = copy_errors.lock().unwrap();
                errors.insert(src_path, err_str);
                copy_bar.inc(1);
                None
            }
        }
    };

    let copied: Vec<(std::path::PathBuf, std::path::PathBuf, i64, i64, u32)> = if parallel {
        prepared_entries.into_par_iter().filter_map(copy_op).collect()
    } else {
        prepared_entries.into_iter().filter_map(copy_op).collect()
    };
    
    // Check if disconnected
    if disconnected.load(std::sync::atomic::Ordering::Relaxed) {
        copy_bar.finish_and_clear();
        if let Some(err_str) = disconnect_error.lock().unwrap().take() {
            eprintln!("\n{}\n", style(&err_str).red().bold());
        }
        eprintln!("{}", style("Import aborted due to device disconnection.").yellow());
        
        // Save pending file with remaining items
        let pending_entries: Vec<_> = scan_entries.iter()
            .filter(|se| se.status == FileStatus::LikelyNew || (opts.force && se.status == FileStatus::LikelyCacheDuplicate))
            .skip(copied.len())
            .map(|e| {
                let rel = e.src_path.strip_prefix(opts.src_path)
                    .unwrap_or(&e.src_path)
                    .display().to_string();
                (rel, e.size, e.mtime_ms, e.crc32c, FileStatus::LikelyNew)
            })
            .collect();
        
        if !pending_entries.is_empty() {
            let pending_path = opts.vault_root
                .join(".svault")
                .join(format!("import-{session_id}-interrupted.pending"));
            if write_pending_vfs(&pending_path, &opts.source_name, &session_id, &pending_entries).is_ok() {
                eprintln!("{}", style(format!("Unimported files saved to: {}", pending_path.display())));
            }
        }
        
        return Err(anyhow::anyhow!("MTP device disconnected during import"));
    }

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

    let verified: Vec<(std::path::PathBuf, std::path::PathBuf, i64, i64, u32, Vec<u8>)> = copied
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
            *size,
            *taken_ms,
            Some(*crc),
            None, // raw_unique_id - not extracted for VFS imports yet
            xxh3_bytes,
            sha256_bytes,
            "imported",
            imported_at,
        );
    }

    let imported = verified.len();

    // Write JSON manifest
    use crate::verify::manifest::{ImportManifest, ImportRecord, ManifestManager};
    let manifest_records: Vec<ImportRecord> = verified
        .iter()
        .map(|(src, dest, size, taken_ms, crc, hash)| {
            let (xxh3, sha256) = match opts.hash {
                HashAlgorithm::Xxh3_128 => {
                    let low = u64::from_le_bytes(hash[..8].try_into().unwrap());
                    let high = u64::from_le_bytes(hash[8..16].try_into().unwrap());
                    let hex = format!("{:016x}{:016x}", high, low);
                    (Some(hex), None)
                }
                HashAlgorithm::Sha256 => {
                    let hex = hash.iter().map(|b| format!("{:02x}", b)).collect::<String>();
                    (None, Some(hex))
                }
            };
            let dest_rel = dest.strip_prefix(opts.vault_root).unwrap_or(dest).to_path_buf();
            ImportRecord {
                src_path: src.clone(),
                dest_path: dest_rel,
                size: *size as u64,
                mtime_ms: *taken_ms,
                crc32c: *crc,
                xxh3_128: xxh3,
                sha256,
                imported_at,
            }
        })
        .collect();

    let manifest = ImportManifest {
        session_id: session_id.clone(),
        source_root: std::path::PathBuf::from(&opts.source_name),
        imported_at,
        hash_algorithm: opts.hash.to_string(),
        files: manifest_records,
    };
    let manifest_manager = ManifestManager::new(opts.vault_root);
    let manifest_path = manifest_manager.save(&manifest)?;

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
        style("files imported")
    );
    eprintln!(
        "  {} {}",
        style(likely_dup).yellow(),
        style("duplicates skipped")
    );
    eprintln!(
        "  {} {}",
        style(failed_b + copy_errors.lock().unwrap().len()).red(),
        style("failed")
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

/// Resolve unique destination path by checking for conflicts and applying rename template.
/// If the destination exists, generates a new name like "IMG_001.1.jpg" using the rename_template.
fn resolve_unique_dest_path(
    dst_fs: &dyn VfsBackend,
    dest_path: &Path,
    rename_template: &str,
) -> std::path::PathBuf {
    // If destination doesn't exist, use it as-is
    match dst_fs.exists(dest_path) {
        Ok(false) => return dest_path.to_path_buf(),
        Ok(true) => {}
        Err(_) => return dest_path.to_path_buf(), // On error, try original path
    }

    // Destination exists - generate unique name
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
        
        match dst_fs.exists(&new_dest) {
            Ok(false) => return new_dest,
            Ok(true) => continue, // Try next number
            Err(_) => return new_dest, // On error, try this path anyway
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

/// Resolve unique destination path, checking both filesystem and in-memory assigned destinations.
/// This is used during the serial preparation phase to avoid conflicts between files
/// that are being imported in the same batch.
fn resolve_unique_dest_path_with_assigned(
    dst_fs: &dyn VfsBackend,
    dest_path: &Path,
    rename_template: &str,
    assigned: &std::collections::HashSet<std::path::PathBuf>,
) -> std::path::PathBuf {
    // Check filesystem first
    match dst_fs.exists(dest_path) {
        Ok(true) => {
            // Destination exists on filesystem - need to find unique name
            return resolve_unique_dest_path(dst_fs, dest_path, rename_template);
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
        let raw = if make.is_empty() || model.starts_with(&make) {
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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::system::SystemFs;
    use std::io::Write;

    /// Create a test file with the given content
    fn create_test_file(path: &Path, content: &[u8]) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::File::create(path)?;
        f.write_all(content)?;
        Ok(())
    }

    /// Create a mock JPEG file with EXIF-like content
    fn create_mock_jpeg(path: &Path) -> std::io::Result<()> {
        // Minimal JPEG-like header
        let mut content = vec![0xFF, 0xD8, 0xFF, 0xE1]; // JPEG SOI + APP1 marker
        // Add some padding to make it look like a file
        content.extend_from_slice(&[0; 100]);
        create_test_file(path, &content)
    }

    #[test]
    fn test_resolve_unique_dest_path_no_conflict() {
        // Create a temporary directory
        let temp_dir = tempfile::tempdir().unwrap();
        let fs = SystemFs::open(temp_dir.path()).unwrap();
        
        let dest = temp_dir.path().join("test.jpg");
        let result = resolve_unique_dest_path(&fs, &dest, "$filename.$n.$ext");
        
        // Should return original path since file doesn't exist
        assert_eq!(result, dest);
    }

    #[test]
    fn test_resolve_unique_dest_path_with_conflict() {
        // Create a temporary directory
        let temp_dir = tempfile::tempdir().unwrap();
        let fs = SystemFs::open(temp_dir.path()).unwrap();
        
        // Create existing file
        let dest = temp_dir.path().join("IMG_001.jpg");
        create_test_file(&dest, b"existing").unwrap();
        
        // Should generate a new name
        let result = resolve_unique_dest_path(&fs, &dest, "$filename.$n.$ext");
        
        assert_ne!(result, dest);
        assert!(result.to_string_lossy().contains("IMG_001.1.jpg"));
    }

    #[test]
    fn test_resolve_unique_dest_path_multiple_conflicts() {
        // Create a temporary directory
        let temp_dir = tempfile::tempdir().unwrap();
        let fs = SystemFs::open(temp_dir.path()).unwrap();
        
        // Create multiple existing files with same base name
        create_test_file(&temp_dir.path().join("IMG_001.jpg"), b"v1").unwrap();
        create_test_file(&temp_dir.path().join("IMG_001.1.jpg"), b"v2").unwrap();
        create_test_file(&temp_dir.path().join("IMG_001.2.jpg"), b"v3").unwrap();
        
        let dest = temp_dir.path().join("IMG_001.jpg");
        let result = resolve_unique_dest_path(&fs, &dest, "$filename.$n.$ext");
        
        // Should find the next available number (3)
        assert!(result.to_string_lossy().contains("IMG_001.3.jpg"));
    }

    /// Test scenario: Multiple cameras with same model importing simultaneously
    /// 
    /// This tests the critical scenario where two photographers with the same
    /// camera model (e.g., two "RICOH GR IV" cameras) import at the same time.
    /// The files will have the same device name in the path template, causing
    /// potential conflicts.
    #[test]
    fn test_multi_camera_same_model_conflict() {
        let temp_dir = tempfile::tempdir().unwrap();
        let vault_dir = tempfile::tempdir().unwrap();
        let _fs = SystemFs::open(temp_dir.path()).unwrap();
        
        // Simulate Camera A files
        let camera_a_dir = temp_dir.path().join("camera_a");
        create_mock_jpeg(&camera_a_dir.join("IMG_001.jpg")).unwrap();
        create_mock_jpeg(&camera_a_dir.join("IMG_002.jpg")).unwrap();
        
        // Simulate Camera B files (same model, same filenames, same date)
        // This happens when two photographers use the same camera model
        // and shoot on the same day, resulting in identical filenames
        let camera_b_dir = temp_dir.path().join("camera_b");
        create_mock_jpeg(&camera_b_dir.join("IMG_001.jpg")).unwrap(); // Same name!
        create_mock_jpeg(&camera_b_dir.join("IMG_002.jpg")).unwrap(); // Same name!
        
        // Import Camera A first
        let entries_a = vec![
            DirEntry {
                path: camera_a_dir.join("IMG_001.jpg"),
                size: 100,
                mtime_ms: 1714552800000, // 2024-05-01 10:00:00
                is_dir: false,
            },
            DirEntry {
                path: camera_a_dir.join("IMG_002.jpg"),
                size: 100,
                mtime_ms: 1714552800000,
                is_dir: false,
            },
        ];
        
        // Copy files to vault (simulating Camera A import)
        let vault_fs = SystemFs::open(vault_dir.path()).unwrap();
        let device_name = "RICOH_GR_IV"; // Same device for both cameras
        
        for entry in &entries_a {
            let filename = entry.path.file_name().unwrap();
            let dest_rel = resolve_dest_path(
                "$year/$mon-$day/$device/$filename",
                Path::new(filename),
                entry.mtime_ms,
                device_name,
            );
            let dest_path = vault_dir.path().join(&dest_rel);
            let unique_dest = resolve_unique_dest_path(&vault_fs, &dest_path, "$filename.$n.$ext");
            
            // Create parent directories and copy
            if let Some(parent) = unique_dest.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::copy(&entry.path, &unique_dest).unwrap();
        }
        
        // Verify Camera A files are in place
        let expected_a1 = vault_dir.path().join("2024/05-01/RICOH_GR_IV/IMG_001.jpg");
        let expected_a2 = vault_dir.path().join("2024/05-01/RICOH_GR_IV/IMG_002.jpg");
        assert!(expected_a1.exists());
        assert!(expected_a2.exists());
        
        // Now import Camera B (same model, same filenames)
        let entries_b = vec![
            DirEntry {
                path: camera_b_dir.join("IMG_001.jpg"),
                size: 150, // Different size (different content)
                mtime_ms: 1714552800000, // Same timestamp
                is_dir: false,
            },
            DirEntry {
                path: camera_b_dir.join("IMG_002.jpg"),
                size: 150,
                mtime_ms: 1714552800000,
                is_dir: false,
            },
        ];
        
        let mut renamed_count = 0;
        for entry in &entries_b {
            let filename = entry.path.file_name().unwrap();
            let dest_rel = resolve_dest_path(
                "$year/$mon-$day/$device/$filename",
                Path::new(filename),
                entry.mtime_ms,
                device_name,
            );
            let dest_path = vault_dir.path().join(&dest_rel);
            let unique_dest = resolve_unique_dest_path(&vault_fs, &dest_path, "$filename.$n.$ext");
            
            // Should have been renamed to avoid conflict
            if unique_dest != dest_path {
                renamed_count += 1;
            }
            
            // Create parent directories and copy
            if let Some(parent) = unique_dest.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::copy(&entry.path, &unique_dest).unwrap();
        }
        
        // Both files should have been renamed (conflict with Camera A)
        assert_eq!(renamed_count, 2, "Both Camera B files should be renamed");
        
        // Verify renamed files exist
        let expected_b1_renamed = vault_dir.path().join("2024/05-01/RICOH_GR_IV/IMG_001.1.jpg");
        let expected_b2_renamed = vault_dir.path().join("2024/05-01/RICOH_GR_IV/IMG_002.1.jpg");
        assert!(expected_b1_renamed.exists(), "Camera B IMG_001 should be renamed");
        assert!(expected_b2_renamed.exists(), "Camera B IMG_002 should be renamed");
        
        // Verify we have 4 files total
        let vault_files: Vec<_> = walkdir::WalkDir::new(vault_dir.path())
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .collect();
        assert_eq!(vault_files.len(), 4, "Should have 4 total files (2 from each camera)");
    }

    /// Test scenario: Same camera, same date, burst mode creates sequential files
    /// 
    /// This tests that burst shots like IMG_001.jpg, IMG_002.jpg don't conflict
    /// with existing files from an earlier import of the same camera.
    #[test]
    fn test_same_camera_burst_mode_import() {
        let _temp_dir = tempfile::tempdir().unwrap();
        let vault_dir = tempfile::tempdir().unwrap();
        let vault_fs = SystemFs::open(vault_dir.path()).unwrap();
        
        // First import: Burst shots 001-003
        let first_import = vec!["IMG_001.jpg", "IMG_002.jpg", "IMG_003.jpg"];
        for name in &first_import {
            let dest = vault_dir.path().join("2024/05-01/Camera/").join(name);
            create_test_file(&dest, b"first").unwrap();
        }
        
        // Second import from same camera: More burst shots, overlapping sequence
        let second_import = vec!["IMG_002.jpg", "IMG_003.jpg", "IMG_004.jpg"];
        let mut rename_results = vec![];
        
        for name in &second_import {
            let dest = vault_dir.path().join("2024/05-01/Camera/").join(name);
            let unique = resolve_unique_dest_path(&vault_fs, &dest, "$filename.$n.$ext");
            create_test_file(&unique, b"second").unwrap();
            
            rename_results.push((name.to_string(), unique.file_name().unwrap().to_string_lossy().to_string()));
        }
        
        // IMG_002 and IMG_003 should be renamed (conflict)
        assert!(rename_results[0].1.contains("IMG_002.1.jpg") || rename_results[0].1 == "IMG_002.jpg",
            "IMG_002 from second import should be renamed or original if not exists: got {}", rename_results[0].1);
        // Actually IMG_002 exists from first import, so should be renamed
        assert!(rename_results[0].1.contains(".1."), "IMG_002 should be renamed to .1.: got {}", rename_results[0].1);
        assert!(rename_results[1].1.contains(".1."), "IMG_003 should be renamed: got {}", rename_results[1].1);
        // IMG_004 is new, should not be renamed
        assert_eq!(rename_results[2].1, "IMG_004.jpg", "IMG_004 should not be renamed");
    }

    /// Test edge case: Filename with multiple dots
    #[test]
    fn test_filename_with_multiple_dots() {
        let temp_dir = tempfile::tempdir().unwrap();
        let fs = SystemFs::open(temp_dir.path()).unwrap();
        
        // Create file with multiple dots in name
        let dest = temp_dir.path().join("IMG_001.COPY.jpg");
        create_test_file(&dest, b"existing").unwrap();
        
        let new_dest = temp_dir.path().join("IMG_001.COPY.jpg");
        let result = resolve_unique_dest_path(&fs, &new_dest, "$filename.$n.$ext");
        
        // Should handle the extension correctly (last dot)
        let result_str = result.to_string_lossy();
        assert!(result_str.contains("IMG_001.COPY.1.jpg"), "Should insert counter before extension: got {}", result_str);
    }

    /// Test edge case: File without extension
    #[test]
    fn test_filename_without_extension() {
        let temp_dir = tempfile::tempdir().unwrap();
        let fs = SystemFs::open(temp_dir.path()).unwrap();
        
        let dest = temp_dir.path().join("README");
        create_test_file(&dest, b"existing").unwrap();
        
        let new_dest = temp_dir.path().join("README");
        let result = resolve_unique_dest_path(&fs, &new_dest, "$filename.$n$ext");
        
        let result_str = result.to_string_lossy();
        assert!(result_str.contains("README.1"), "Should add counter to file without extension: got {}", result_str);
    }
}
