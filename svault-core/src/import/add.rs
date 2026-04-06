//! `svault add` — register files already inside the vault.
//!
//! Scans a directory within the vault, computes hashes for any files not
//! already tracked in the database, and inserts them as `file.imported` events.
//!
//! Uses the shared pipeline stages:
//! - Stage A: Directory scan (pipeline::scan)
//! - Stage B: CRC32C fingerprint (pipeline::crc)
//! - Lookup: DB duplicate check (pipeline::lookup)
//! - Stage D: Strong hash verification (pipeline::hash)
//! - Stage E: DB batch insert (pipeline::insert)
//!
//! Note: No Stage C (copy) since files are already in vault.

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
    // Stage A: Scan
    // ------------------------------------------------------------------
    let entries = pipeline::scan::scan_files_simple(&opts.path, &exts)?;
    
    // Get canonical path for opts.path to ensure absolute paths
    let opts_path = std::fs::canonicalize(&opts.path)?;
    
    // Convert paths from relative to opts.path to relative to vault_root
    let entries: Vec<pipeline::types::FileEntry> = entries
        .into_iter()
        .map(|e| {
            let abs_path = opts_path.join(&e.path);
            let vault_rel = abs_path.strip_prefix(&opts.vault_root)
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|_| e.path);
            pipeline::types::FileEntry {
                path: vault_rel,
                size: e.size,
                mtime_ms: e.mtime_ms,
            }
        })
        .collect();
    
    let total = entries.len();

    if total == 0 {
        eprintln!(
            "{} No files found in {}",
            style("Warning:").yellow().bold(),
            opts.path.display()
        );
        return Ok(AddSummary::default());
    }

    eprintln!(
        "{} Scanning {} files in {}",
        style("Adding:").bold().cyan(),
        style(total).cyan(),
        style(opts.path.display())
    );

    // ------------------------------------------------------------------
    // Stage B: CRC32C fingerprint
    // ------------------------------------------------------------------
    let scan_bar = ProgressBar::new(total as u64);
    scan_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.cyan} [{bar:40}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("=> "),
    );
    scan_bar.set_prefix("Scanning");

    let crc_results = pipeline::crc::compute_crcs(entries, &opts.vault_root, Some(&scan_bar));
    scan_bar.finish_and_clear();

    let (crc_entries, crc_errors) = pipeline::crc::filter_successful(crc_results);

    // Report CRC errors
    for (path, err) in &crc_errors {
        eprintln!("  {} {} - {}", style("Error").red(), style(path.display()), err);
    }

    // ------------------------------------------------------------------
    // DB lookup for duplicates
    // ------------------------------------------------------------------
    let lookup_results = pipeline::lookup::lookup_duplicates(crc_entries, db, &opts.vault_root)?;

    // Report scan results
    for r in &lookup_results {
        // The path is already relative to vault_root
        let rel_path = &r.entry.file.path;
        match r.status {
            pipeline::types::FileStatus::LikelyNew => {
                eprintln!(
                    "  {} {}",
                    style("Found").green(),
                    style(rel_path.display())
                );
            }
            pipeline::types::FileStatus::LikelyCacheDuplicate => {
                eprintln!(
                    "  {} {}",
                    style("Duplicate").yellow(),
                    style(rel_path.display())
                );
            }
            _ => {}
        }
    }

    // Filter to new files only (no force mode for add)
    let (new_files, dup_files) = pipeline::lookup::filter_new(lookup_results, false);
    let likely_dup = dup_files.len();
    let failed_b = crc_errors.len();

    // Pre-flight summary
    eprintln!();
    eprintln!("{}", style("Pre-flight:").bold());
    eprintln!(
        "  {}  {}",
        style(format!("Likely new:       {:>6}", new_files.len())).green(),
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

    // Early exit if all cache hits
    if new_files.is_empty() {
        eprintln!();
        eprintln!("All {} files already tracked (no new files).", total);
        return Ok(AddSummary {
            total,
            skipped: likely_dup,
            ..Default::default()
        });
    }

    // ------------------------------------------------------------------
    // Stage D: Strong hash verification
    // ------------------------------------------------------------------
    let hash_bar = ProgressBar::new(new_files.len() as u64);
    hash_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.yellow} [{bar:40}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("=> "),
    );
    hash_bar.set_prefix("Hashing  ");

    let hash_results = pipeline::hash::compute_hashes(new_files, opts.hash, Some(&hash_bar));
    hash_bar.finish_and_clear();

    // Check duplicates (allow same path re-add)
    let hash_results = pipeline::hash::check_duplicates(
        hash_results,
        db,
        &opts.vault_root,
        &opts.hash,
        true, // allow_same_path
    )?;

    // ------------------------------------------------------------------
    // Stage E: DB batch insert (no manifest for add)
    // ------------------------------------------------------------------
    let session_id = crate::import::utils::session_id_now();
    let insert_opts = pipeline::insert::InsertOptions {
        vault_root: &opts.vault_root,
        hash_algo: &opts.hash,
        session_id: &session_id,
        write_manifest: false, // add doesn't write manifest
        source_root: None,
    };

    let summary = pipeline::insert::batch_insert(hash_results, db, insert_opts)?;
    
    eprintln!("DEBUG: summary.total={}, summary.added={}, summary.duplicate={}, summary.skipped={}, summary.failed={}",
        summary.total, summary.added, summary.duplicate, summary.skipped, summary.failed);

    // Print summary
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
    if summary.failed > 0 {
        eprintln!(
            "         {} file(s) failed",
            style(summary.failed).red()
        );
    }
    if summary.skipped > 0 {
        eprintln!(
            "         {} already tracked by path",
            style(summary.skipped).cyan()
        );
    }

    Ok(AddSummary {
        total,
        added: summary.added,
        duplicate: summary.duplicate + likely_dup,
        skipped: summary.skipped,
        failed: summary.failed + failed_b,
    })
}
