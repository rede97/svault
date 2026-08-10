//! `svault update` — update database paths for moved or renamed files.
//!
//! Scans the vault directory, computes hashes, and matches them against
//! database records that are marked `imported` but whose paths no longer exist.
//! When a match is found, the file has been moved/renamed outside of Svault.
//!
//! Missing files are marked `missing` in the database. **Svault never deletes
//! user files** (core principle) — there is intentionally no delete option.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use jwalk::WalkDir;
use rayon::prelude::*;
use serde::Serialize;

use crate::db::Db;
use crate::event::{
    Event, EventSink, Hint, Interactor, MatchConfidence, Phase, PhaseContext, Summary,
};
use crate::hash::{sha256_file, xxh3_128_file};

/// Convert a path to Unix-style string (forward slashes) for cross-platform storage.
fn path_to_unix_string(path: &Path) -> String {
    let path_str = path.to_string_lossy();
    let normalized = path_str.replace('\\', "/");

    // Remove leading slash if present (from absolute paths)
    normalized
        .strip_prefix('/')
        .map(String::from)
        .unwrap_or(normalized)
}

/// Summary of an `update` operation.
#[derive(Debug, Clone, Default, Serialize)]
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
}

/// A single update match.
#[derive(Debug)]
pub struct UpdateMatch {
    pub old_path: String,
    pub new_path: String,
    pub file_id: i64,
}

/// Run `update` on the vault.
pub fn run_update(
    opts: UpdateOptions,
    db: &Db,
    sink: &dyn EventSink,
    interactor: &dyn Interactor,
) -> anyhow::Result<UpdateSummary> {
    // 1. Find missing files in DB
    let missing_files = db.get_missing_files(&opts.vault_root)?;
    let missing_count = missing_files.len();

    if missing_count == 0 {
        sink.emit(&Event::Hint(Hint::NothingToUpdate));
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
        if let Some(xxh3) = row.xxh3_128.as_ref().map(|b| hex_encode(b)) {
            missing_by_xxh3.entry(xxh3).or_default().push(row);
        }
        if let Some(sha256) = row.sha256.as_ref().map(|b| hex_encode(b)) {
            missing_by_sha256.entry(sha256).or_default().push(row);
        }
    }

    // 3. Hash all disk files and look for matches
    sink.emit(&Event::PhaseStarted {
        phase: Phase::Hash,
        total: Some(scanned as u64),
        context: PhaseContext::vault(opts.vault_root.clone()),
    });

    let matches: Vec<(UpdateMatch, MatchConfidence)> = disk_entries
        .into_par_iter()
        .filter_map(|path| {
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

            sink.emit(&Event::HashStarted {
                path: path.clone(),
                bytes: size,
            });

            let result: Option<(UpdateMatch, MatchConfidence)> = (|| {
                // Always compute xxh3_128 first (fast)
                let xxh3_str = xxh3_128_file(&path)
                    .map(|h| hex_encode(&h.to_bytes()))
                    .ok()?;

                // Try fast match by xxh3_128
                let candidates = missing_by_xxh3.get(&xxh3_str)?;
                let meta = fs::metadata(&path).ok()?;

                for candidate in candidates {
                    if candidate.size == meta.len() as i64 {
                        let rel_new = path.strip_prefix(&opts.vault_root).unwrap_or(&path);

                        // If candidate has sha256, verify for definitive match
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
                                        // SHA-256 mismatch — collision or corruption
                                        continue;
                                    }
                                }
                                Err(_) => MatchConfidence::Fast,
                            }
                        } else {
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
            })();

            sink.emit(&Event::HashFinished {
                path: path.clone(),
                bytes: size,
                error: None,
            });
            result
        })
        .collect();

    let matched = matches.len();
    let unmatched = missing_count - matched;

    for (m, conf) in &matches {
        sink.emit(&Event::RelocateMatched {
            old_path: m.old_path.clone(),
            new_path: m.new_path.clone(),
            confidence: *conf,
        });
    }
    sink.emit(&Event::PhaseFinished { phase: Phase::Hash });

    // 4. Dry-run or confirm
    let to_clean: Vec<_> = missing_files
        .iter()
        .filter(|f| !matches.iter().any(|(m, _)| m.file_id == f.id))
        .collect();

    let mut updated = 0;
    let apply_total = if matched > 0 { matched } else { 0 }
        + if unmatched > 0 && !opts.dry_run {
            unmatched
        } else {
            0
        };

    sink.emit(&Event::PhaseStarted {
        phase: Phase::Apply,
        total: Some(apply_total as u64),
        context: PhaseContext::default(),
    });

    // Session journal: the apply plan is persisted after the confirmation
    // gate and before the first DB write (dry-run writes nothing).
    let mut apply_errors: Vec<(String, String)> = Vec::new();
    let mut session_dir: Option<std::path::PathBuf> = None;
    let mut session_id = String::new();
    let write_plan = |confirmed: bool,
                      session_dir: &mut Option<std::path::PathBuf>,
                      session_id: &mut String|
     -> anyhow::Result<()> {
        if opts.dry_run || !confirmed || (matched == 0 && unmatched == 0) {
            return Ok(());
        }
        *session_id = crate::ops::utils::session_id_now();
        let dir = crate::session::session_dir(
            &opts.vault_root,
            crate::verify::manifest::SessionType::Update,
            session_id,
        );
        let plan = crate::session::UpdatePlan {
            session_id: session_id.clone(),
            session_type: crate::verify::manifest::SessionType::Update,
            root: opts.root.clone(),
            created_at: crate::ops::utils::unix_now_ms(),
            moves: matches
                .iter()
                .map(|(m, conf)| crate::session::UpdatePlanMove {
                    old_path: m.old_path.clone(),
                    new_path: m.new_path.clone(),
                    confidence: match conf {
                        MatchConfidence::Definitive => "definitive".to_string(),
                        MatchConfidence::Fast => "fast".to_string(),
                    },
                })
                .collect(),
            mark_missing: to_clean.iter().map(|f| f.path.clone()).collect(),
        };
        crate::session::write_json_atomic(&dir.join(crate::session::PLAN_FILE), &plan)
            .map_err(|e| anyhow::anyhow!("cannot write update plan: {e}"))?;
        *session_dir = Some(dir);
        Ok(())
    };

    if !opts.dry_run && matched > 0 {
        if !opts.yes && !interactor.confirm("Apply path updates?") {
            sink.emit(&Event::PhaseFinished {
                phase: Phase::Apply,
            });
            return Ok(UpdateSummary {
                missing: missing_count,
                scanned,
                matched,
                unmatched,
                updated: 0,
            });
        }

        write_plan(true, &mut session_dir, &mut session_id)?;

        // Apply updates
        for (idx, m) in matches.iter().map(|(m, _)| m).enumerate() {
            if let Err(e) = db.update_file_path(m.file_id, &m.new_path) {
                let message = format!("Failed to update: {}", e);
                apply_errors.push((m.old_path.clone(), message.clone()));
                sink.emit(&Event::ApplyError {
                    path: m.old_path.clone(),
                    message,
                });
            } else {
                updated += 1;
            }
            sink.emit(&Event::Progress {
                phase: Phase::Apply,
                done: (idx + 1) as u64,
                total: apply_total as u64,
            });
        }
    }

    // 5. Clean phase (mark unmatched as missing — never delete files)
    if unmatched > 0 {
        if opts.dry_run {
            sink.emit(&Event::Hint(Hint::DryRunMissing { count: unmatched }));
        } else {
            // Pure missing-marking was never gated by a confirmation; write
            // the plan here when the matched>0 branch did not.
            write_plan(matched == 0, &mut session_dir, &mut session_id)?;

            let mut marked_missing = 0usize;
            for (idx, f) in to_clean.iter().enumerate() {
                if let Err(e) = db.update_file_status(f.id, "missing") {
                    let message = format!("Failed to mark as missing: {}", e);
                    apply_errors.push((f.path.clone(), message.clone()));
                    sink.emit(&Event::ApplyError {
                        path: f.path.clone(),
                        message,
                    });
                } else {
                    marked_missing += 1;
                }
                sink.emit(&Event::Progress {
                    phase: Phase::Apply,
                    done: (matched + idx + 1) as u64,
                    total: apply_total as u64,
                });
            }
            let _ = marked_missing;
        }
    }

    // Session manifest: per-item outcomes (only when a plan was written).
    if let Some(dir) = &session_dir {
        let now = crate::ops::utils::unix_now_ms();
        let mut records = Vec::new();
        for (m, _) in &matches {
            let err = apply_errors
                .iter()
                .find(|(p, _)| p == &m.old_path)
                .map(|(_, e)| e.clone());
            let row = missing_files.iter().find(|f| f.id == m.file_id);
            records.push(crate::verify::manifest::ImportRecord {
                src_path: Path::new(&m.old_path).to_path_buf(),
                dest_path: Some(Path::new(&m.new_path).to_path_buf()),
                size: row.map(|r| r.size as u64).unwrap_or(0),
                mtime_ms: row.map(|r| r.mtime).unwrap_or(0),
                fingerprint: row
                    .and_then(|r| r.fingerprint.as_ref())
                    .map(|b| hex_encode(b))
                    .unwrap_or_default(),
                xxh3_128: row.and_then(|r| r.xxh3_128.as_ref()).map(|b| hex_encode(b)),
                sha256: row.and_then(|r| r.sha256.as_ref()).map(|b| hex_encode(b)),
                imported_at: now,
                status: if err.is_some() {
                    crate::verify::manifest::ItemStatus::Failed
                } else {
                    crate::verify::manifest::ItemStatus::Moved
                },
                error: err,
            });
        }
        for f in &to_clean {
            let err = apply_errors
                .iter()
                .find(|(p, _)| p == &f.path)
                .map(|(_, e)| e.clone());
            records.push(crate::verify::manifest::ImportRecord {
                src_path: Path::new(&f.path).to_path_buf(),
                dest_path: None,
                size: f.size as u64,
                mtime_ms: f.mtime,
                fingerprint: f
                    .fingerprint
                    .as_ref()
                    .map(|b| hex_encode(b))
                    .unwrap_or_default(),
                xxh3_128: f.xxh3_128.as_ref().map(|b| hex_encode(b)),
                sha256: f.sha256.as_ref().map(|b| hex_encode(b)),
                imported_at: now,
                status: if err.is_some() {
                    crate::verify::manifest::ItemStatus::Failed
                } else {
                    crate::verify::manifest::ItemStatus::Missing
                },
                error: err,
            });
        }
        if !records.is_empty() {
            let failed = apply_errors.len();
            let manifest = crate::verify::manifest::ImportManifest {
                session_id,
                session_type: crate::verify::manifest::SessionType::Update,
                source_root: opts.root.clone(),
                imported_at: now,
                hash_algorithm: "xxh3_128".to_string(),
                summary: Some(crate::verify::manifest::ManifestSummary {
                    total: records.len(),
                    added: records.len() - failed,
                    duplicate: 0,
                    failed,
                    skipped: 0,
                }),
                files: records,
            };
            let manager = crate::verify::manifest::ManifestManager::new(&opts.vault_root);
            manager.save(&manifest)?;
        }
        let _ = dir;
    }

    sink.emit(&Event::PhaseFinished {
        phase: Phase::Apply,
    });

    let summary = UpdateSummary {
        scanned,
        missing: missing_count,
        matched,
        unmatched,
        updated,
    };
    sink.emit(&Event::Summary(Summary::Update(summary.clone())));

    Ok(summary)
}

/// Hex encode bytes.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_to_unix_string_update_module() {
        let windows_path = Path::new("2024\\03\\file.jpg");
        assert_eq!(path_to_unix_string(windows_path), "2024/03/file.jpg");
    }

    #[test]
    fn test_path_to_unix_string_unix_stays_unix() {
        let unix_path = Path::new("2024/03/file.jpg");
        assert_eq!(path_to_unix_string(unix_path), "2024/03/file.jpg");
    }

    #[test]
    fn test_path_to_unix_string_mixed_separators() {
        let mixed_path = Path::new("2024/03\\file.jpg");
        assert_eq!(path_to_unix_string(mixed_path), "2024/03/file.jpg");
    }
}
