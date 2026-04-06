//! Import pipeline (Stages A–E).
//!
//! Uses the shared pipeline stages from `crate::pipeline`:
//! - Stage A: Scan (pipeline::scan)
//! - Stage B: CRC32C (pipeline::crc)
//! - Lookup: DB duplicate check (pipeline::lookup)
//! - Stage D: Hash (pipeline::hash)
//! - Stage E: Insert (pipeline::insert)

pub mod add;
pub mod types;
pub mod exif;
pub mod path;
pub mod recheck;
pub mod reconcile;
pub mod staging;
pub mod utils;
pub mod vfs_import;

pub use types::{ImportOptions, FileStatus, ScanEntry, ImportSummary};

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::db::Db;
use crate::pipeline;
use crate::vfs::system::SystemFs;
use crate::vfs::transfer::transfer_file;

use exif::read_exif_date_device;
use path::resolve_dest_path;
use utils::session_id_now;

/// Check if a file is a duplicate via DB lookup.
/// Returns FileStatus::LikelyCacheDuplicate if found in vault, otherwise LikelyNew.
fn check_duplicate(entry: &pipeline::types::CrcEntry, db: &Db, vault_root: &Path) -> pipeline::types::FileStatus {
    let ext = entry.file.path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    
    let cached = match db.lookup_by_crc32c(
        entry.file.size as i64,
        entry.crc32c,
        ext,
        entry.raw_unique_id.as_deref(),
    ) {
        Ok(c) => c,
        Err(_) => return pipeline::types::FileStatus::LikelyNew,
    };
    
    if let Some(row) = cached {
        let is_same_raw_id = match (&entry.raw_unique_id, &row.raw_unique_id) {
            (Some(new_id), Some(existing_id)) => new_id == existing_id,
            _ => true,
        };
        let vault_path = vault_root.join(&row.path);
        if vault_path.exists() && is_same_raw_id {
            return pipeline::types::FileStatus::LikelyCacheDuplicate;
        }
    }
    
    pipeline::types::FileStatus::LikelyNew
}

/// Run the full import pipeline (Stages A–E).
pub fn run(opts: ImportOptions, db: &Db) -> anyhow::Result<ImportSummary> {
    let session_id = session_id_now();
    let source_canon = std::fs::canonicalize(&opts.source)
        .unwrap_or_else(|_| opts.source.clone());
    let vault_canon = std::fs::canonicalize(&opts.vault_root)
        .unwrap_or_else(|_| opts.vault_root.clone());

    // ------------------------------------------------------------------
    // Stage A+B+C: Scan + CRC + Lookup (parallel pipeline with real-time output)
    // ------------------------------------------------------------------
    let exts: Vec<&str> = opts.import_config.allowed_extensions
        .iter().map(|s| s.as_str()).collect();

    // Stage A: Walk (parallel) → Stage B: CRC (parallel via channel)
    let scan_rx = pipeline::scan::scan_stream(&source_canon, &exts)?;
    
    // Progress bar for scanning phase
    let scan_bar = ProgressBar::new_spinner();
    scan_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.cyan} {spinner} {pos} files ({per_sec})")
            .unwrap(),
    );
    scan_bar.set_prefix("Scanning");

    let crc_rx = pipeline::crc::compute_crcs_stream(scan_rx, Some(scan_bar.clone()));

    // Stage C: Lookup (serial from channel) with real-time output
    let mut lookup_results = Vec::new();
    let mut total_files = 0usize;
    
    for result in crc_rx {
        total_files += 1;
        scan_bar.inc(1);
        
        // Skip vault paths
        if result.file.path.ancestors().any(|p| p == vault_canon) {
            continue;
        }
        
        // Handle CRC errors
        let crc = match result.crc {
            Ok(c) => c,
            Err(e) => {
                scan_bar.println(format!("  {} {} - {}", 
                    style("Error").red(), 
                    style(&result.file.path.display()),
                    e));
                continue;
            }
        };
        
        // Build CrcEntry
        let ext = result.file.path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let raw_unique_id = if crate::media::raw_id::is_raw_file(ext) {
            crate::media::raw_id::extract_raw_id_if_raw(&result.file.path)
                .and_then(|raw_id| crate::media::raw_id::get_fingerprint_string(&raw_id))
        } else {
            None
        };
        
        let entry = pipeline::types::CrcEntry {
            file: pipeline::types::FileEntry {
                path: result.file.path.clone(),
                size: result.file.size,
                mtime_ms: result.file.mtime_ms,
            },
            src_path: None,
            crc32c: crc,
            raw_unique_id,
        };
        
        // Immediate DB lookup and real-time output
        let rel_path = entry.file.path.strip_prefix(&source_canon)
            .unwrap_or(&entry.file.path);
        let status = check_duplicate(&entry, db, &opts.vault_root);
        
        match status {
            pipeline::types::FileStatus::LikelyCacheDuplicate if opts.show_dup => {
                scan_bar.println(format!("  {} {}",
                    style("Duplicate").yellow(),
                    style(rel_path.display())));
            }
            pipeline::types::FileStatus::LikelyNew => {
                scan_bar.println(format!("  {} {}",
                    style("Found").green(),
                    style(rel_path.display())));
            }
            _ => {}
        }
        
        lookup_results.push(pipeline::types::LookupResult { entry, status });
    }
    scan_bar.finish_and_clear();

    if lookup_results.is_empty() {
        eprintln!("\nNo files found to import.");
        return Ok(ImportSummary::default());
    }

    let (new_files, dup_files) = pipeline::lookup::filter_new(lookup_results, opts.force);
    let likely_dup = dup_files.len();

    // Pre-flight summary
    eprintln!();
    eprintln!("{}", style("Pre-flight:").bold());
    eprintln!("  {}  {}",
        style(format!("Likely new:       {:>6}", new_files.len())).green(),
        style("will be imported"));
    if likely_dup > 0 {
        eprintln!("  {}  {}",
            style(format!("Likely duplicate: {:>6}", likely_dup)).yellow(),
            style("already in vault (cache hit)"));
    }


    // Show duplicate files if --show-dup is enabled (for non-TTY environments)
    if opts.show_dup && !dup_files.is_empty() {
        eprintln!();
        for dup in &dup_files {
            let rel_path = dup.file.path.strip_prefix(&source_canon)
                .unwrap_or(&dup.file.path);
            eprintln!("  {} {}",
                style("Duplicate").yellow(),
                style(rel_path.display()));
        }
    }

    // Early exit if no new files
    if new_files.is_empty() {
        eprintln!();
        eprintln!("All {} files matched cache (no new files detected).", total_files);
        return Ok(ImportSummary {
            total: total_files,
            duplicate: likely_dup,
            failed: 0,
            all_cache_hit: true,
            ..Default::default()
        });
    }

    // ------------------------------------------------------------------
    // Interactive confirmation
    // ------------------------------------------------------------------
    let staging_dir = opts.vault_root.join(".svault").join("staging");
    fs::create_dir_all(&staging_dir)?;
    // ... staging logic

    if !opts.yes && !opts.dry_run {
        eprint!("{}", style("Proceed with import? [y/N] ").bold());
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
            eprintln!("{}", style("Aborted.").yellow());
            return Ok(ImportSummary { total: total_files, duplicate: likely_dup, ..Default::default() });
        }
    }

    if opts.dry_run {
        eprintln!("\n(dry-run: no files copied)");
        return Ok(ImportSummary { total: total_files, duplicate: likely_dup, ..Default::default() });
    }

    // ------------------------------------------------------------------
    // Stage C: Copy files (parallel)
    // ------------------------------------------------------------------
    let vault_archive = opts.vault_root.clone();
    let dst_fs = SystemFs::open(&vault_archive)?;

    // Pre-resolve destination paths
    let mut prepared = Vec::new();
    let mut assigned = std::collections::HashSet::new();

    for entry in &new_files {
        let rel = entry.file.path.strip_prefix(&source_canon)
            .unwrap_or(&entry.file.path);
        let (taken_ms, device) = read_exif_date_device(&entry.file.path, entry.file.mtime_ms);
        let dest_rel = resolve_dest_path(
            &opts.import_config.path_template,
            rel,
            taken_ms,
            &device,
        );
        let dest_abs = vault_archive.join(&dest_rel);
        
        // Handle conflicts
        let unique_dest = resolve_unique_dest(&dest_abs, &opts.import_config.rename_template, &assigned);
        assigned.insert(unique_dest.clone());
        
        prepared.push((
            entry.file.path.clone(),
            unique_dest,
            entry.file.size,
            entry.file.mtime_ms,
            entry.crc32c,
            entry.raw_unique_id.clone(),
        ));
    }

    let copy_errors: Arc<Mutex<HashMap<std::path::PathBuf, String>>> = 
        Arc::new(Mutex::new(HashMap::new()));
    let copy_bar = ProgressBar::new(prepared.len() as u64);
    copy_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.green} [{bar:40}] {pos}/{len}")
            .unwrap()
            .progress_chars("=> "),
    );
    copy_bar.set_prefix("Copying  ");

    let src_fs = SystemFs::open(&source_canon)?;
    let transfer_strategies = opts.strategy.to_transfer_strategies();

    let copied: Vec<_> = prepared
        .into_par_iter()
        .filter_map(|(src, dest, size, mtime, crc, raw_id)| {
            if let Some(parent) = dest.parent() {
                if fs::create_dir_all(parent).is_err() {
                    copy_errors.lock().unwrap().insert(src.clone(), "mkdir failed".to_string());
                    copy_bar.inc(1);
                    return None;
                }
            }

            let rel = src.strip_prefix(&source_canon).unwrap_or(&src);
            match transfer_file(&src_fs, rel, &dst_fs, &dest, &transfer_strategies) {
                Ok(_) => {
                    let vault_rel = dest.strip_prefix(&opts.vault_root).unwrap_or(&dest);
                    copy_bar.println(format!("  {} {}",
                        style("Added").green(),
                        style(vault_rel.display())));
                    copy_bar.inc(1);
                    Some((src, dest, size, mtime, crc, raw_id))
                }
                Err(e) => {
                    copy_errors.lock().unwrap().insert(src, e.to_string());
                    copy_bar.inc(1);
                    None
                }
            }
        })
        .collect();
    copy_bar.finish_and_clear();

    // ------------------------------------------------------------------
    // Stage D: Strong hash (parallel)
    // ------------------------------------------------------------------
    let hash_bar = ProgressBar::new(copied.len() as u64);
    hash_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.yellow} [{bar:40}] {pos}/{len}")
            .unwrap()
            .progress_chars("=> "),
    );
    hash_bar.set_prefix("Hashing  ");

    // Convert to CrcEntry for hash stage
    let crc_entries: Vec<pipeline::types::CrcEntry> = copied
        .into_iter()
        .map(|(src, dest, size, mtime, crc, raw_id)| {
            pipeline::types::CrcEntry {
                file: pipeline::types::FileEntry { path: dest, size, mtime_ms: mtime },
                src_path: Some(src),
                crc32c: crc,
                raw_unique_id: raw_id,
            }
        })
        .collect();

    let hash_results = pipeline::hash::compute_hashes(crc_entries, opts.hash, Some(&hash_bar));
    hash_bar.finish_and_clear();

    // Check duplicates (skip if force mode - will overwrite existing files)
    let hash_results = if opts.force {
        hash_results
    } else {
        pipeline::hash::check_duplicates(
            hash_results, db, &opts.vault_root, &opts.hash, false)?
    };

    // ------------------------------------------------------------------
    // Stage E: DB insert
    // ------------------------------------------------------------------
    let insert_bar = ProgressBar::new(hash_results.len() as u64);
    insert_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.magenta} [{bar:40}] {pos}/{len}")
            .unwrap()
            .progress_chars("=> "),
    );
    insert_bar.set_prefix("Inserting");

    let insert_opts = pipeline::insert::InsertOptions {
        vault_root: &opts.vault_root,
        hash_algo: &opts.hash,
        session_id: &session_id,
        write_manifest: true,
        source_root: Some(&opts.source),
        force: opts.force,
    };

    let summary = pipeline::insert::batch_insert(hash_results, db, insert_opts, Some(&insert_bar))?;
    insert_bar.finish_and_clear();

    // Print summary
    eprintln!("{} {} file(s) imported",
        style("Finished:").bold().green(),
        style(summary.added).green());
    if summary.duplicate > 0 {
        eprintln!("         {} duplicate(s) skipped",
            style(summary.duplicate).yellow());
    }
    if summary.failed > 0 {
        eprintln!("         {} file(s) failed",
            style(summary.failed).red());
    }

    Ok(ImportSummary {
        total: total_files,
        imported: summary.added,
        duplicate: summary.duplicate + likely_dup,
        failed: summary.failed + copy_errors.lock().unwrap().len(),
        manifest_path: None, // Set by insert stage
        all_cache_hit: false,
    })
}

/// Resolve unique destination path.
fn resolve_unique_dest(
    dest: &Path,
    rename_template: &str,
    assigned: &std::collections::HashSet<std::path::PathBuf>,
) -> std::path::PathBuf {
    if !dest.exists() && !assigned.contains(dest) {
        return dest.to_path_buf();
    }

    let parent = dest.parent().unwrap_or(Path::new(""));
    let filename = dest.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let (stem, ext) = if let Some(pos) = filename.rfind('.') {
        (&filename[..pos], &filename[pos..])
    } else {
        (&filename[..], "")
    };

    for n in 1..=9999 {
        let new_name = rename_template
            .replace("$filename", stem)
            .replace("$ext", ext.trim_start_matches('.'))
            .replace("$n", &n.to_string());
        let new_dest = parent.join(&new_name);
        if !new_dest.exists() && !assigned.contains(&new_dest) {
            return new_dest;
        }
    }

    // Fallback
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    parent.join(format!("{}.{}{}", stem, ts, ext))
}
