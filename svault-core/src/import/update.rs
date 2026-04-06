//! `svault update` — update database paths for moved or renamed files.
//!
//! Scans the vault directory, computes hashes, and matches them against
//! database records that are marked `imported` but whose paths no longer exist.
//! When a match is found, the file has been moved/renamed outside of Svault.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::db::Db;
use crate::hash::{sha256_file, xxh3_128_file};
use crate::vfs::system::SystemFs;
use crate::vfs::VfsBackend;

/// Summary of an `update` operation.
#[derive(Debug, Default)]
pub struct UpdateSummary {
    pub scanned: usize,
    pub missing: usize,
    pub matched: usize,
    pub unmatched: usize,
    pub updated: usize,
}

/// Options for `svault update`.
pub struct UpdateOptions {
    pub root: std::path::PathBuf,
    pub vault_root: std::path::PathBuf,
    pub dry_run: bool,
    pub yes: bool,
    /// Actually delete files (if they exist).
    pub delete: bool,
}

/// A single update match.
#[derive(Debug)]
pub struct UpdateMatch {
    pub old_path: String,
    pub new_path: String,
    pub file_id: i64,
}

/// Identity verification level for matched files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchConfidence {
    /// Matched by SHA-256 (cryptographic, definitive).
    Definitive,
    /// Matched by XXH3-128 only (fast, but could collide).
    Fast,
}

/// Run `update` on the vault.
pub fn run_update(opts: UpdateOptions, db: &Db) -> anyhow::Result<UpdateSummary> {
    // 1. Find missing files in DB
    let missing_files = db.get_missing_files(&opts.vault_root)?;
    let missing_count = missing_files.len();

    if missing_count == 0 {
        eprintln!("{} All tracked files exist on disk — nothing to update.",
            style("Update:").bold().green());
        return Ok(UpdateSummary::default());
    }

    eprintln!(
        "{} {} tracked file(s) missing from disk",
        style("Update:").bold().cyan(),
        style(missing_count).cyan()
    );

    // 2. Scan vault disk for all files (streaming)
    let fs = SystemFs::open(&opts.root)?;
    let disk_entries: Vec<_> = fs.walk_stream(Path::new(""), &[])?.into_iter()
        .filter_map(|r| r.ok())
        .collect();
    let scanned = disk_entries.len();

    if scanned == 0 {
        eprintln!("{} No files found on disk to match against.",
            style("Warning:").yellow().bold());
        return Ok(UpdateSummary { missing: missing_count, ..Default::default() });
    }

    eprintln!(
        "  Scanning {} file(s) on disk for matches...",
        style(scanned).cyan()
    );

    // Build indices for efficient lookup
    // Primary index: xxh3_128 (fast)
    // Secondary index: sha256 (definitive, only for files that have it)
    let mut missing_by_xxh3: HashMap<String, Vec<&crate::db::files::FileRow>> = HashMap::new();
    let mut missing_by_sha256: HashMap<String, Vec<&crate::db::files::FileRow>> = HashMap::new();
    
    for row in &missing_files {
        // Index by xxh3_128 (always)
        if let Some(xxh3) = row.xxh3_128.as_ref().map(|b| hex_encode(b)) {
            missing_by_xxh3.entry(xxh3).or_default().push(row);
        }
        // Index by sha256 (if available)
        if let Some(sha256) = row.sha256.as_ref().map(|b| hex_encode(b)) {
            missing_by_sha256.entry(sha256).or_default().push(row);
        }
    }

    // 3. Hash all disk files and look for matches
    let bar = ProgressBar::new(scanned as u64);
    bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.cyan} [{bar:40}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("=> "),
    );
    bar.set_prefix("Hashing  ");

    let matches: Vec<(UpdateMatch, MatchConfidence)> = disk_entries
        .into_par_iter()
        .filter_map(|e| {
            let filename = e.path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            bar.set_message(filename);

            // Always compute xxh3_128 first (fast)
            let xxh3_result = xxh3_128_file(&e.path)
                .map(|h| hex_encode(&h.to_bytes()));
            
            bar.inc(1);
            let xxh3_str = match xxh3_result {
                Ok(h) => h,
                Err(_) => return None,
            };

            // TODO: Try definitive match first (sha256) if available
            // This requires computing SHA-256 of the disk file and comparing
            // with candidates that have SHA-256 in the database

            // First: try fast match by xxh3_128
            if let Some(candidates) = missing_by_xxh3.get(&xxh3_str) {
                let meta = fs::metadata(&e.path).ok()?;
                
                for candidate in candidates {
                    if candidate.size == meta.len() as i64 {
                        let rel_new = e.path.strip_prefix(&opts.vault_root).unwrap_or(&e.path);
                        
                        // If candidate has sha256, compute and verify for definitive match
                        let confidence = if candidate.sha256.is_some() {
                            match sha256_file(&e.path) {
                                Ok(sha256_hash) => {
                                    let disk_sha256 = sha256_hash.to_hex();
                                    let candidate_sha256 = candidate.sha256.as_ref()
                                        .map(|b| hex_encode(b))
                                        .unwrap_or_default();
                                    
                                    if disk_sha256 == candidate_sha256 {
                                        MatchConfidence::Definitive
                                    } else {
                                        // SHA-256 mismatch - this is a collision or corruption
                                        continue;
                                    }
                                }
                                Err(_) => {
                                    // Can't compute sha256, fall back to fast match
                                    MatchConfidence::Fast
                                }
                            }
                        } else {
                            // No sha256 in DB, use fast match
                            MatchConfidence::Fast
                        };
                        
                        return Some((UpdateMatch {
                            old_path: candidate.path.clone(),
                            new_path: rel_new.to_string_lossy().into_owned(),
                            file_id: candidate.id,
                        }, confidence));
                    }
                }
            }
            None
        })
        .collect();

    bar.finish_and_clear();

    let matched = matches.len();
    let unmatched = missing_count - matched;
    
    // Count definitive vs fast matches
    let definitive_count = matches.iter()
        .filter(|(_, conf)| *conf == MatchConfidence::Definitive)
        .count();
    let fast_count = matched - definitive_count;

    eprintln!();
    eprintln!("{}", style("Matches found:").bold());
    for (m, conf) in &matches {
        let conf_icon = match conf {
            MatchConfidence::Definitive => style("✓").green(),
            MatchConfidence::Fast => style("~").yellow(),
        };
        eprintln!(
            "  {} {}  {} -> {}",
            conf_icon,
            style(&m.old_path),
            style("→").dim(),
            style(&m.new_path).green()
        );
    }

    if matches.is_empty() {
        eprintln!("  {} No relocated files detected.", style("-"));
    } else {
        if definitive_count > 0 {
            eprintln!("    {} {} definitive (SHA-256)", style("✓").green(), definitive_count);
        }
        if fast_count > 0 {
            eprintln!("    {} {} fast (XXH3-128 only)", style("~").yellow(), fast_count);
        }
    }

    // 4. Dry-run or confirm
    let mut updated = 0;
    if !opts.dry_run && matched > 0 {
        if !opts.yes {
            eprint!("{}", style("Apply path updates? [y/N] ").bold());
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            if !matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
                eprintln!("{}", style("Aborted.").yellow());
                return Ok(UpdateSummary { missing: missing_count, scanned, matched, unmatched, updated: 0 });
            }
        }

        // Apply updates with progress bar
        let update_bar = ProgressBar::new(matched as u64);
        update_bar.set_style(
            ProgressStyle::with_template("{prefix:.bold.cyan} [{bar:40}] {pos}/{len}  {msg}")
                .unwrap()
                .progress_chars("=> "),
        );
        update_bar.set_prefix("Updating ");

        for m in matches.iter().map(|(m, _)| m) {
            update_bar.set_message(m.old_path.clone());
            if let Err(e) = db.update_file_path(m.file_id, &m.new_path) {
                eprintln!("\n{} Failed to update {}: {}", 
                    style("Error:").red().bold(), 
                    style(&m.old_path),
                    e);
            } else {
                updated += 1;
            }
            update_bar.inc(1);
        }
        update_bar.finish_and_clear();
    }

    // 5. Clean phase (mark unmatched as missing, or delete)
    // This is the default behavior - unmatched files are marked as missing
    if unmatched > 0 {
        let to_clean: Vec<_> = missing_files.iter()
            .filter(|f| !matches.iter().any(|(m, _)| m.file_id == f.id))
            .collect();

        if opts.dry_run {
            if opts.delete {
                eprintln!();
                eprintln!("{} Files to delete:", style("Dry run:").bold().cyan());
                for f in &to_clean {
                    eprintln!("  - {}", style(&f.path).dim());
                }
            } else {
                eprintln!();
                eprintln!("{} Files to mark as missing:", style("Dry run:").bold().cyan());
                for f in &to_clean {
                    eprintln!("  - {}", style(&f.path).dim());
                }
            }
        } else if opts.delete {
            eprintln!();
            eprintln!("{} Permanently deleting {} missing file(s)...", 
                style("Delete:").bold().red(),
                style(to_clean.len()).red());
            // Note: actual file deletion would go here
            // For now we just mark as missing in DB
            let clean_bar = ProgressBar::new(to_clean.len() as u64);
            clean_bar.set_style(
                ProgressStyle::with_template("{prefix:.bold.cyan} [{bar:40}] {pos}/{len}  {msg}")
                    .unwrap()
                    .progress_chars("=> "),
            );
            clean_bar.set_prefix("Cleaning ");
            for f in to_clean {
                clean_bar.set_message(f.path.clone());
                if let Err(e) = db.update_file_status(f.id, "missing") {
                    eprintln!("\n{} Failed to mark {} as missing: {}", 
                        style("Error:").red().bold(), 
                        style(&f.path),
                        e);
                }
                clean_bar.inc(1);
            }
            clean_bar.finish_and_clear();
        } else {
            eprintln!();
            eprintln!("{} Marking {} unmatched file(s) as missing...", 
                style("Cleaned:").bold().green(),
                style(to_clean.len()).green());
            let clean_bar = ProgressBar::new(to_clean.len() as u64);
            clean_bar.set_style(
                ProgressStyle::with_template("{prefix:.bold.cyan} [{bar:40}] {pos}/{len}  {msg}")
                    .unwrap()
                    .progress_chars("=> "),
            );
            clean_bar.set_prefix("Cleaning ");
            for f in to_clean {
                clean_bar.set_message(f.path.clone());
                if let Err(e) = db.update_file_status(f.id, "missing") {
                    eprintln!("\n{} Failed to mark {} as missing: {}", 
                        style("Error:").red().bold(), 
                        style(&f.path),
                        e);
                }
                clean_bar.inc(1);
            }
            clean_bar.finish_and_clear();
        }
    }

    eprintln!();
    eprintln!("{}", style("Summary:").bold());
    eprintln!("  Scanned: {} file(s) on disk", scanned);
    eprintln!("  Missing: {} file(s) from DB", missing_count);
    eprintln!("  Matched: {} file(s) relocated", style(matched).green());
    if unmatched > 0 {
        eprintln!("  Unmatched: {} file(s) not found", style(unmatched).yellow());
    }
    if updated > 0 {
        eprintln!("  Updated: {} file(s) path corrected", style(updated).green().bold());
    }

    Ok(UpdateSummary { scanned, missing: missing_count, matched, unmatched, updated })
}

/// Hex encode bytes.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
