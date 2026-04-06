//! `svault add` — register files already inside the vault.
//!
//! Uses the shared pipeline stages:
//! - Stage A: Scan (pipeline::scan)
//! - Stage B: CRC32C (pipeline::crc)
//! - Lookup: DB duplicate check (pipeline::lookup)
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
    // Stage A+B: Stream scan + CRC (parallel)
    // ------------------------------------------------------------------
    let scan_rx = pipeline::scan::scan_stream(&opts.path, &exts)?;

    // Set up progress bar with spinner
    let scan_bar = ProgressBar::new_spinner();
    scan_bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.cyan} {spinner} {pos} files ({per_sec})")
            .unwrap(),
    );
    scan_bar.set_prefix("Scanning");

    // Stream CRC computation
    let crc_rx = pipeline::crc::compute_crcs_stream(scan_rx, Some(scan_bar.clone()));

    // Collect CRC results with real-time error display
    let mut crc_results = Vec::new();
    let mut total = 0usize;
    for result in crc_rx {
        total += 1;
        scan_bar.set_position(total as u64);
        
        // Show errors in real-time
        if let Err(e) = &result.crc {
            scan_bar.println(format!("  {} {} - {}", 
                style("Error").red(), 
                style(&result.file.path.display()),
                e));
        }
        
        crc_results.push(result);
    }
    scan_bar.finish_and_clear();

    if total == 0 {
        eprintln!(
            "{} No files found in {}",
            style("Warning:").yellow().bold(),
            opts.path.display()
        );
        return Ok(AddSummary::default());
    }

    eprintln!(
        "{} Scanned {} files in {}",
        style("Adding:").bold().cyan(),
        style(total).cyan(),
        style(opts.path.display())
    );

    let (crc_entries, crc_errors) = pipeline::crc::split_results(crc_results);

    // Report errors
    for err in &crc_errors {
        eprintln!("  {} {}", 
            style("Error").red(), 
            style(&err.file.path.display()));
    }

    // ------------------------------------------------------------------
    // Lookup: DB duplicate check
    // ------------------------------------------------------------------
    let lookup_results = pipeline::lookup::lookup_duplicates(crc_entries, db, &opts.vault_root)?;

    for r in &lookup_results {
        let rel_path = &r.entry.file.path;
        match r.status {
            pipeline::types::FileStatus::LikelyNew => {
                eprintln!("  {} {}", style("Found").green(), style(rel_path.display()));
            }
            pipeline::types::FileStatus::LikelyCacheDuplicate => {
                eprintln!("  {} {}", style("Duplicate").yellow(), style(rel_path.display()));
            }
            _ => {}
        }
    }

    let (new_files, dup_files) = pipeline::lookup::filter_new(lookup_results, false);
    let likely_dup = dup_files.len();
    let failed_b = crc_errors.len();

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
            total,
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

    let summary = pipeline::insert::batch_insert(hash_results, db, insert_opts)?;

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

    Ok(AddSummary {
        total,
        added: summary.added,
        duplicate: summary.duplicate + likely_dup,
        skipped: summary.skipped,
        failed: summary.failed + failed_b,
    })
}
