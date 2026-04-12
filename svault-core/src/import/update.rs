//! `svault update` — update database paths for moved or renamed files.
//!
//! Scans the vault directory, computes hashes, and matches them against
//! database records that are marked `imported` but whose paths no longer exist.
//! When a match is found, the file has been moved/renamed outside of Svault.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use jwalk::WalkDir;
use rayon::prelude::*;

use crate::db::Db;
use crate::hash::{sha256_file, xxh3_128_file};
use crate::reporting::{
    HashReporter, Interactor, MatchConfidence, ReporterBuilder, UpdateApplyReporter,
};

/// Convert a path to Unix-style string (forward slashes) for cross-platform storage.
fn path_to_unix_string(path: &Path) -> String {
    let mut result = String::new();
    for (i, component) in path.components().enumerate() {
        if i > 0 {
            result.push('/');
        }
        if let Some(s) = component.as_os_str().to_str() {
            result.push_str(s);
        } else {
            result.push_str(&component.as_os_str().to_string_lossy());
        }
    }
    result
}

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

// MatchConfidence is now defined in crate::reporting

/// Run `update` on the vault.
pub fn run_update<RB: ReporterBuilder, I: Interactor>(
    opts: UpdateOptions,
    db: &Db,
    reporter_builder: &RB,
    interactor: &I,
) -> anyhow::Result<UpdateSummary> {
    // 1. Find missing files in DB
    let missing_files = db.get_missing_files(&opts.vault_root)?;
    let missing_count = missing_files.len();

    if missing_count == 0 {
        let apply_reporter = reporter_builder.update_apply_reporter(0);
        apply_reporter.nothing_to_update();
        return Ok(UpdateSummary::default());
    }

    // 2. Scan vault disk for all files.
    // Keep `.svault` excluded to match the previous traversal behavior.
    let disk_entries: Vec<_> = WalkDir::new(&opts.root)
        .skip_hidden(false)
        .process_read_dir(|_depth, _path, _state, children| {
            children.iter_mut().for_each(|child_result| {
                if let Ok(child) = child_result
                    && child.file_name == OsStr::new(".svault")
                {
                    child.read_children_path = None;
                }
            });
        })
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path())
        .collect();
    let scanned = disk_entries.len();

    if scanned == 0 {
        return Ok(UpdateSummary {
            missing: missing_count,
            ..Default::default()
        });
    }

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
    let hash_reporter = reporter_builder.update_hash_reporter(&opts.vault_root, scanned as u64);

    let matches: Vec<(UpdateMatch, MatchConfidence)> = disk_entries
        .into_par_iter()
        .filter_map(|path| {
            // Get file size for reporter
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            
            // Signal start of hashing this file
            hash_reporter.item_started(&path, size);

            // Helper closure to ensure item_finished is called
            let (result, error): (Option<(UpdateMatch, MatchConfidence)>, Option<String>) = 
                match (|| -> Option<(UpdateMatch, MatchConfidence)> {
                    // Always compute xxh3_128 first (fast)
                    let xxh3_str = xxh3_128_file(&path)
                        .map(|h| hex_encode(&h.to_bytes()))
                        .ok()?;

                    // First: try fast match by xxh3_128
                    let candidates = missing_by_xxh3.get(&xxh3_str)?;
                    let meta = fs::metadata(&path).ok()?;

                    for candidate in candidates {
                        if candidate.size == meta.len() as i64 {
                            let rel_new = path.strip_prefix(&opts.vault_root).unwrap_or(&path);

                            // If candidate has sha256, compute and verify for definitive match
                            let confidence = if candidate.sha256.is_some() {
                                match sha256_file(&path) {
                                    Ok(sha256_hash) => {
                                        let disk_sha256 = sha256_hash.to_hex();
                                        let candidate_sha256 = candidate
                                            .sha256
                                            .as_ref()
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

                            return Some((
                                UpdateMatch {
                                    old_path: candidate.path.clone(),
                                    new_path: path_to_unix_string(rel_new),
                                    file_id: candidate.id,
                                },
                                confidence,
                            ));
                        }
                    }
                    None
                })() {
                Some(m) => (Some(m), None),
                None => (None, None), // No match found is not an error
            };

            // Signal end of hashing this file
            hash_reporter.item_finished(&path, error.as_deref(), size);
            result
        })
        .collect();

    let matched = matches.len();
    let unmatched = missing_count - matched;

    // Report matches
    for (m, conf) in &matches {
        hash_reporter.matched(&m.old_path, &m.new_path, *conf);
    }
    hash_reporter.finish();

    // 4. Dry-run or confirm
    let mut updated = 0;
    let apply_total = if matched > 0 { matched } else { 0 }
        + if unmatched > 0 && !opts.dry_run {
            unmatched
        } else {
            0
        };
    let apply_reporter = reporter_builder.update_apply_reporter(apply_total as u64);

    if !opts.dry_run && matched > 0 {
        if !opts.yes && !interactor.confirm("Apply path updates?") {
            return Ok(UpdateSummary {
                missing: missing_count,
                scanned,
                matched,
                unmatched,
                updated: 0,
            });
        }

        // Apply updates
        for (idx, m) in matches.iter().map(|(m, _)| m).enumerate() {
            if let Err(e) = db.update_file_path(m.file_id, &m.new_path) {
                apply_reporter.error(&format!("Failed to update: {}", e), &m.old_path);
            } else {
                updated += 1;
            }
            apply_reporter.progress((idx + 1) as u64, apply_total as u64);
        }
    }

    // 5. Clean phase (mark unmatched as missing, or delete)
    if unmatched > 0 {
        if opts.dry_run {
            apply_reporter.dry_run_missing(unmatched);
        } else {
            let to_clean: Vec<_> = missing_files
                .iter()
                .filter(|f| !matches.iter().any(|(m, _)| m.file_id == f.id))
                .collect();

            for (idx, f) in to_clean.iter().enumerate() {
                if let Err(e) = db.update_file_status(f.id, "missing") {
                    apply_reporter.error(&format!("Failed to mark as missing: {}", e), &f.path);
                }
                apply_reporter.progress((matched + idx + 1) as u64, apply_total as u64);
            }
        }
    }

    apply_reporter.finish();
    apply_reporter.summary(scanned, missing_count, matched, unmatched, updated);

    Ok(UpdateSummary {
        scanned,
        missing: missing_count,
        matched,
        unmatched,
        updated,
    })
}

/// Hex encode bytes.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
