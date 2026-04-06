//! `svault add` — register files already inside the vault.
//!
//! Uses the shared pipeline stages:
//! - Stage A: Scan (pipeline::scan)
//! - Stage B: CRC32C (pipeline::crc)
//! - Lookup: DB duplicate check (inline, real-time)
//! - Stage D: Hash (pipeline::hash)
//! - Stage E: Insert (pipeline::insert)

use console::style;
use indicatif::{ProgressBar, ProgressStyle};

use crate::config::{Config, HashAlgorithm};
use crate::db::Db;
use crate::pipeline;

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

/// Check if a file is a duplicate via DB lookup.
fn check_duplicate(entry: &pipeline::types::CrcEntry, db: &Db, vault_root: &std::path::Path) -> pipeline::types::FileStatus {
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

/// Run `add` on a directory inside the vault.
pub fn run_add(opts: AddOptions, db: &Db) -> anyhow::Result<AddSummary> {
    let config = Config::load(&opts.vault_root)?;
    let exts: Vec<&str> = config
        .import
        .allowed_extensions
        .iter()
        .map(|s| s.as_str())
        .collect();

    // ------------------------------------------------------------------
    // Stage A+B+C: Scan + CRC + Lookup (parallel pipeline with real-time output)
    // ------------------------------------------------------------------
    let scan_rx = pipeline::scan::scan_stream(&opts.path, &exts)?;
    
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
        let rel_path = entry.file.path.strip_prefix(&opts.vault_root)
            .unwrap_or(&entry.file.path);
        let status = check_duplicate(&entry, db, &opts.vault_root);
        
        match status {
            pipeline::types::FileStatus::LikelyCacheDuplicate => {
                eprintln!("  {} {}",
                    style("Duplicate").yellow(),
                    style(rel_path.display()));
            }
            pipeline::types::FileStatus::LikelyNew => {
                eprintln!("  {} {}",
                    style("Found").green(),
                    style(rel_path.display()));
            }
            _ => {}
        }
        
        lookup_results.push(pipeline::types::LookupResult { entry, status });
    }
    scan_bar.finish_and_clear();

    if lookup_results.is_empty() {
        eprintln!("\nNo files found to add.");
        return Ok(AddSummary::default());
    }

    let (new_files, dup_files) = pipeline::lookup::filter_new(lookup_results, false);
    let likely_dup = dup_files.len();
    let failed_b = total_files.saturating_sub(new_files.len() + dup_files.len());

    // Pre-flight
    eprintln!();
    eprintln!("{}", style("Pre-flight:").bold());
    eprintln!("  {}  {}",
        style(format!("Likely new:       {:>6}", new_files.len())).green(),
        style("will be added"));
    if likely_dup > 0 {
        eprintln!("  {}  {}",
            style(format!("Likely duplicate: {:>6}", likely_dup)).yellow(),
            style("already in vault"));
    }

    if new_files.is_empty() {
        eprintln!("\nAll files already tracked.");
        return Ok(AddSummary {
            total: total_files,
            skipped: likely_dup,
            ..Default::default()
        });
    }

    // ------------------------------------------------------------------
    // Stage D: Hash
    // ------------------------------------------------------------------
    let hash_bar = ProgressBar::new(new_files.len() as u64);
    hash_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.yellow} [{bar:40}] {pos}/{len}")
            .unwrap()
            .progress_chars("=> "),
    );
    hash_bar.set_prefix("Hashing  ");

    let hash_results = pipeline::hash::compute_hashes(new_files, opts.hash, Some(&hash_bar));
    hash_bar.finish_and_clear();

    // Check duplicates (allow same path re-add)
    let hash_results = pipeline::hash::check_duplicates(
        hash_results, db, &opts.vault_root, &opts.hash, true)?;

    // Detect vault-internal moves (same content, different path)
    let mut moved_files: Vec<(std::path::PathBuf, String)> = Vec::new();
    for r in &hash_results {
        if r.is_duplicate && r.dup_reason.as_deref() == Some("db") {
            if let Ok(Some(existing)) = db.lookup_by_hash(&r.hash_bytes, &opts.hash) {
                let existing_path = opts.vault_root.join(&existing.path);
                if existing_path != r.path {
                    moved_files.push((r.path.clone(), existing.path.clone()));
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Stage E: Insert
    // ------------------------------------------------------------------
    let session_id = crate::import::utils::session_id_now();
    let insert_opts = pipeline::insert::InsertOptions {
        vault_root: &opts.vault_root,
        hash_algo: &opts.hash,
        session_id: &session_id,
        write_manifest: false,
        source_root: None,
        force: false,
    };

    let summary = pipeline::insert::batch_insert(hash_results, db, insert_opts, None)?;

    // Print summary
    eprintln!(
        "{} {} file(s) added",
        style("Finished:").bold().green(),
        style(summary.added).green()
    );
    if summary.duplicate > 0 {
        eprintln!("         {} duplicate(s) skipped",
            style(summary.duplicate).yellow());
    }
    if summary.failed > 0 {
        eprintln!("         {} file(s) failed",
            style(summary.failed).red());
    }

    // Suggest reconcile for vault-internal moves
    if !moved_files.is_empty() {
        eprintln!();
        eprintln!("{}", style("Note:").bold().cyan());
        eprintln!("  {} file(s) appear to have been moved within the vault.",
            style(moved_files.len()).cyan());
        eprintln!("  Use {} to update their paths:", style("svault reconcile").bold());
        
        for (_, (current, original)) in moved_files.iter().take(3).enumerate() {
            let current_rel = current.strip_prefix(&opts.vault_root).unwrap_or(current);
            eprintln!("    {} → {}", 
                style(original).dim(),
                style(current_rel.display()).cyan());
        }
        if moved_files.len() > 3 {
            eprintln!("    ... and {} more", moved_files.len() - 3);
        }
    }

    Ok(AddSummary {
        total: total_files,
        added: summary.added,
        duplicate: summary.duplicate + likely_dup,
        skipped: summary.skipped,
        failed: summary.failed + failed_b,
    })
}
