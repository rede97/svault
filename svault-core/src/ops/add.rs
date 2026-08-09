//! `svault add` — register files already inside the vault.
//!
//! Uses the shared pipeline stages:
//! - Stage A: Scan (pipeline::scan)
//! - Stage B: CRC32C (pipeline::crc)
//! - Lookup: DB duplicate check (inline, real-time)
//! - Stage D: Hash (pipeline::hash)
//! - Stage E: Insert (pipeline::insert)

use serde::Serialize;

use crate::config::Config;
use crate::db::Db;
use crate::event::{Event, EventSink, Hint, ItemStatus, Phase, PhaseContext, Summary};
use crate::ops::check_duplicate_by_hash;
use crate::pipeline;

/// Summary of an `add` operation.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AddSummary {
    pub total: usize,
    pub added: usize,
    pub duplicate: usize,
    pub skipped: usize,
    pub failed: usize,
    /// Files detected as vault-internal moves
    pub moved: usize,
}

/// Options for `svault add`.
pub struct AddOptions {
    /// Directories inside the vault to register (git-add style: one or more).
    pub paths: Vec<std::path::PathBuf>,
    pub vault_root: std::path::PathBuf,
    /// Compute SHA-256 for definitive identity.
    pub full_id: bool,
    /// Skip the interactive y/N confirmation after the scan phase.
    pub yes: bool,
}

/// Run `add` on a directory inside the vault.
pub fn run_add(
    opts: AddOptions,
    db: &Db,
    sink: &dyn EventSink,
    interactor: &dyn crate::event::Interactor,
) -> anyhow::Result<AddSummary> {
    let config = Config::load(&opts.vault_root)?;
    let exts: Vec<&str> = config
        .import
        .allowed_extensions
        .iter()
        .map(|s| s.as_str())
        .collect();

    // ------------------------------------------------------------------
    // Stage A+B: Scan + full XXH3-128 (no region fingerprint for dedup —
    // vault-internal files may be edited in place, and the fingerprint's
    // blind zone would silently absorb middle edits as duplicates)
    // ------------------------------------------------------------------
    sink.emit(&Event::PhaseStarted {
        phase: Phase::Scan,
        total: None,
        context: PhaseContext::both(opts.paths[0].clone(), opts.vault_root.clone()),
    });

    // Every root must live inside the vault (add registers in-place files).
    let vault_canon =
        dunce::canonicalize(&opts.vault_root).unwrap_or_else(|_| opts.vault_root.clone());
    for path in &opts.paths {
        let canon = dunce::canonicalize(path).unwrap_or_else(|_| path.clone());
        if !canon.starts_with(&vault_canon) {
            anyhow::bail!(
                "add path must be inside the vault: {} (vault root: {})",
                path.display(),
                vault_canon.display()
            );
        }
    }

    // Merge per-root scan streams into one channel (roots are drained
    // sequentially; each root still scans/hashes in parallel internally).
    let (scan_tx, scan_rx) = std::sync::mpsc::channel();
    for path in &opts.paths {
        let rx = pipeline::scan::scan_stream(path, &exts, &crate::fs::ScanFilter::default())?;
        for item in rx {
            if scan_tx.send(item).is_err() {
                break;
            }
        }
    }
    drop(scan_tx);
    let hash_rx = pipeline::fingerprint::compute_full_hashes_stream(scan_rx);

    let mut lookup_results = Vec::new();
    let mut moved_files: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut total_files = 0usize;

    for result in hash_rx {
        total_files += 1;

        let full_hash = match result.fingerprint {
            Ok(h) => h,
            Err(e) => {
                sink.emit(&Event::ScanItem {
                    path: result.file.path.clone(),
                    size: result.file.size,
                    mtime_ms: result.file.mtime_ms,
                    status: ItemStatus::Failed,
                    error: Some(format!("hash computation failed: {}", e)),
                });
                continue;
            }
        };

        let ext = result
            .file
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let raw_unique_id = if crate::media::raw_id::is_raw_file(ext) {
            crate::media::raw_id::extract_raw_id_if_raw(&result.file.path)
                .and_then(|raw_id| crate::media::raw_id::get_fingerprint_string(&raw_id))
        } else {
            None
        };

        let entry = pipeline::types::FingerprintEntry {
            file: pipeline::types::FileEntry {
                path: result.file.path.clone(),
                size: result.file.size,
                mtime_ms: result.file.mtime_ms,
            },
            src_path: None,
            staged_path: None,
            // Region fingerprint is not computed for add (dedup uses the
            // full hash); Stage D reuses the precomputed full XXH3-128.
            fingerprint: Vec::new(),
            raw_unique_id,
            precomputed_hash: Some(full_hash.clone()),
        };

        let check_result = check_duplicate_by_hash(&entry, db, &opts.vault_root, &full_hash);
        let item_status = match &check_result {
            pipeline::CheckResult::New | pipeline::CheckResult::Recover { .. } => ItemStatus::New,
            pipeline::CheckResult::Duplicate => ItemStatus::Duplicate,
            pipeline::CheckResult::Moved { .. } => ItemStatus::MovedInVault,
        };
        sink.emit(&Event::ScanItem {
            path: result.file.path.clone(),
            size: result.file.size,
            mtime_ms: result.file.mtime_ms,
            status: item_status,
            error: None,
        });

        match check_result {
            pipeline::CheckResult::Duplicate => {
                lookup_results.push(pipeline::types::LookupResult {
                    entry,
                    status: pipeline::types::FileStatus::LikelyCacheDuplicate,
                });
            }
            pipeline::CheckResult::Moved { old_path } => {
                moved_files.push((result.file.path, old_path));
                // Not added to lookup_results — handled separately
            }
            pipeline::CheckResult::Recover { .. } | pipeline::CheckResult::New => {
                lookup_results.push(pipeline::types::LookupResult {
                    entry,
                    status: pipeline::types::FileStatus::LikelyNew,
                });
            }
        }
    }

    sink.emit(&Event::PhaseFinished { phase: Phase::Scan });

    let (new_files, dup_files) = pipeline::lookup::filter_new(lookup_results, false);
    let likely_dup = dup_files.len();
    let moved_count = moved_files.len();
    let failed_scan = total_files.saturating_sub(new_files.len() + dup_files.len() + moved_count);

    sink.emit(&Event::Preflight {
        source: opts.paths[0].clone(),
        total: total_files,
        new: new_files.len(),
        duplicate: likely_dup,
        moved: moved_count,
        failed: failed_scan,
    });

    // Early exit: only moved files, nothing to add
    if new_files.is_empty() && moved_count > 0 {
        sink.emit(&Event::Hint(Hint::OnlyMoved {
            moved: moved_files.clone(),
            vault_root: opts.vault_root.clone(),
        }));
        return Ok(AddSummary {
            total: total_files,
            skipped: likely_dup,
            moved: moved_count,
            ..Default::default()
        });
    }

    // Nothing to register — all duplicates (and no moves to hint about).
    if new_files.is_empty() {
        let summary = AddSummary {
            total: total_files,
            duplicate: likely_dup,
            skipped: 0,
            failed: failed_scan,
            moved: 0,
            ..Default::default()
        };
        sink.emit(&Event::Summary(Summary::Add(summary.clone())));
        return Ok(summary);
    }

    // Confirm before writing anything (same contract as import).
    if !opts.yes && !interactor.confirm("Proceed with add?") {
        let summary = AddSummary {
            total: total_files,
            duplicate: likely_dup,
            moved: moved_count,
            ..Default::default()
        };
        sink.emit(&Event::Summary(Summary::Add(summary.clone())));
        return Ok(summary);
    }

    // Session journal: persist the registration plan before Stage D.
    // add copies nothing (no staging subtree); the plan makes an
    // interrupted add session visible and self-describing.
    let session_id = crate::ops::utils::session_id_now();
    let session_dir = crate::session::session_dir(
        &opts.vault_root,
        crate::verify::manifest::SessionType::Add,
        &session_id,
    );
    let plan = crate::session::AddPlan {
        session_id: session_id.clone(),
        session_type: crate::verify::manifest::SessionType::Add,
        target_dirs: opts.paths.clone(),
        created_at: crate::ops::utils::unix_now_ms(),
        files: new_files
            .iter()
            .map(|e| {
                let rel = e
                    .file
                    .path
                    .strip_prefix(&opts.vault_root)
                    .unwrap_or(&e.file.path)
                    .to_string_lossy()
                    .replace('\\', "/");
                crate::session::AddPlanEntry {
                    path: rel,
                    size: e.file.size,
                    xxh3_128: e
                        .precomputed_hash
                        .as_deref()
                        .unwrap_or_default()
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect(),
                }
            })
            .collect(),
    };
    crate::session::write_json_atomic(&session_dir.join(crate::session::PLAN_FILE), &plan)
        .map_err(|e| anyhow::anyhow!("cannot write add plan: {e}"))?;

    // ------------------------------------------------------------------
    // Stage D: Hash
    // ------------------------------------------------------------------
    let hash_total = new_files.len() as u64;
    sink.emit(&Event::PhaseStarted {
        phase: Phase::Hash,
        total: Some(hash_total),
        context: PhaseContext::both(opts.paths[0].clone(), opts.vault_root.clone()),
    });

    let hash_results = pipeline::hash::compute_hashes(new_files, opts.full_id, Some(sink));

    sink.emit(&Event::PhaseFinished { phase: Phase::Hash });

    // Check duplicates (allow same path re-add)
    let hash_results = pipeline::hash::check_duplicates(hash_results, db, &opts.vault_root, true)?;

    // ------------------------------------------------------------------
    // Stage E: Insert
    // ------------------------------------------------------------------
    let insert_opts = pipeline::insert::InsertOptions {
        vault_root: &opts.vault_root,
        session_id: &session_id,
        write_manifest: true,
        source_root: opts.paths.first().map(|p| p.as_path()),
        force: false,
        session_type: crate::verify::manifest::SessionType::Add,
    };

    let pipeline_summary = pipeline::insert::batch_insert(hash_results, db, insert_opts, None)?;

    let summary = AddSummary {
        total: total_files,
        added: pipeline_summary.added,
        duplicate: pipeline_summary.duplicate + likely_dup,
        skipped: pipeline_summary.skipped,
        failed: pipeline_summary.failed + failed_scan,
        moved: moved_count,
    };

    sink.emit(&Event::Summary(Summary::Add(summary.clone())));

    // Post-insert moved hint (if mixed with new files)
    if !moved_files.is_empty() {
        sink.emit(&Event::Hint(Hint::MovedHint {
            moved: moved_files,
            vault_root: opts.vault_root.clone(),
        }));
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{NoopSink, YesInteractor};
    use std::path::Path;

    struct DeclineInteractor;
    impl crate::event::Interactor for DeclineInteractor {
        fn confirm(&self, _message: &str) -> bool {
            false
        }
    }

    /// Vault dir with default config + one unregistered file inside.
    fn setup() -> (tempfile::TempDir, Db) {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        crate::config::Config::write_default(vault).unwrap();
        let incoming = vault.join("incoming");
        std::fs::create_dir_all(&incoming).unwrap();
        std::fs::write(incoming.join("a.jpg"), b"some-bytes").unwrap();
        (tmp, Db::open_in_memory().unwrap())
    }

    fn opts(vault: &Path, yes: bool) -> AddOptions {
        AddOptions {
            paths: vec![vault.join("incoming")],
            vault_root: vault.to_path_buf(),
            full_id: false,
            yes,
        }
    }

    #[test]
    fn decline_registers_nothing_and_writes_no_session() {
        let (tmp, db) = setup();
        let vault = tmp.path().to_path_buf();

        let summary = run_add(opts(&vault, false), &db, &NoopSink, &DeclineInteractor).unwrap();

        assert_eq!(summary.added, 0);
        assert!(db.get_all_files().unwrap().is_empty());
        assert!(
            !crate::session::sessions_root(&vault).exists(),
            "declined add must not create a session journal"
        );
    }

    #[test]
    fn accept_writes_plan_and_manifest_session() {
        let (tmp, db) = setup();
        let vault = tmp.path().to_path_buf();

        let summary = run_add(opts(&vault, true), &db, &NoopSink, &YesInteractor).unwrap();

        assert_eq!(summary.added, 1);
        let add_root = crate::session::sessions_root(&vault).join("add");
        let sessions: Vec<_> = std::fs::read_dir(&add_root)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];

        let plan: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(session.join(crate::session::PLAN_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(plan["session_type"], "add");
        let files = plan["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["path"].as_str().unwrap(), "incoming/a.jpg");
        assert_eq!(files[0]["xxh3_128"].as_str().unwrap().len(), 32);

        assert!(session.join(crate::session::MANIFEST_FILE).exists());
    }
}
