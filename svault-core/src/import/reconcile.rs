//! `svault reconcile` — update database paths for moved or renamed files.
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

use crate::config::HashAlgorithm;
use crate::db::Db;
use crate::hash::{sha256_file, xxh3_128_file};
use crate::vfs::system::SystemFs;
use crate::vfs::VfsBackend;

/// Summary of a `reconcile` operation.
#[derive(Debug, Default)]
pub struct ReconcileSummary {
    pub scanned: usize,
    pub missing: usize,
    pub matched: usize,
    pub unmatched: usize,
    pub updated: usize,
}

/// Options for `svault reconcile`.
pub struct ReconcileOptions {
    pub root: std::path::PathBuf,
    pub vault_root: std::path::PathBuf,
    pub dry_run: bool,
    pub yes: bool,
    /// Mark unmatched files as missing in database.
    pub clean: bool,
    /// Actually delete files (if they exist).
    pub delete: bool,
}

/// A single reconciliation match.
#[derive(Debug)]
pub struct ReconcileMatch {
    pub old_path: String,
    pub new_path: String,
    pub file_id: i64,
}

/// Run `reconcile` on the vault.
pub fn run_reconcile(opts: ReconcileOptions, db: &Db) -> anyhow::Result<ReconcileSummary> {
    // 1. Find missing files in DB
    let missing_files = db.get_missing_files(&opts.vault_root)?;
    let missing_count = missing_files.len();

    if missing_count == 0 {
        eprintln!("{} All tracked files exist on disk — nothing to reconcile.",
            style("Reconcile:").bold().green());
        return Ok(ReconcileSummary::default());
    }

    eprintln!(
        "{} {} tracked file(s) missing from disk",
        style("Reconcile:").bold().cyan(),
        style(missing_count).cyan()
    );

    // 2. Scan vault disk for all files
    let fs = SystemFs::open(&opts.root)?;
    let disk_entries = fs.walk(Path::new(""), &[])?;
    let scanned = disk_entries.len();

    if scanned == 0 {
        eprintln!("{} No files found on disk to match against.",
            style("Warning:").yellow().bold());
        return Ok(ReconcileSummary { missing: missing_count, ..Default::default() });
    }

    eprintln!(
        "  Scanning {} file(s) on disk for matches...",
        style(scanned).cyan()
    );

    // Determine hash algorithm from missing files (prefer sha256, fallback to xxh3_128)
    let hash_algo = if missing_files.iter().any(|f| f.sha256.is_some()) {
        HashAlgorithm::Sha256
    } else {
        HashAlgorithm::Xxh3_128
    };

    // Build index of missing files by hash
    let mut missing_by_hash: HashMap<String, Vec<&crate::db::files::FileRow>> = HashMap::new();
    for row in &missing_files {
        let hash_key = match &hash_algo {
            HashAlgorithm::Sha256 => row.sha256.as_ref().map(|b| hex_encode(b)),
            HashAlgorithm::Xxh3_128 => row.xxh3_128.as_ref().map(|b| hex_encode(b)),
        };
        if let Some(key) = hash_key {
            missing_by_hash.entry(key).or_default().push(row);
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

    let matches: Vec<ReconcileMatch> = disk_entries
        .into_par_iter()
        .filter_map(|e| {
            let filename = e.path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            bar.set_message(filename);

            let hash_result = match hash_algo {
                HashAlgorithm::Xxh3_128 => xxh3_128_file(&e.path)
                    .map(|h| hex_encode(&h.to_bytes())),
                HashAlgorithm::Sha256 => sha256_file(&e.path)
                    .map(|h| h.to_hex()),
            };

            bar.inc(1);
            let hash_str = match hash_result {
                Ok(h) => h,
                Err(_) => return None,
            };

            if let Some(candidates) = missing_by_hash.get(&hash_str) {
                // Verify size matches to avoid hash collision false positives
                let meta = fs::metadata(&e.path).ok()?;
                for candidate in candidates {
                    if candidate.size == meta.len() as i64 {
                        let rel_new = e.path.strip_prefix(&opts.vault_root).unwrap_or(&e.path);
                        return Some(ReconcileMatch {
                            old_path: candidate.path.clone(),
                            new_path: rel_new.to_string_lossy().into_owned(),
                            file_id: candidate.id,
                        });
                    }
                }
            }
            None
        })
        .collect();

    bar.finish_and_clear();

    let matched = matches.len();
    let unmatched = missing_count - matched;

    eprintln!();
    eprintln!("{}", style("Matches found:").bold());
    for m in &matches {
        eprintln!(
            "  {}  {} -> {}",
            style("→").green(),
            style(&m.old_path),
            style(&m.new_path).green()
        );
    }

    if matches.is_empty() {
        eprintln!("  {} No relocated files detected.", style("-"));
    }

    // 4. Dry-run or confirm
    let mut updated = 0;
    if !opts.dry_run && matched > 0 {
        if !opts.yes {
            eprint!("{}", style("Apply path updates? [y/N] ").bold());
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            if !matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
                eprintln!("{}", style("Aborted. No changes made.").yellow());
                return Ok(ReconcileSummary {
                    scanned,
                    missing: missing_count,
                    matched,
                    unmatched,
                    updated: 0,
                });
            }
        }

        let _now_ms = crate::import::utils::unix_now_ms();
        for m in &matches {
            let payload = serde_json::json!({
                "old_path": m.old_path,
                "new_path": m.new_path,
            }).to_string();

            if let Err(e) = db.append_event(
                "file.path_updated", "file", m.file_id, &payload,
                |conn| {
                    conn.execute(
                        "UPDATE files SET path = ?1 WHERE id = ?2",
                        rusqlite::params![m.new_path, m.file_id],
                    )?;
                    Ok(())
                },
            ) {
                eprintln!("  {} Failed to update {}: {}", style("Error").red(), m.old_path, e);
                continue;
            }
            updated += 1;
        }
    }

    // 5. Clean unmatched files (if requested)
    let mut cleaned = 0;
    let mut deleted = 0;
    if opts.clean && unmatched > 0 {
        // Get list of unmatched files
        let matched_ids: std::collections::HashSet<i64> = matches.iter().map(|m| m.file_id).collect();
        let unmatched_files: Vec<_> = missing_files
            .into_iter()
            .filter(|f| !matched_ids.contains(&f.id))
            .collect();

        if !unmatched_files.is_empty() {
            eprintln!();
            if opts.delete {
                eprintln!("{}", style("Files to delete:").bold().red());
            } else {
                eprintln!("{}", style("Files to mark as missing:").bold().yellow());
            }
            for f in &unmatched_files {
                eprintln!("  {} {}", style("-").red(), style(&f.path).dim());
            }

            let should_proceed = if opts.dry_run {
                false
            } else if opts.yes {
                true
            } else {
                eprint!("{}", style("Proceed? [y/N] ").bold());
                let mut line = String::new();
                std::io::stdin().read_line(&mut line)?;
                matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
            };

            if should_proceed {
                for f in unmatched_files {
                    // Delete file if requested (and it somehow exists)
                    if opts.delete {
                        let full_path = opts.vault_root.join(&f.path);
                        if full_path.exists() {
                            if let Err(e) = fs::remove_file(&full_path) {
                                eprintln!("  {} Failed to delete {}: {}", style("Error").red(), f.path, e);
                                continue;
                            }
                            deleted += 1;
                        }
                    }

                    // Update status to 'missing'
                    if let Err(e) = db.update_file_status(f.id, "missing") {
                        eprintln!("  {} Failed to update status for {}: {}", style("Error").red(), f.path, e);
                        continue;
                    }
                    cleaned += 1;
                }
            }
        }
    }

    eprintln!();
    eprintln!("{}", style("Summary:").bold());
    eprintln!("  Scanned:    {}", style(scanned).cyan());
    eprintln!("  Missing:    {}", style(missing_count).yellow());
    eprintln!("  Matched:    {}", style(matched).green());
    eprintln!("  Unmatched:  {}", style(unmatched));
    if !opts.dry_run {
        eprintln!("  Updated:    {}", style(updated).green());
        if opts.clean {
            eprintln!("  Cleaned:    {}", style(cleaned).yellow());
            if opts.delete {
                eprintln!("  Deleted:    {}", style(deleted).red());
            }
        }
    }

    Ok(ReconcileSummary {
        scanned,
        missing: missing_count,
        matched,
        unmatched,
        updated,
    })
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
