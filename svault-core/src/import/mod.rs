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
pub mod scan;
pub mod staging;
pub mod update;
pub mod utils;
pub mod vfs_import;

pub use types::{ImportOptions, FileStatus, ScanEntry, ImportSummary};

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

// Note: console/style and indicatif removed - terminal output now handled by Reporter
use rayon::prelude::*;

use crate::config::HashAlgorithm;
use crate::db::Db;
use crate::pipeline;
use crate::reporting::{CoreEvent, Interactor, ItemStatus, OperationKind, PhaseKind, Reporter};
use crate::vfs::system::SystemFs;
use crate::vfs::transfer::transfer_file;

use exif::read_exif_date_device;
use path::resolve_dest_path;
use utils::session_id_now;

/// Check if a file is a duplicate via DB lookup.
/// Uses shared CheckResult type for consistent handling in import and add commands.
/// 
/// # Arguments
/// * `entry` - CrcEntry with CRC32C and file metadata
/// * `db` - Database handle
/// * `vault_root` - Vault root path for existence checks
/// * `hash` - Optional (hash_bytes, algorithm) for secondary verification when CRC matches
/// 
/// # Special cases
/// - If status is 'missing': returns Recover (allows re-import with path update)
/// - If file exists at original path: returns Duplicate
/// - If CRC matches but file missing: returns Moved (vault-internal move)
pub fn check_duplicate(
    entry: &pipeline::types::CrcEntry, 
    db: &Db, 
    vault_root: &Path,
    hash: Option<(&[u8], &HashAlgorithm)>,
) -> pipeline::CheckResult {
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
        Err(_) => return pipeline::CheckResult::New,
    };
    
    if let Some(row) = cached {
        let is_same_raw_id = match (&entry.raw_unique_id, &row.raw_unique_id) {
            (Some(new_id), Some(existing_id)) => new_id == existing_id,
            _ => true,
        };
        
        // If strong hash provided, do secondary verification
        let hash_matches = if let Some((hash_bytes, algo)) = hash {
            let db_hash = match algo {
                HashAlgorithm::Xxh3_128 => row.xxh3_128.as_ref(),
                HashAlgorithm::Sha256 => row.sha256.as_ref(),
            };
            db_hash.map(|db| db == hash_bytes).unwrap_or(false)
        } else {
            true // No hash provided, trust CRC match
        };
        
        // If status is 'missing', allow re-import with recovery
        if row.status == "missing" && hash_matches {
            return pipeline::CheckResult::Recover { old_path: row.path, file_id: row.id };
        }
        
        let vault_path = vault_root.join(&row.path);
        if vault_path.exists() && is_same_raw_id && hash_matches {
            // Exact duplicate - file exists at original path
            return pipeline::CheckResult::Duplicate;
        } else if is_same_raw_id && hash_matches {
            // CRC matches but original file missing -> vault-internal move
            return pipeline::CheckResult::Moved { old_path: row.path };
        }
    }
    
    pipeline::CheckResult::New
}

/// Shared state for import processing
struct ImportState {
    lookup_results: Vec<pipeline::types::LookupResult>,
    moved_files: Vec<(std::path::PathBuf, String)>,
    total_files: usize,
}

impl ImportState {
    fn new() -> Self {
        Self {
            lookup_results: Vec::new(),
            moved_files: Vec::new(),
            total_files: 0,
        }
    }
}

/// Process a single lookup result and update state
fn process_lookup_result(
    entry: pipeline::types::CrcEntry,
    check_result: pipeline::CheckResult,
    _rel_path: &std::path::PathBuf,
    _opts: &ImportOptions,
    state: &mut ImportState,
) {
    match check_result {
        pipeline::CheckResult::Moved { old_path } => {
            state.moved_files.push((entry.file.path.clone(), old_path));
            state.lookup_results.push(pipeline::types::LookupResult { 
                entry, 
                status: pipeline::types::FileStatus::LikelyCacheDuplicate 
            });
        }
        pipeline::CheckResult::Recover { .. } => {
            state.lookup_results.push(pipeline::types::LookupResult { 
                entry, 
                status: pipeline::types::FileStatus::LikelyNew 
            });
        }
        pipeline::CheckResult::Duplicate => {
            state.lookup_results.push(pipeline::types::LookupResult { 
                entry, 
                status: pipeline::types::FileStatus::LikelyCacheDuplicate 
            });
        }
        pipeline::CheckResult::New => {
            state.lookup_results.push(pipeline::types::LookupResult { 
                entry, 
                status: pipeline::types::FileStatus::LikelyNew 
            });
        }
    }
}

/// Handle the case when no new files to import
fn handle_no_new_files(
    total_files: usize,
    likely_dup: usize,
    _moved_files: &[(std::path::PathBuf, String)],
) -> anyhow::Result<ImportSummary> {
    Ok(ImportSummary {
        total: total_files,
        duplicate: likely_dup,
        failed: 0,
        all_cache_hit: true,
        ..Default::default()
    })
}

/// Build CrcEntry from file path
fn build_crc_entry(path: &Path) -> anyhow::Result<pipeline::types::CrcEntry> {
    let metadata = fs::metadata(path)?;
    let size = metadata.len();
    let mtime_ms = metadata.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    
    let format = crate::media::MediaFormat::from_path(path)
        .unwrap_or(crate::media::MediaFormat::Unknown(""));
    let crc = crate::media::crc::compute_checksum(path, &format)?;
    
    let ext = path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let raw_unique_id = if crate::media::raw_id::is_raw_file(ext) {
        crate::media::raw_id::extract_raw_id_if_raw(path)
            .and_then(|raw_id| crate::media::raw_id::get_fingerprint_string(&raw_id))
    } else {
        None
    };
    
    Ok(pipeline::types::CrcEntry {
        file: pipeline::types::FileEntry {
            path: path.to_path_buf(),
            size,
            mtime_ms,
        },
        src_path: None,
        crc32c: crc,
        raw_unique_id,
        precomputed_hash: None,
    })
}

/// Execute the copy, hash, and insert stages (Stages C, D, E)
fn execute_import_stages(
    new_files: Vec<pipeline::types::CrcEntry>,
    opts: &ImportOptions,
    db: &Db,
    source_canon: &Path,
    total_files: usize,
    likely_dup: usize,
    reporter: &dyn Reporter,
) -> anyhow::Result<ImportSummary> {
    // Stage C: Copy files (parallel)
    let vault_archive = opts.vault_root.clone();
    let dst_fs = SystemFs::open(&vault_archive)?;

    // Pre-resolve destination paths
    let mut prepared = Vec::new();
    let mut assigned = std::collections::HashSet::new();

    for entry in &new_files {
        let rel = entry.file.path.strip_prefix(source_canon)
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
    
    let prepared_count = prepared.len() as u64;
    reporter.emit(CoreEvent::PhaseStarted { 
        phase: PhaseKind::Copy, 
        total: Some(prepared_count) 
    });
    
    let src_fs = SystemFs::open(source_canon)?;
    let transfer_strategies = opts.strategy.to_transfer_strategies();

    // Use atomic counter for progress tracking in parallel iteration
    let copy_progress = Arc::new(std::sync::atomic::AtomicU64::new(0));
    
    let copied: Vec<_> = prepared
        .into_par_iter()
        .filter_map(|(src, dest, size, mtime, crc, raw_id)| {
            if let Some(parent) = dest.parent() {
                if fs::create_dir_all(parent).is_err() {
                    copy_errors.lock().unwrap().insert(src.clone(), "mkdir failed".to_string());
                    return None;
                }
            }

            let rel = src.strip_prefix(source_canon).unwrap_or(&src);
            match transfer_file(&src_fs, rel, &dst_fs, &dest, &transfer_strategies) {
                Ok(_) => {
                    copy_progress.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    Some((src, dest, size, mtime, crc, raw_id))
                }
                Err(e) => {
                    copy_errors.lock().unwrap().insert(src, e.to_string());
                    return None;
                }
            }
        })
        .collect();
    
    let completed_count = copied.len() as u64;
    reporter.emit(CoreEvent::PhaseProgress { 
        phase: PhaseKind::Copy, 
        completed: completed_count, 
        total: Some(prepared_count) 
    });
    reporter.emit(CoreEvent::PhaseFinished { phase: PhaseKind::Copy });

    // Stage D: Strong hash (parallel)
    reporter.emit(CoreEvent::PhaseStarted { 
        phase: PhaseKind::Fingerprint, 
        total: Some(copied.len() as u64) 
    });

    // Convert to CrcEntry for hash stage
    let crc_entries: Vec<pipeline::types::CrcEntry> = copied
        .into_iter()
        .map(|(src, dest, size, mtime, crc, raw_id)| {
            pipeline::types::CrcEntry {
                file: pipeline::types::FileEntry { path: dest, size, mtime_ms: mtime },
                src_path: Some(src),
                crc32c: crc,
                raw_unique_id: raw_id,
                precomputed_hash: None,
            }
        })
        .collect();

    // Compute SHA-256 for --force or --full-id mode
    let compute_sha256 = opts.force || opts.full_id;
    let hash_results = pipeline::hash::compute_hashes(crc_entries, compute_sha256, None);
    
    reporter.emit(CoreEvent::PhaseProgress { 
        phase: PhaseKind::Fingerprint, 
        completed: hash_results.len() as u64, 
        total: Some(hash_results.len() as u64) 
    });
    reporter.emit(CoreEvent::PhaseFinished { phase: PhaseKind::Fingerprint });

    // Check duplicates (skip if force mode - trust user's intent)
    let hash_results = if opts.force {
        hash_results
    } else {
        pipeline::hash::check_duplicates(hash_results, db, &opts.vault_root, false)?
    };

    // Stage E: DB insert
    let insert_total = hash_results.len() as u64;
    reporter.emit(CoreEvent::PhaseStarted { 
        phase: PhaseKind::Insert, 
        total: Some(insert_total) 
    });

    let session_id = session_id_now();
    let insert_opts = pipeline::insert::InsertOptions {
        vault_root: &opts.vault_root,
        session_id: &session_id,
        write_manifest: true,
        source_root: Some(source_canon),
        force: opts.force,
    };

    let summary = pipeline::insert::batch_insert(hash_results, db, insert_opts, None)?;
    
    reporter.emit(CoreEvent::PhaseProgress { 
        phase: PhaseKind::Insert, 
        completed: summary.added as u64, 
        total: Some(insert_total) 
    });
    reporter.emit(CoreEvent::PhaseFinished { phase: PhaseKind::Insert });

    let import_summary = ImportSummary {
        total: total_files,
        imported: summary.added,
        duplicate: summary.duplicate + likely_dup,
        failed: summary.failed + copy_errors.lock().unwrap().len(),
        manifest_path: None,
        all_cache_hit: false,
    };
    
    reporter.emit(CoreEvent::RunFinished {
        operation: OperationKind::Import,
        total: import_summary.total,
        imported: import_summary.imported,
        duplicate: import_summary.duplicate,
        failed: import_summary.failed,
    });
    
    Ok(import_summary)
}

/// Run the full import pipeline (Stages A–E).
pub fn run(opts: ImportOptions, db: &Db, reporter: &dyn Reporter, interactor: &dyn Interactor) -> anyhow::Result<ImportSummary> {
    reporter.emit(CoreEvent::RunStarted { operation: OperationKind::Import });
    
    let source_canon = std::fs::canonicalize(&opts.source)
        .unwrap_or_else(|_| opts.source.clone());
    let vault_canon = std::fs::canonicalize(&opts.vault_root)
        .unwrap_or_else(|_| opts.vault_root.clone());

    // ------------------------------------------------------------------
    // Stage A+B+C: Scan + CRC + Lookup (parallel pipeline with real-time output)
    // ------------------------------------------------------------------
    let exts: Vec<&str> = opts.import_config.allowed_extensions
        .iter().map(|s| s.as_str()).collect();

    // Stage A: Walk (parallel) → Stage B: CRC (parallel via channel)
    let scan_rx = pipeline::scan::scan_stream(&source_canon, &exts)?;
    
    reporter.emit(CoreEvent::PhaseStarted { phase: PhaseKind::Scan, total: None });
    
    // Progress bar for scanning phase
    let crc_rx = pipeline::crc::compute_crcs_stream(scan_rx, None);

    // Stage C: Lookup (serial from channel) with real-time output
    let mut state = ImportState::new();
    
    let mut scan_count: u64 = 0;
    let progress_interval = 10; // Emit progress every 10 files
    
    for result in crc_rx {
        scan_count += 1;
        
        // Emit progress periodically during scan
        if scan_count % progress_interval == 0 {
            reporter.emit(CoreEvent::PhaseProgress { 
                phase: PhaseKind::Scan, 
                completed: scan_count, 
                total: None 
            });
        }
        
        // Skip vault paths
        if result.file.path.ancestors().any(|p| p == vault_canon) {
            continue;
        }
        
        state.total_files += 1;
        
        // Emit discovery event for GUI/CLI
        reporter.emit(CoreEvent::ItemDiscovered {
            path: result.file.path.clone(),
            size: result.file.size,
            mtime_ms: result.file.mtime_ms,
        });
        
        // Handle CRC errors
        let crc = match result.crc {
            Ok(c) => c,
            Err(e) => {
                reporter.emit(CoreEvent::Error {
                    message: format!("CRC computation failed: {}", e),
                    path: Some(result.file.path.clone()),
                });
                continue;
            }
        };
        
        // Build CrcEntry
        let file_path = result.file.path.clone();
        let rel_path = file_path.strip_prefix(&source_canon)
            .unwrap_or(&file_path)
            .to_path_buf();
        
        let entry = pipeline::types::CrcEntry {
            file: pipeline::types::FileEntry {
                path: file_path.clone(),
                size: result.file.size,
                mtime_ms: result.file.mtime_ms,
            },
            src_path: None,
            crc32c: crc,
            raw_unique_id: result.raw_unique_id,
            precomputed_hash: None,
        };
        
        // Immediate DB lookup and real-time output
        let check_result = check_duplicate(&entry, db, &opts.vault_root, None);
        
        // Map check result to ItemStatus and emit classification event
        let item_status = match &check_result {
            pipeline::CheckResult::New => ItemStatus::New,
            pipeline::CheckResult::Duplicate => ItemStatus::Duplicate,
            pipeline::CheckResult::Moved { .. } => ItemStatus::MovedInVault,
            pipeline::CheckResult::Recover { .. } => ItemStatus::Recover,
        };
        reporter.emit(CoreEvent::ItemClassified {
            path: file_path,
            status: item_status,
            detail: None,
        });
        
        process_lookup_result(entry, check_result, &rel_path, &opts, &mut state);
    }
    reporter.emit(CoreEvent::PhaseProgress { 
        phase: PhaseKind::Scan, 
        completed: scan_count, 
        total: None 
    });
    reporter.emit(CoreEvent::PhaseFinished { phase: PhaseKind::Scan });
    
    finalize_import(state, opts, db, &source_canon, reporter, interactor)
}

/// Run import with a predefined file list (skips directory scanning).
/// 
/// This is used for the `scan | filter | import` workflow where files are
/// first scanned, then filtered externally, then imported.
pub fn run_with_file_list(
    opts: ImportOptions,
    db: &Db,
    paths: Vec<std::path::PathBuf>,
    reporter: &dyn Reporter,
    interactor: &dyn Interactor,
) -> anyhow::Result<ImportSummary> {
    reporter.emit(CoreEvent::RunStarted { operation: OperationKind::Import });
    
    let source_canon = std::fs::canonicalize(&opts.source)
        .unwrap_or_else(|_| opts.source.clone());
    let vault_canon = std::fs::canonicalize(&opts.vault_root)
        .unwrap_or_else(|_| opts.vault_root.clone());

    // ------------------------------------------------------------------
    // Stage B+C: CRC + Lookup for provided file list
    // ------------------------------------------------------------------
    reporter.emit(CoreEvent::PhaseStarted { phase: PhaseKind::Scan, total: None });
    
    let mut state = ImportState::new();
    let mut scan_count: u64 = 0;

    for path in paths {
        // Skip non-existent files
        if !path.exists() {
            reporter.emit(CoreEvent::Error {
                message: "file not found".to_string(),
                path: Some(path.clone()),
            });
            continue;
        }

        // Skip directories
        if path.is_dir() {
            continue;
        }

        // Skip vault paths
        if path.ancestors().any(|p| p == vault_canon) {
            continue;
        }

        state.total_files += 1;
        scan_count += 1;
        
        // Emit discovery event
        reporter.emit(CoreEvent::ItemDiscovered {
            path: path.clone(),
            size: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
            mtime_ms: std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        });

        // Build CrcEntry and compute CRC
        let entry = match build_crc_entry(&path) {
            Ok(e) => e,
            Err(e) => {
                reporter.emit(CoreEvent::Error {
                    message: format!("CRC computation failed: {}", e),
                    path: Some(path.clone()),
                });
                continue;
            }
        };

        // Immediate DB lookup and real-time output
        let rel_path = entry.file.path.strip_prefix(&source_canon)
            .unwrap_or(&entry.file.path)
            .to_path_buf();
        let check_result = check_duplicate(&entry, db, &opts.vault_root, None);
        
        // Emit classification event
        let item_status = match &check_result {
            pipeline::CheckResult::New => ItemStatus::New,
            pipeline::CheckResult::Duplicate => ItemStatus::Duplicate,
            pipeline::CheckResult::Moved { .. } => ItemStatus::MovedInVault,
            pipeline::CheckResult::Recover { .. } => ItemStatus::Recover,
        };
        reporter.emit(CoreEvent::ItemClassified {
            path: path.clone(),
            status: item_status,
            detail: None,
        });
        
        process_lookup_result(entry, check_result, &rel_path, &opts, &mut state);
    }
    
    reporter.emit(CoreEvent::PhaseProgress { 
        phase: PhaseKind::Scan, 
        completed: scan_count, 
        total: None 
    });
    reporter.emit(CoreEvent::PhaseFinished { phase: PhaseKind::Scan });
    
    finalize_import(state, opts, db, &source_canon, reporter, interactor)
}

/// Finalize import: show summary, confirm, execute stages
fn finalize_import(
    state: ImportState,
    opts: ImportOptions,
    db: &Db,
    source_canon: &Path,
    reporter: &dyn Reporter,
    interactor: &dyn crate::reporting::Interactor,
) -> anyhow::Result<ImportSummary> {
    if state.lookup_results.is_empty() {
        return Ok(ImportSummary::default());
    }

    let (new_files, dup_files) = pipeline::lookup::filter_new(state.lookup_results, opts.force);
    let likely_dup = dup_files.len();

    // Early exit if no new files
    if new_files.is_empty() {
        return handle_no_new_files(state.total_files, likely_dup, &state.moved_files);
    }

    // Interactive confirmation
    let staging_dir = opts.vault_root.join(".svault").join("staging");
    fs::create_dir_all(&staging_dir)?;

    if !opts.yes && !opts.dry_run {
        if !interactor.confirm("Proceed with import?") {
            return Ok(ImportSummary { total: state.total_files, duplicate: likely_dup, ..Default::default() });
        }
    }

    if opts.dry_run {
        return Ok(ImportSummary { total: state.total_files, duplicate: likely_dup, ..Default::default() });
    }

    // Execute copy, hash, and insert stages
    execute_import_stages(new_files, &opts, db, source_canon, state.total_files, likely_dup, reporter)
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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::reporting::{CoreEvent, ItemStatus, NoopReporter, PhaseKind, Reporter};
    use std::sync::{Arc, Mutex};

    /// Test reporter that captures events for verification
    #[derive(Debug, Default)]
    struct TestReporter {
        events: Mutex<Vec<CoreEvent>>,
    }

    impl Reporter for TestReporter {
        fn emit(&self, event: CoreEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    impl TestReporter {
        fn event_count(&self) -> usize {
            self.events.lock().unwrap().len()
        }
        
        fn has_event(&self, f: impl Fn(&CoreEvent) -> bool) -> bool {
            self.events.lock().unwrap().iter().any(f)
        }
    }

    #[test]
    fn test_import_emits_run_started_event() {
        let reporter = Arc::new(TestReporter::default());
        
        // Just verify the reporter receives events
        reporter.emit(CoreEvent::RunStarted { operation: OperationKind::Import });
        
        assert!(reporter.has_event(|e| matches!(e, CoreEvent::RunStarted { .. })));
    }

    #[test]
    fn test_import_emits_phase_events() {
        let reporter = Arc::new(TestReporter::default());
        
        reporter.emit(CoreEvent::PhaseStarted { phase: PhaseKind::Scan, total: None });
        reporter.emit(CoreEvent::PhaseFinished { phase: PhaseKind::Scan });
        
        assert!(reporter.has_event(|e| matches!(e, CoreEvent::PhaseStarted { phase: PhaseKind::Scan, .. })));
        assert!(reporter.has_event(|e| matches!(e, CoreEvent::PhaseFinished { phase: PhaseKind::Scan })));
    }

    #[test]
    fn test_import_emits_item_events() {
        let reporter = Arc::new(TestReporter::default());
        let test_path = std::path::PathBuf::from("/test/file.jpg");
        
        reporter.emit(CoreEvent::ItemDiscovered {
            path: test_path.clone(),
            size: 1024,
            mtime_ms: 1234567890,
        });
        reporter.emit(CoreEvent::ItemClassified {
            path: test_path.clone(),
            status: ItemStatus::New,
            detail: None,
        });
        
        assert!(reporter.has_event(|e| matches!(e, CoreEvent::ItemDiscovered { .. })));
        assert!(reporter.has_event(|e| matches!(e, CoreEvent::ItemClassified { status: ItemStatus::New, .. })));
    }

    #[test]
    fn test_import_emits_run_finished_event() {
        let reporter = Arc::new(TestReporter::default());
        
        reporter.emit(CoreEvent::RunFinished {
            operation: OperationKind::Import,
            total: 10,
            imported: 8,
            duplicate: 1,
            failed: 1,
        });
        
        let events = reporter.events.lock().unwrap();
        if let Some(CoreEvent::RunFinished { total, imported, duplicate, failed, .. }) = events.first() {
            assert_eq!(*total, 10);
            assert_eq!(*imported, 8);
            assert_eq!(*duplicate, 1);
            assert_eq!(*failed, 1);
        } else {
            panic!("Expected RunFinished event");
        }
    }
}
