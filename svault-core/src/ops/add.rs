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
use crate::ops::check_duplicate;
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
    pub path: std::path::PathBuf,
    pub vault_root: std::path::PathBuf,
    /// Compute SHA-256 for definitive identity.
    pub full_id: bool,
}

/// Run `add` on a directory inside the vault.
pub fn run_add(opts: AddOptions, db: &Db, sink: &dyn EventSink) -> anyhow::Result<AddSummary> {
    let config = Config::load(&opts.vault_root)?;
    let exts: Vec<&str> = config
        .import
        .allowed_extensions
        .iter()
        .map(|s| s.as_str())
        .collect();

    // ------------------------------------------------------------------
    // Stage A+B+C: Scan + CRC + Lookup
    // ------------------------------------------------------------------
    sink.emit(&Event::PhaseStarted {
        phase: Phase::Scan,
        total: None,
        context: PhaseContext::both(opts.path.clone(), opts.vault_root.clone()),
    });

    let scan_rx = pipeline::scan::scan_stream(&opts.path, &exts)?;
    let crc_rx = pipeline::crc::compute_crcs_stream(scan_rx);

    let mut lookup_results = Vec::new();
    let mut moved_files: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut total_files = 0usize;

    for result in crc_rx {
        total_files += 1;

        let crc = match result.crc {
            Ok(c) => c,
            Err(e) => {
                sink.emit(&Event::ScanItem {
                    path: result.file.path.clone(),
                    size: result.file.size,
                    mtime_ms: result.file.mtime_ms,
                    status: ItemStatus::Failed,
                    error: Some(format!("CRC computation failed: {}", e)),
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

        let entry = pipeline::types::CrcEntry {
            file: pipeline::types::FileEntry {
                path: result.file.path.clone(),
                size: result.file.size,
                mtime_ms: result.file.mtime_ms,
            },
            src_path: None,
            crc32c: crc,
            raw_unique_id,
            precomputed_hash: None,
        };

        let check_result = check_duplicate(&entry, db, &opts.vault_root, None);
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
        source: opts.path.clone(),
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

    // ------------------------------------------------------------------
    // Stage D: Hash
    // ------------------------------------------------------------------
    let hash_total = new_files.len() as u64;
    sink.emit(&Event::PhaseStarted {
        phase: Phase::Hash,
        total: Some(hash_total),
        context: PhaseContext::both(opts.path.clone(), opts.vault_root.clone()),
    });

    let hash_results = pipeline::hash::compute_hashes(new_files, opts.full_id, Some(sink));

    sink.emit(&Event::PhaseFinished { phase: Phase::Hash });

    // Check duplicates (allow same path re-add)
    let hash_results = pipeline::hash::check_duplicates(hash_results, db, &opts.vault_root, true)?;

    // ------------------------------------------------------------------
    // Stage E: Insert
    // ------------------------------------------------------------------
    let session_id = crate::ops::utils::session_id_now();
    let insert_opts = pipeline::insert::InsertOptions {
        vault_root: &opts.vault_root,
        session_id: &session_id,
        write_manifest: true,
        source_root: Some(&opts.path),
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
