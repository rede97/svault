//! Import pipeline (Stages A–E).
//!
//! All pipeline logic lives inside `impl ImportOptions`.  The public entry
//! point is [`ImportOptions::run`]; internal helpers are associated functions
//! (no `self`) so their dependencies are explicit from their signatures.
//!
//! Pipeline overview:
//! ```text
//! run()
//!  ├─ collect_from_scan()   │  Stage A: walk + CRC  (via pipeline::scan / crc)
//!  │   or                   │  Stage B: DB lookup   (check_duplicate)
//!  └─ collect_from_list()   │
//!          │
//!     finalize()            │  Preflight summary, user confirmation
//!          │
//!     stage_copy()          │  Stage C: file transfer
//!     stage_hash()          │  Stage D: strong hash (XXH3 / SHA-256)
//!     stage_insert()        │  Stage E: DB insert + manifest
//! ```

pub mod add;
pub mod exif;
pub mod path;
pub mod recheck;

pub mod staging;
pub mod types;
pub mod update;
pub mod utils;

pub use types::{FileStatus, ImportOptions, ImportSummary, ScanEntry};

// Re-export recheck types for easier access from reporting module
pub use recheck::{RecheckStatus, RecheckResult, RecheckOptions};

use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::config::{HashAlgorithm, ImportConfig, SyncStrategy};
use crate::db::Db;
use crate::fs::transfer_file_with_reporter;
use crate::pipeline;
use crate::reporting::{
    CopyReporter, HashReporter, InsertReporter, Interactor, ItemStatus, ReporterBuilder,
    ScanReporter,
};

use exif::read_exif_date_device;
use path::resolve_dest_path;
use utils::session_id_now;

/// Normalize a path by removing trailing backslashes and quotes.
/// 
/// On Windows, PowerShell may add trailing backslashes when auto-completing paths,
/// which can cause issues when the backslash escapes the closing quote.
fn normalize_path(path: &Path) -> PathBuf {
    let path_str = path.as_os_str().to_string_lossy();
    
    // Repeatedly strip trailing backslashes and quotes
    let mut cleaned = path_str.as_ref();
    loop {
        let new_cleaned = cleaned
            .trim_end_matches('\\')
            .trim_end_matches('/')
            .trim_end_matches('"')
            .trim_end_matches('\'');
        if new_cleaned == cleaned {
            break;
        }
        cleaned = new_cleaned;
    }
    
    PathBuf::from(cleaned)
}

// ─────────────────────────────────────────────────────────────────────────────
// Public helper: duplicate detection (also used by add.rs)
// ─────────────────────────────────────────────────────────────────────────────

/// Check if a file is a duplicate via DB lookup.
///
/// Uses shared `CheckResult` type for consistent handling in import and add
/// commands.
///
/// # Arguments
/// * `entry`      – `CrcEntry` with CRC32C and file metadata
/// * `db`         – Database handle
/// * `vault_root` – Vault root path for existence checks
/// * `hash`       – Optional `(hash_bytes, algorithm)` for secondary
///   verification when CRC matches
///
/// # Special cases
/// - Status `'missing'` → returns `Recover` (allows re-import with path update)
/// - File exists at original path → returns `Duplicate`
/// - CRC matches but file missing → returns `Moved` (vault-internal move)
pub fn check_duplicate(
    entry: &pipeline::types::CrcEntry,
    db: &Db,
    vault_root: &Path,
    hash: Option<(&[u8], &HashAlgorithm)>,
) -> pipeline::CheckResult {
    let ext = entry
        .file
        .path
        .extension()
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

        let hash_matches = if let Some((hash_bytes, algo)) = hash {
            let db_hash = match algo {
                HashAlgorithm::Xxh3_128 => row.xxh3_128.as_ref(),
                HashAlgorithm::Sha256 => row.sha256.as_ref(),
            };
            db_hash.map(|db| db == hash_bytes).unwrap_or(false)
        } else {
            true
        };

        if row.status == "missing" && hash_matches {
            return pipeline::CheckResult::Recover {
                old_path: row.path,
                file_id: row.id,
            };
        }

        let vault_path = vault_root.join(&row.path);
        if vault_path.exists() && is_same_raw_id && hash_matches {
            return pipeline::CheckResult::Duplicate;
        } else if is_same_raw_id && hash_matches {
            return pipeline::CheckResult::Moved { old_path: row.path };
        }
    }

    pipeline::CheckResult::New
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal types
// ─────────────────────────────────────────────────────────────────────────────

/// Accumulated state while streaming through the scan / lookup phase.
struct ImportState {
    lookup_results: Vec<pipeline::types::LookupResult>,
    moved_files: Vec<(PathBuf, String)>,
    total_files: usize,
    failed_files: usize,
}

impl ImportState {
    fn new() -> Self {
        Self {
            lookup_results: Vec::new(),
            moved_files: Vec::new(),
            total_files: 0,
            failed_files: 0,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Free helpers (used internally and/or by sub-modules)
// ─────────────────────────────────────────────────────────────────────────────

fn process_lookup_result(
    entry: pipeline::types::CrcEntry,
    check_result: pipeline::CheckResult,
    state: &mut ImportState,
) {
    match check_result {
        pipeline::CheckResult::Moved { old_path } => {
            state.moved_files.push((entry.file.path.clone(), old_path));
            state.lookup_results.push(pipeline::types::LookupResult {
                entry,
                status: pipeline::types::FileStatus::LikelyCacheDuplicate,
            });
        }
        pipeline::CheckResult::Recover { .. } | pipeline::CheckResult::New => {
            state.lookup_results.push(pipeline::types::LookupResult {
                entry,
                status: pipeline::types::FileStatus::LikelyNew,
            });
        }
        pipeline::CheckResult::Duplicate => {
            state.lookup_results.push(pipeline::types::LookupResult {
                entry,
                status: pipeline::types::FileStatus::LikelyCacheDuplicate,
            });
        }
    }
}

/// Build a `CrcEntry` from a file path (reads metadata + computes CRC32C).
fn build_crc_entry(path: &Path) -> anyhow::Result<pipeline::types::CrcEntry> {
    let metadata = fs::metadata(path)?;
    let size = metadata.len();
    let mtime_ms = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let format = crate::media::MediaFormat::from_path(path)
        .unwrap_or(crate::media::MediaFormat::Unknown(""));
    let crc = crate::media::crc::compute_checksum(path, &format)?;

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
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

/// Classify a file and call the corresponding reporter method.
fn classify_and_emit<SR: ScanReporter>(
    entry: pipeline::types::CrcEntry,
    check_result: pipeline::CheckResult,
    scan_reporter: &SR,
    state: &mut ImportState,
) {
    let item_status = match &check_result {
        pipeline::CheckResult::New => ItemStatus::New,
        pipeline::CheckResult::Duplicate => ItemStatus::Duplicate,
        pipeline::CheckResult::Moved { .. } => ItemStatus::MovedInVault,
        pipeline::CheckResult::Recover { .. } => ItemStatus::Recover,
    };

    // Report the item with all information
    scan_reporter.item(
        &entry.file.path,
        entry.file.size,
        entry.file.mtime_ms,
        item_status,
        None,
    );

    process_lookup_result(entry, check_result, state);
}

/// Resolve a unique destination path that does not conflict with already-
/// assigned destinations or existing files on disk.
fn resolve_unique_dest(
    dest: &Path,
    rename_template: &str,
    assigned: &std::collections::HashSet<PathBuf>,
) -> PathBuf {
    if !dest.exists() && !assigned.contains(dest) {
        return dest.to_path_buf();
    }

    let parent = dest.parent().unwrap_or(Path::new(""));
    let filename = dest
        .file_name()
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

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    parent.join(format!("{}.{}{}", stem, ts, ext))
}

// ─────────────────────────────────────────────────────────────────────────────
// impl ImportOptions — pipeline entry point + stage functions
// ─────────────────────────────────────────────────────────────────────────────

impl ImportOptions {
    /// Run the full import pipeline.
    ///
    /// Branches on `self.files_from`:
    /// - `None`       → scan `self.source` recursively (Stage A + B)
    /// - `Some(list)` → use the pre-parsed path list (Stage B only)
    pub fn run_import<RB: ReporterBuilder, I: Interactor>(
        self,
        db: &Db,
        reporter_builder: &RB,
        interactor: &I,
    ) -> anyhow::Result<ImportSummary> {
        let source_canon =
            dunce::canonicalize(&self.source).unwrap_or_else(|_| self.source.clone());
        
        // Normalize path: remove trailing backslashes and quotes (PowerShell quirk)
        let source_canon = normalize_path(&source_canon);

        // Use normalized source for reporter so path relativization works correctly
        let scan_reporter = reporter_builder.scan_reporter(&source_canon);

        let state = match self.files_from {
            Some(ref paths) => ImportOptions::collect_from_list(
                paths,
                &self.vault_root,
                self.show_dup,
                Some(db),
                &scan_reporter,
            )?,
            None => ImportOptions::collect_from_scan(
                &source_canon,
                &self.vault_root,
                &self.import_config.allowed_extensions,
                self.show_dup,
                Some(db),
                &scan_reporter,
            )?,
        };

        self.finalize(
            state,
            source_canon,
            scan_reporter,
            db,
            reporter_builder,
            interactor,
        )
    }

    // ── Scan-phase collectors ─────────────────────────────────────────────────

    /// Stage A + B: walk source directory, compute CRC32C, look up DB.
    ///
    /// `db` may be `None` when no vault is open (e.g. bare scan); in that
    /// case every file is classified as `New` without a duplicate check.
    fn collect_from_scan<SR: ScanReporter>(
        source_canon: &Path,
        vault_root: &Path,
        allowed_extensions: &[String],
        _show_dup: bool,
        db: Option<&Db>,
        scan_reporter: &SR,
    ) -> anyhow::Result<ImportState> {
        let vault_canon =
            dunce::canonicalize(vault_root).unwrap_or_else(|_| vault_root.to_path_buf());
        let exts: Vec<&str> = allowed_extensions.iter().map(|s| s.as_str()).collect();

        let scan_rx = pipeline::scan::scan_stream(source_canon, &exts)?;
        let crc_rx = pipeline::crc::compute_crcs_stream(scan_rx);

        let mut state = ImportState::new();

        for result in crc_rx {
            // Skip vault sub-tree
            if result.file.path.ancestors().any(|p| p == vault_canon) {
                continue;
            }

            state.total_files += 1;

            let crc = match result.crc {
                Ok(c) => c,
                Err(e) => {
                    // Report failed item
                    scan_reporter.item(
                        &result.file.path,
                        result.file.size,
                        result.file.mtime_ms,
                        ItemStatus::Failed,
                        Some(&format!("CRC computation failed: {}", e)),
                    );
                    state.failed_files += 1;
                    continue;
                }
            };

            let entry = pipeline::types::CrcEntry {
                file: pipeline::types::FileEntry {
                    path: result.file.path.clone(),
                    size: result.file.size,
                    mtime_ms: result.file.mtime_ms,
                },
                src_path: None,
                crc32c: crc,
                raw_unique_id: result.raw_unique_id,
                precomputed_hash: None,
            };

            let check_result = match db {
                Some(db) => check_duplicate(&entry, db, vault_root, None),
                None => pipeline::CheckResult::New,
            };
            classify_and_emit(entry, check_result, scan_reporter, &mut state);
        }

        Ok(state)
    }

    /// Stage B: process a pre-provided file list, compute CRC32C, look up DB.
    ///
    /// `db` may be `None`; in that case every file is classified as `New`.
    fn collect_from_list<SR: ScanReporter>(
        paths: &[PathBuf],
        vault_root: &Path,
        _show_dup: bool,
        db: Option<&Db>,
        scan_reporter: &SR,
    ) -> anyhow::Result<ImportState> {
        let vault_canon =
            dunce::canonicalize(vault_root).unwrap_or_else(|_| vault_root.to_path_buf());

        let mut state = ImportState::new();

        for path in paths {
            if !path.exists() {
                scan_reporter.item(path, 0, 0, ItemStatus::Failed, Some("file not found"));
                continue;
            }
            if path.is_dir() {
                continue;
            }
            if path.ancestors().any(|p| p == vault_canon) {
                continue;
            }

            state.total_files += 1;

            let meta = std::fs::metadata(path);
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime_ms = meta
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);

            let entry = match build_crc_entry(path) {
                Ok(e) => e,
                Err(e) => {
                    scan_reporter.item(path, size, mtime_ms, ItemStatus::Failed, Some(&format!("CRC computation failed: {}", e)));
                    state.failed_files += 1;
                    continue;
                }
            };

            let check_result = match db {
                Some(db) => check_duplicate(&entry, db, vault_root, None),
                None => pipeline::CheckResult::New,
            };
            classify_and_emit(entry, check_result, scan_reporter, &mut state);
        }

        Ok(state)
    }

    /// Scan-only entry point: runs Stage A + B without copying, hashing, or
    /// inserting anything.
    ///
    /// Reuses [`ImportOptions::collect_from_scan`] so the scan logic is never
    /// duplicated.  `db` is optional — pass `None` when no vault is open.
    ///
    /// All per-file output is handled by the reporter (`PipeReporter` on the
    /// CLI side).  Returns `Err` if any files failed to scan so the caller can
    /// propagate a non-zero exit code.
    pub fn run_scan<RB: ReporterBuilder>(
        self,
        db: Option<&Db>,
        reporter_builder: &RB,
    ) -> anyhow::Result<()> {
        let source_canon =
            dunce::canonicalize(&self.source).unwrap_or_else(|_| self.source.clone());

        let scan_reporter = reporter_builder.scan_reporter(&self.source);

        let state = ImportOptions::collect_from_scan(
            &source_canon,
            &self.vault_root,
            &self.import_config.allowed_extensions,
            self.show_dup,
            db,
            &scan_reporter,
        )?;

        scan_reporter.finish();
        drop(scan_reporter); // clear progress bar before any further output

        if state.failed_files > 0 {
            anyhow::bail!(
                "{} file(s) could not be scanned (see fail: lines above)",
                state.failed_files
            );
        }

        Ok(())
    }

    // ── Finalisation ──────────────────────────────────────────────────────────

    /// Emit pre-flight summary, confirm with the user, then run Copy/Hash/Insert.
    ///
    /// Takes ownership of `scan_reporter` so it can be dropped (and the progress
    /// bar cleared) before the confirmation prompt is shown.
    fn finalize<RB: ReporterBuilder, I: Interactor>(
        self,
        state: ImportState,
        source_canon: PathBuf,
        scan_reporter: RB::Scan,
        db: &Db,
        reporter_builder: &RB,
        interactor: &I,
    ) -> anyhow::Result<ImportSummary> {
        if state.lookup_results.is_empty() {
            return Ok(ImportSummary::default());
        }

        let (new_files, dup_files) = pipeline::lookup::filter_new(state.lookup_results, self.force);
        let likely_dup = dup_files.len();

        scan_reporter.preflight(
            state.total_files,
            new_files.len(),
            likely_dup,
            state.moved_files.len(),
            state.failed_files,
            &source_canon,
        );
        scan_reporter.finish();
        drop(scan_reporter); // clear progress bar before prompt

        // Nothing to import - all files were duplicates, moved, or failed
        if new_files.is_empty() {
            return Ok(ImportSummary {
                total: state.total_files,
                duplicate: likely_dup,
                failed: 0,
                all_cache_hit: true,
                ..Default::default()
            });
        }

        if !self.yes && !self.dry_run && !interactor.confirm("Proceed with import?") {
            return Ok(ImportSummary {
                total: state.total_files,
                duplicate: likely_dup,
                ..Default::default()
            });
        }

        if self.dry_run {
            return Ok(ImportSummary {
                total: state.total_files,
                duplicate: likely_dup,
                ..Default::default()
            });
        }

        // ── Stage C ───────────────────────────────────────────────────────────
        let (copied, copy_error_count) = ImportOptions::stage_copy(
            new_files,
            &source_canon,
            &self.vault_root,
            &self.strategy,
            &self.import_config,
            reporter_builder,
        );

        // ── Stage D ───────────────────────────────────────────────────────────
        let hash_results = ImportOptions::stage_hash(
            copied,
            &source_canon,
            &self.vault_root,
            self.force,
            self.full_id,
            db,
            reporter_builder,
        )?;

        // ── Stage E ───────────────────────────────────────────────────────────
        let import_summary = ImportOptions::stage_insert(
            hash_results,
            &self.vault_root,
            &source_canon,
            self.force,
            db,
            state.total_files,
            likely_dup,
            copy_error_count,
            reporter_builder,
        )?;

        Ok(import_summary)
    }

    // ── Stage functions (associated, no self) ─────────────────────────────────

    /// Stage C: copy files from source to vault.
    ///
    /// Returns the successfully copied entries (as `CrcEntry` with `src_path`
    /// set) and the number of copy errors.
    fn stage_copy<RB: ReporterBuilder>(
        new_files: Vec<pipeline::types::CrcEntry>,
        source_canon: &Path,
        vault_root: &Path,
        strategy: &SyncStrategy,
        import_config: &ImportConfig,
        reporter_builder: &RB,
    ) -> (Vec<pipeline::types::CrcEntry>, usize) {
        // Resolve destination paths up-front (serial, EXIF-aware)
        let mut prepared: Vec<(PathBuf, PathBuf, u64, i64, u32, Option<String>)> = Vec::new();
        let mut assigned = std::collections::HashSet::new();

        for entry in &new_files {
            let rel = entry
                .file
                .path
                .strip_prefix(source_canon)
                .unwrap_or(&entry.file.path);
            let (taken_ms, device) = read_exif_date_device(&entry.file.path, entry.file.mtime_ms);
            let dest_rel = resolve_dest_path(&import_config.path_template, rel, taken_ms, &device);
            let dest_abs = vault_root.join(&dest_rel);
            let unique_dest =
                resolve_unique_dest(&dest_abs, &import_config.rename_template, &assigned);
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

        let total = prepared.len() as u64;
        let transfer_strategies = strategy.to_transfer_strategies();

        let copied = {
            let reporter = reporter_builder.copy_reporter(source_canon, vault_root, total);

            let result: Vec<pipeline::types::CrcEntry> = prepared
                .into_par_iter()
                .filter_map(|(src, dest, size, mtime, crc, raw_id)| {
                    // Create parent directory
                    if let Some(parent) = dest.parent()
                        && let Err(e) = fs::create_dir_all(parent)
                    {
                        let msg = e.to_string();
                        reporter.item_started(&src, &dest, size);
                        reporter.item_finished(&src, &dest, &crate::reporting::CopyItemResult::Failed { message: msg });
                        return None;
                    }

                    let src_rel = src.strip_prefix(source_canon).unwrap_or(&src);
                    match transfer_file_with_reporter(
                        source_canon, src_rel, vault_root, &dest, 
                        &transfer_strategies, Some(&reporter)
                    ) {
                        Ok(_) => {
                            Some(pipeline::types::CrcEntry {
                                file: pipeline::types::FileEntry {
                                    path: dest,
                                    size,
                                    mtime_ms: mtime,
                                },
                                src_path: Some(src),
                                crc32c: crc,
                                raw_unique_id: raw_id,
                                precomputed_hash: None,
                            })
                        }
                        Err(_) => None,
                    }
                })
                .collect();

            reporter.finish();
            result
        };

        let error_count = total as usize - copied.len();
        (copied, error_count)
    }

    /// Stage D: compute strong hashes (XXH3-128, optionally SHA-256).
    ///
    /// Also performs a post-hash dedup check unless `force` is set.
    fn stage_hash<RB: ReporterBuilder>(
        copied: Vec<pipeline::types::CrcEntry>,
        source_canon: &Path,
        vault_root: &Path,
        force: bool,
        full_id: bool,
        db: &Db,
        reporter_builder: &RB,
    ) -> anyhow::Result<Vec<pipeline::types::HashResult>> {
        let total = copied.len() as u64;
        let hash_results = {
            let reporter = reporter_builder.hash_reporter(source_canon, total);

            let results =
                pipeline::hash::compute_hashes(copied, force || full_id, Some(&reporter));

            reporter.finish();
            results
        }; // reporter dropped → bar cleared

        if force {
            Ok(hash_results)
        } else {
            Ok(pipeline::hash::check_duplicates(
                hash_results,
                db,
                vault_root,
                false,
            )?)
        }
    }

    /// Stage E: batch-insert records into the DB and write the import manifest.
    ///
    /// Also emits the final import summary via [`InsertReporter::summary`].
    #[allow(clippy::too_many_arguments)]
    fn stage_insert<RB: ReporterBuilder>(
        hash_results: Vec<pipeline::types::HashResult>,
        vault_root: &Path,
        source_root: &Path,
        force: bool,
        db: &Db,
        total_files: usize,
        likely_dup: usize,
        copy_error_count: usize,
        reporter_builder: &RB,
    ) -> anyhow::Result<ImportSummary> {
        let insert_count = hash_results.len() as u64;
        let session_id = session_id_now();

        let pipeline_summary = {
            let reporter = reporter_builder.insert_reporter(source_root, insert_count);
            let progress = std::sync::atomic::AtomicU64::new(0);
            let progress_cb = || {
                let done = progress.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                reporter.progress(done, insert_count);
            };

            let insert_opts = pipeline::insert::InsertOptions {
                vault_root,
                session_id: &session_id,
                write_manifest: true,
                source_root: Some(source_root),
                force,
            };

            let result =
                pipeline::insert::batch_insert(hash_results, db, insert_opts, Some(&progress_cb))?;

            let done = progress.load(std::sync::atomic::Ordering::Relaxed);
            if done < insert_count {
                reporter.progress(done, insert_count);
            }
            reporter.finish();

            let import_summary = ImportSummary {
                total: total_files,
                imported: result.added,
                duplicate: result.duplicate + likely_dup,
                failed: result.failed + copy_error_count,
                manifest_path: result.manifest_path.clone(),
                all_cache_hit: false,
            };

            reporter.summary(
                import_summary.total,
                import_summary.imported,
                import_summary.duplicate,
                import_summary.failed,
                import_summary.manifest_path.as_deref(),
            );

            import_summary
        }; // reporter dropped → bar cleared

        Ok(pipeline_summary)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reporting::{ItemStatus, Noop, ReporterBuilder, ScanReporter};
    use std::sync::{Arc, Mutex};

    // ── Test scan reporter ────────────────────────────────────────────────────

    #[derive(Debug, Default)]
    struct ScanLog {
        classified: Vec<(PathBuf, ItemStatus)>,
        finished: bool,
    }

    #[derive(Debug, Clone, Default)]
    struct TestScanReporter(Arc<Mutex<ScanLog>>);

    impl ScanReporter for TestScanReporter {
        fn item(&self, path: &Path, _size: u64, _mtime_ms: i64, status: ItemStatus, _error: Option<&str>) {
            self.0
                .lock()
                .unwrap()
                .classified
                .push((path.to_path_buf(), status));
        }
        fn preflight(
            &self,
            _total_scanned: usize,
            _new_count: usize,
            _duplicate_count: usize,
            _moved_count: usize,
            _failed_count: usize,
            _source: &Path,
        ) {
        }
        fn finish(&self) {
            self.0.lock().unwrap().finished = true;
        }
    }

    impl Drop for TestScanReporter {
        fn drop(&mut self) {}
    }

    // ── Test reporter builder ─────────────────────────────────────────────────

    struct TestReporterBuilder {
        log: Arc<Mutex<ScanLog>>,
    }

    impl ReporterBuilder for TestReporterBuilder {
        type Scan = TestScanReporter;
        type Copy = Noop;
        type Hash = Noop;
        type Insert = Noop;
        type AddSummary = Noop;
        type Recheck = Noop;
        type UpdateApply = Noop;
        type Verify = Noop;

        fn scan_reporter(&self, _source: &Path) -> TestScanReporter {
            TestScanReporter(Arc::clone(&self.log))
        }
        fn copy_reporter(&self, _source: &Path, _vault_root: &Path, _total: u64) -> Noop {
            Noop
        }
        fn hash_reporter(&self, _source: &Path, _total: u64) -> Noop {
            Noop
        }
        fn insert_reporter(&self, _source: &Path, _total: u64) -> Noop {
            Noop
        }
        fn add_summary_reporter(&self, _vault_root: &Path) -> Noop {
            Noop
        }
        fn recheck_reporter(&self, _total: u64) -> Noop {
            Noop
        }
        fn update_hash_reporter(&self, _source: &Path, _total: u64) -> Noop {
            Noop
        }
        fn update_apply_reporter(&self, _total: u64) -> Noop {
            Noop
        }
        fn verify_reporter(&self, _total: u64) -> Noop {
            Noop
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_scan_reporter_records_items() {
        let log = Arc::new(Mutex::new(ScanLog::default()));
        let rb = TestReporterBuilder {
            log: Arc::clone(&log),
        };
        let reporter = rb.scan_reporter(Path::new("/source"));
        reporter.item(Path::new("/source/photo.jpg"), 1024, 0, ItemStatus::New, None);
        reporter.finish();

        let log = log.lock().unwrap();
        assert_eq!(log.classified.len(), 1);
        assert_eq!(log.classified[0].1, ItemStatus::New);
        assert!(log.finished);
    }

    #[test]
    fn test_noop_reporter_builder_compiles() {
        use crate::reporting::NoopReporterBuilder;
        let rb = NoopReporterBuilder;
        let sr = rb.scan_reporter(Path::new("/"));
        sr.item(Path::new("/test.jpg"), 1024, 0, ItemStatus::New, None);
        ScanReporter::finish(&sr);
        // No assertions needed — just verify it compiles and runs without panic.
    }
}
