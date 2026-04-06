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

/// Run the full import pipeline (Stages A–E).
pub fn run(opts: ImportOptions, db: &Db) -> anyhow::Result<ImportSummary> {
    let session_id = session_id_now();
    let source_canon = std::fs::canonicalize(&opts.source)
        .unwrap_or_else(|_| opts.source.clone());
    let vault_canon = std::fs::canonicalize(&opts.vault_root)
        .unwrap_or_else(|_| opts.vault_root.clone());

    // ------------------------------------------------------------------
    // Stage A+B: Stream scan + CRC (parallel)
    // ------------------------------------------------------------------
    let exts: Vec<&str> = opts.import_config.allowed_extensions
        .iter().map(|s| s.as_str()).collect();

    // Stream scan files (parallel traversal)
    let scan_rx = pipeline::scan::scan_stream(&source_canon, &exts)?;

    // Set up progress bar with spinner (unknown total during streaming)
    let scan_bar = ProgressBar::new_spinner();
    scan_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.blue} {spinner} {pos} files ({per_sec})")
            .unwrap(),
    );
    scan_bar.set_prefix("Scanning");

    // Stream CRC computation (batched + parallel)
    let crc_rx = pipeline::crc::compute_crcs_stream(scan_rx, Some(scan_bar.clone()));

    // Collect CRC results and filter out vault paths
    let mut crc_results = Vec::new();
    let mut total = 0usize;
    let mut scanned = 0usize;
    for result in crc_rx {
        scanned += 1;
        scan_bar.set_position(scanned as u64);
        
        // Filter out vault paths and show errors in real-time
        // Check if the file path is within the vault directory by checking ancestors
        let is_in_vault = result.file.path.ancestors().any(|p| p == vault_canon);
        match &result.crc {
            Ok(_) if is_in_vault => {
                continue; // Skip vault paths
            }
            Err(e) => {
                scan_bar.println(format!("  {} {} - {}", 
                    style("Error").red(), 
                    style(&result.file.path.display()),
                    e));
            }
            _ => {}
        }
        
        total += 1;
        crc_results.push(result);
    }
    scan_bar.finish_and_clear();

    let (crc_entries, crc_errors) = pipeline::crc::split_results(crc_results);

    // ------------------------------------------------------------------
    // Lookup: DB duplicate check
    // ------------------------------------------------------------------
    eprintln!("{} {} files in {}", 
        style("Scanning").bold().cyan(),
        style(crc_entries.len() + crc_errors.len()).cyan(),
        style(opts.source.display()));

    let lookup_results = pipeline::lookup::lookup_duplicates(crc_entries, db, &opts.vault_root)?;

    // Report results (only show new files, duplicates counted in summary)
    for r in &lookup_results {
        let rel_path = r.entry.file.path.strip_prefix(&source_canon)
            .unwrap_or(&r.entry.file.path);
        if let pipeline::types::FileStatus::LikelyNew = r.status {
            eprintln!("  {} {}", style("Found").green(), style(rel_path.display()));
        }
    }

    let (new_files, dup_files) = pipeline::lookup::filter_new(lookup_results, opts.force);
    let likely_dup = dup_files.len();
    let failed_b = crc_errors.len();

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
    if failed_b > 0 {
        eprintln!("  {}",
            style(format!("Errors:           {:>6}", failed_b)).red());
    }

    // Early exit if no new files
    if new_files.is_empty() {
        eprintln!();
        eprintln!("All {} files matched cache (no new files detected).", total);
        return Ok(ImportSummary {
            total,
            duplicate: likely_dup,
            failed: failed_b,
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
            return Ok(ImportSummary { total, duplicate: likely_dup, ..Default::default() });
        }
    }

    if opts.dry_run {
        eprintln!("\n(dry-run: no files copied)");
        return Ok(ImportSummary { total, duplicate: likely_dup, ..Default::default() });
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
    let insert_opts = pipeline::insert::InsertOptions {
        vault_root: &opts.vault_root,
        hash_algo: &opts.hash,
        session_id: &session_id,
        write_manifest: true,
        source_root: Some(&opts.source),
        force: opts.force,
    };

    let summary = pipeline::insert::batch_insert(hash_results, db, insert_opts)?;

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
        total,
        imported: summary.added,
        duplicate: summary.duplicate + likely_dup,
        failed: summary.failed + failed_b + copy_errors.lock().unwrap().len(),
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
