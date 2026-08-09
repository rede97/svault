//! `svault import` — import media files from a source directory.
//!
//! Pipeline overview:
//! ```text
//! run_import()
//!  ├─ collect_from_scan()   │  Stage A: walk + CRC  (via pipeline::scan / crc)
//!  │   or                   │  Stage B: DB lookup   (check_duplicate)
//!  └─ collect_from_list()   │
//!          │
//!     finalize()            │  Preflight event, user confirmation
//!          │
//!     stage_copy()          │  Stage C: file transfer
//!     stage_hash()          │  Stage D: strong hash (XXH3 / SHA-256)
//!     stage_insert()        │  Stage E: DB insert + manifest
//! ```
//!
//! All user-facing output is emitted as [`Event`]s; see [`crate::event`].

use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::config::{ImportConfig, SyncStrategy};
use crate::db::Db;
use crate::event::{Event, EventSink, Hint, Interactor, ItemStatus, Phase, PhaseContext, Summary};
use crate::fs::transfer_file;
use crate::ops::check_duplicate_with_level;
use crate::ops::exif::read_exif_date_device;
use crate::ops::path::resolve_dest_path;
use crate::ops::types::{ImportOptions, ImportSummary};
use crate::ops::utils::session_id_now;
use crate::pipeline;

/// Normalize a path by removing trailing backslashes and quotes.
///
/// On Windows, PowerShell may add trailing backslashes when auto-completing
/// paths, which can cause issues when the backslash escapes the closing quote.
///
/// SAFETY: Preserves root paths (e.g. `/`, `C:\`) — never returns an empty
/// string or a bare drive letter.
pub fn normalize_path(path: &Path) -> PathBuf {
    let path_str = path.as_os_str().to_string_lossy();

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

    // Restore Unix root `/` (was stripped to empty string).
    if cleaned.is_empty() {
        return PathBuf::from("/");
    }

    // Restore Windows drive root `C:\` (was stripped to `C:`).
    #[cfg(windows)]
    {
        let bytes = cleaned.as_bytes();
        if cleaned.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return PathBuf::from(format!("{}\\", cleaned));
        }
    }

    PathBuf::from(cleaned)
}

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
        staged_path: None,
        crc32c: crc,
        raw_unique_id,
        precomputed_hash: None,
    })
}

/// Classify a file, emit a [`Event::ScanItem`], and update state.
fn classify_and_emit(
    entry: pipeline::types::CrcEntry,
    check_result: pipeline::CheckResult,
    sink: &dyn EventSink,
    state: &mut ImportState,
) {
    let item_status = match &check_result {
        pipeline::CheckResult::New => ItemStatus::New,
        pipeline::CheckResult::Duplicate => ItemStatus::Duplicate,
        pipeline::CheckResult::Moved { .. } => ItemStatus::MovedInVault,
        pipeline::CheckResult::Recover { .. } => ItemStatus::Recover,
    };

    sink.emit(&Event::ScanItem {
        path: entry.file.path.clone(),
        size: entry.file.size,
        mtime_ms: entry.file.mtime_ms,
        status: item_status,
        error: None,
    });

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

/// A file queued for Stage C: source, final destination, staging path, and
/// the metadata carried through to the later pipeline stages.
struct PreparedCopy {
    src: PathBuf,
    dest: PathBuf,
    staged: PathBuf,
    size: u64,
    mtime_ms: i64,
    crc32c: u32,
    raw_unique_id: Option<String>,
}

/// Sink wrapper used during Stage C: rewrites staging paths to the final
/// destination in copy events, so the UI shows where each file will end up
/// rather than the transient session staging location.
struct StagingSink<'a> {
    inner: &'a dyn EventSink,
    staging_dir: &'a Path,
    vault_root: &'a Path,
}

impl StagingSink<'_> {
    fn unstage(&self, path: &Path) -> PathBuf {
        path.strip_prefix(self.staging_dir)
            .map(|rel| self.vault_root.join(rel))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

impl EventSink for StagingSink<'_> {
    fn emit(&self, event: &Event) {
        match event {
            Event::CopyStarted { src, dst, bytes } => self.inner.emit(&Event::CopyStarted {
                src: src.clone(),
                dst: self.unstage(dst),
                bytes: *bytes,
            }),
            Event::CopyFinished { src, dst, error } => self.inner.emit(&Event::CopyFinished {
                src: src.clone(),
                dst: self.unstage(dst),
                error: error.clone(),
            }),
            other => self.inner.emit(other),
        }
    }
}

impl ImportOptions {
    /// Run the full import pipeline.
    ///
    /// Branches on `self.files_from`:
    /// - `None`       → scan `self.source` recursively (Stage A + B)
    /// - `Some(list)` → use the pre-parsed path list (Stage B only)
    pub fn run_import(
        self,
        db: &Db,
        sink: &dyn EventSink,
        interactor: &dyn Interactor,
    ) -> anyhow::Result<ImportSummary> {
        let source_canon =
            dunce::canonicalize(&self.source).unwrap_or_else(|_| self.source.clone());
        let source_canon = normalize_path(&source_canon);

        // Finish or report session leftovers from an interrupted import
        // before scanning, so freed paths are available to this run.
        crate::session::reconcile(&self.vault_root, db, sink);

        sink.emit(&Event::PhaseStarted {
            phase: Phase::Scan,
            total: None,
            context: PhaseContext::source(source_canon.clone()),
        });

        let state = match self.files_from {
            Some(ref paths) => Self::collect_from_list(
                paths,
                &self.vault_root,
                self.compare_level,
                Some(db),
                sink,
            )?,
            None => Self::collect_from_scan(
                &source_canon,
                &self.vault_root,
                &self.import_config.allowed_extensions,
                &crate::fs::ScanFilter {
                    max_depth: self.max_depth,
                    include: self.include.clone(),
                    exclude: self.exclude.clone(),
                },
                self.compare_level,
                Some(db),
                sink,
            )?,
        };

        self.finalize(state, source_canon, db, sink, interactor)
    }

    // ── Scan-phase collectors ─────────────────────────────────────────────────

    /// Stage A + B: walk source directory, compute CRC32C, look up DB.
    ///
    /// `db` may be `None` when no vault is open (e.g. bare scan); in that
    /// case every file is classified as `New` without a duplicate check.
    fn collect_from_scan(
        source_canon: &Path,
        vault_root: &Path,
        allowed_extensions: &[String],
        filter: &crate::fs::ScanFilter,
        compare_level: crate::ops::types::CompareLevel,
        db: Option<&Db>,
        sink: &dyn EventSink,
    ) -> anyhow::Result<ImportState> {
        let vault_canon =
            dunce::canonicalize(vault_root).unwrap_or_else(|_| vault_root.to_path_buf());
        let exts: Vec<&str> = allowed_extensions.iter().map(|s| s.as_str()).collect();

        let scan_rx = pipeline::scan::scan_stream(source_canon, &exts, filter)?;
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
                    sink.emit(&Event::ScanItem {
                        path: result.file.path.clone(),
                        size: result.file.size,
                        mtime_ms: result.file.mtime_ms,
                        status: ItemStatus::Failed,
                        error: Some(format!("CRC computation failed: {}", e)),
                    });
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
                staged_path: None,
                crc32c: crc,
                raw_unique_id: result.raw_unique_id,
                precomputed_hash: None,
            };

            let check_result = match db {
                Some(db) => check_duplicate_with_level(&entry, db, vault_root, compare_level),
                None => pipeline::CheckResult::New,
            };
            classify_and_emit(entry, check_result, sink, &mut state);
        }

        Ok(state)
    }

    /// Stage B: process a pre-provided file list, compute CRC32C, look up DB.
    ///
    /// `db` may be `None`; in that case every file is classified as `New`.
    fn collect_from_list(
        paths: &[PathBuf],
        vault_root: &Path,
        compare_level: crate::ops::types::CompareLevel,
        db: Option<&Db>,
        sink: &dyn EventSink,
    ) -> anyhow::Result<ImportState> {
        let vault_canon =
            dunce::canonicalize(vault_root).unwrap_or_else(|_| vault_root.to_path_buf());

        let mut state = ImportState::new();

        for path in paths {
            if !path.exists() {
                sink.emit(&Event::ScanItem {
                    path: path.clone(),
                    size: 0,
                    mtime_ms: 0,
                    status: ItemStatus::Failed,
                    error: Some("file not found".to_string()),
                });
                continue;
            }
            if path.is_dir() {
                continue;
            }
            if path.ancestors().any(|p| p == vault_canon) {
                continue;
            }

            state.total_files += 1;

            let entry = match build_crc_entry(path) {
                Ok(e) => e,
                Err(e) => {
                    sink.emit(&Event::ScanItem {
                        path: path.clone(),
                        size: 0,
                        mtime_ms: 0,
                        status: ItemStatus::Failed,
                        error: Some(format!("CRC computation failed: {}", e)),
                    });
                    state.failed_files += 1;
                    continue;
                }
            };

            let check_result = match db {
                Some(db) => check_duplicate_with_level(&entry, db, vault_root, compare_level),
                None => pipeline::CheckResult::New,
            };
            classify_and_emit(entry, check_result, sink, &mut state);
        }

        Ok(state)
    }

    /// Scan-only entry point: runs Stage A + B without copying, hashing, or
    /// inserting anything.
    ///
    /// Reuses [`ImportOptions::collect_from_scan`] so the scan logic is never
    /// duplicated. `db` is optional — pass `None` when no vault is open.
    ///
    /// Returns `Err` if any files failed to scan so the caller can propagate
    /// a non-zero exit code.
    pub fn run_scan(self, db: Option<&Db>, sink: &dyn EventSink) -> anyhow::Result<()> {
        let source_canon =
            dunce::canonicalize(&self.source).unwrap_or_else(|_| self.source.clone());
        let source_canon = normalize_path(&source_canon);

        sink.emit(&Event::PhaseStarted {
            phase: Phase::Scan,
            total: None,
            context: PhaseContext::source(source_canon.clone()),
        });

        let state = Self::collect_from_scan(
            &source_canon,
            &self.vault_root,
            &self.import_config.allowed_extensions,
            &crate::fs::ScanFilter {
                max_depth: self.max_depth,
                include: self.include.clone(),
                exclude: self.exclude.clone(),
            },
            self.compare_level,
            db,
            sink,
        )?;

        sink.emit(&Event::PhaseFinished { phase: Phase::Scan });

        if state.failed_files > 0 {
            anyhow::bail!("{} file(s) could not be scanned", state.failed_files);
        }

        Ok(())
    }

    // ── Finalisation ──────────────────────────────────────────────────────────

    /// Emit pre-flight summary, confirm with the user, then run Copy/Hash/Insert.
    fn finalize(
        self,
        state: ImportState,
        source_canon: PathBuf,
        db: &Db,
        sink: &dyn EventSink,
        interactor: &dyn Interactor,
    ) -> anyhow::Result<ImportSummary> {
        if state.lookup_results.is_empty() {
            sink.emit(&Event::PhaseFinished { phase: Phase::Scan });
            let summary = ImportSummary::default();
            sink.emit(&Event::Summary(Summary::Import(summary.clone())));
            return Ok(summary);
        }

        let (new_files, dup_files) = pipeline::lookup::filter_new(state.lookup_results, self.force);
        let likely_dup = dup_files.len();

        sink.emit(&Event::Preflight {
            source: source_canon.clone(),
            total: state.total_files,
            new: new_files.len(),
            duplicate: likely_dup,
            moved: state.moved_files.len(),
            failed: state.failed_files,
        });
        sink.emit(&Event::PhaseFinished { phase: Phase::Scan });

        // Nothing to import — all files were duplicates, moved, or failed
        if new_files.is_empty() {
            let summary = ImportSummary {
                total: state.total_files,
                duplicate: likely_dup,
                failed: 0,
                all_cache_hit: true,
                ..Default::default()
            };
            sink.emit(&Event::Summary(Summary::Import(summary.clone())));
            return Ok(summary);
        }

        if !self.yes && !self.dry_run && !interactor.confirm("Proceed with import?") {
            let summary = ImportSummary {
                total: state.total_files,
                duplicate: likely_dup,
                ..Default::default()
            };
            sink.emit(&Event::Summary(Summary::Import(summary.clone())));
            return Ok(summary);
        }

        if self.dry_run {
            let summary = ImportSummary {
                total: state.total_files,
                duplicate: likely_dup,
                ..Default::default()
            };
            sink.emit(&Event::Summary(Summary::Import(summary.clone())));
            return Ok(summary);
        }

        // ── Stage C ───────────────────────────────────────────────────────────
        // One session journal per import run: files are copied into
        // `.svault/sessions/import/<ts-id>/staging/` and only renamed to
        // their final destination after the Stage-E DB transaction commits.
        let session_id = session_id_now();
        let session_dir = crate::session::session_dir(
            &self.vault_root,
            crate::verify::manifest::SessionType::Import,
            &session_id,
        );

        let (copied, copy_error_count) = Self::stage_copy(
            new_files,
            &source_canon,
            &self.vault_root,
            &session_id,
            &session_dir,
            &self.strategy,
            &self.import_config,
            sink,
        )?;

        // ── Stage D ───────────────────────────────────────────────────────────
        let hash_results = Self::stage_hash(
            copied,
            &source_canon,
            &self.vault_root,
            self.force,
            self.full_id,
            db,
            sink,
        )?;

        // ── Stage E ───────────────────────────────────────────────────────────
        let import_summary = Self::stage_insert(
            hash_results,
            &self.vault_root,
            &source_canon,
            &session_id,
            &session_dir,
            self.force,
            db,
            state.total_files,
            likely_dup,
            copy_error_count,
            sink,
        )?;

        Ok(import_summary)
    }

    // ── Stage functions (associated, no self) ─────────────────────────────────

    /// Stage C: copy files from source into the session staging area.
    ///
    /// Writes `plan.json` (pre-copy intent) atomically first — a plan write
    /// failure aborts the import before any byte is transferred. Each file
    /// is then transferred to `<session>/staging/` (mirroring its final
    /// relative path) and fsynced; it is only renamed to the final
    /// destination after the Stage-E DB commit, so an interrupted copy never
    /// pollutes the user-visible vault tree.
    ///
    /// Returns the successfully copied entries (as `CrcEntry` with `src_path`
    /// and `staged_path` set) and the number of copy errors.
    #[allow(clippy::too_many_arguments)]
    fn stage_copy(
        new_files: Vec<pipeline::types::CrcEntry>,
        source_canon: &Path,
        vault_root: &Path,
        session_id: &str,
        session_dir: &Path,
        strategy: &SyncStrategy,
        import_config: &ImportConfig,
        sink: &dyn EventSink,
    ) -> anyhow::Result<(Vec<pipeline::types::CrcEntry>, usize)> {
        // Resolve destination paths up-front (serial, EXIF-aware)
        let mut prepared: Vec<PreparedCopy> = Vec::new();
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
            let staged = crate::session::staged_path_for(session_dir, vault_root, &unique_dest);

            prepared.push(PreparedCopy {
                src: entry.file.path.clone(),
                dest: unique_dest,
                staged,
                size: entry.file.size,
                mtime_ms: entry.file.mtime_ms,
                crc32c: entry.crc32c,
                raw_unique_id: entry.raw_unique_id.clone(),
            });
        }

        // Persist the pre-copy intent BEFORE transferring anything. The plan
        // makes an interrupted session self-describing (what came from where,
        // bound for which destination). Fail-fast: a plan that cannot be
        // written signals disk trouble that would only get worse mid-copy.
        let plan = crate::session::ImportPlan {
            session_id: session_id.to_string(),
            session_type: crate::verify::manifest::SessionType::Import,
            source_root: source_canon.to_path_buf(),
            created_at: crate::ops::utils::unix_now_ms(),
            files: prepared
                .iter()
                .map(|p| crate::session::PlanEntry {
                    src_path: p.src.clone(),
                    dest_path: p
                        .dest
                        .strip_prefix(vault_root)
                        .unwrap_or(&p.dest)
                        .to_string_lossy()
                        .replace('\\', "/"),
                    size: p.size,
                    crc32c: p.crc32c,
                })
                .collect(),
        };
        crate::session::write_json_atomic(&session_dir.join(crate::session::PLAN_FILE), &plan)
            .map_err(|e| anyhow::anyhow!("cannot write import plan: {e}"))?;

        let total = prepared.len() as u64;
        let transfer_strategies = strategy.to_transfer_strategies();
        let staging_dir = crate::session::staging_dir(session_dir);
        let staging_sink = StagingSink {
            inner: sink,
            staging_dir: &staging_dir,
            vault_root,
        };

        sink.emit(&Event::PhaseStarted {
            phase: Phase::Copy,
            total: Some(total),
            context: PhaseContext::both(source_canon.to_path_buf(), vault_root.to_path_buf()),
        });

        let copied: Vec<pipeline::types::CrcEntry> = prepared
            .into_par_iter()
            .filter_map(|item| {
                let PreparedCopy {
                    src,
                    dest,
                    staged,
                    size,
                    mtime_ms,
                    crc32c,
                    raw_unique_id,
                } = item;
                let src_rel = src.strip_prefix(source_canon).unwrap_or(&src);
                let transferred = transfer_file(
                    source_canon,
                    src_rel,
                    vault_root,
                    &staged,
                    &transfer_strategies,
                    Some(&staging_sink),
                );
                // Fsync the staged copy so the Stage-D hash read is
                // guaranteed to match durable storage even on power loss.
                let ok = match transferred {
                    Ok(()) => match crate::fs::sync_file_and_dir(&staged) {
                        Ok(()) => true,
                        Err(e) => {
                            // transfer_file already reported success; surface
                            // the fsync failure explicitly.
                            sink.emit(&Event::CopyFinished {
                                src: src.clone(),
                                dst: dest.clone(),
                                error: Some(e.to_string()),
                            });
                            false
                        }
                    },
                    // transfer_file already emitted CopyFinished with the error.
                    Err(_) => false,
                };

                if !ok {
                    return None;
                }
                Some(pipeline::types::CrcEntry {
                    file: pipeline::types::FileEntry {
                        path: dest,
                        size,
                        mtime_ms,
                    },
                    src_path: Some(src),
                    staged_path: Some(staged),
                    crc32c,
                    raw_unique_id,
                    precomputed_hash: None,
                })
            })
            .collect();

        sink.emit(&Event::PhaseFinished { phase: Phase::Copy });

        let error_count = total as usize - copied.len();
        Ok((copied, error_count))
    }

    /// Stage D: compute strong hashes (XXH3-128, optionally SHA-256).
    ///
    /// Also performs a post-hash dedup check unless `force` is set.
    #[allow(clippy::too_many_arguments)]
    fn stage_hash(
        copied: Vec<pipeline::types::CrcEntry>,
        source_canon: &Path,
        vault_root: &Path,
        force: bool,
        full_id: bool,
        db: &Db,
        sink: &dyn EventSink,
    ) -> anyhow::Result<Vec<pipeline::types::HashResult>> {
        let total = copied.len() as u64;

        sink.emit(&Event::PhaseStarted {
            phase: Phase::Hash,
            total: Some(total),
            context: PhaseContext::both(source_canon.to_path_buf(), vault_root.to_path_buf()),
        });

        let hash_results = pipeline::hash::compute_hashes(copied, force || full_id, Some(sink));

        sink.emit(&Event::PhaseFinished { phase: Phase::Hash });

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
    /// Crash-safe ordering: the DB transaction commits first (recording the
    /// final paths), then each committed staged file is atomically renamed
    /// to its final destination. If the process dies between commit and
    /// rename, the next import's staging reconcile finishes the renames.
    #[allow(clippy::too_many_arguments)]
    fn stage_insert(
        hash_results: Vec<pipeline::types::HashResult>,
        vault_root: &Path,
        source_root: &Path,
        session_id: &str,
        session_dir: &Path,
        force: bool,
        db: &Db,
        total_files: usize,
        likely_dup: usize,
        copy_error_count: usize,
        sink: &dyn EventSink,
    ) -> anyhow::Result<ImportSummary> {
        let insert_count = hash_results.len() as u64;

        sink.emit(&Event::PhaseStarted {
            phase: Phase::Insert,
            total: Some(insert_count),
            context: PhaseContext::both(source_root.to_path_buf(), vault_root.to_path_buf()),
        });

        let progress = std::sync::atomic::AtomicU64::new(0);
        let progress_cb = || {
            let done = progress.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            sink.emit(&Event::Progress {
                phase: Phase::Insert,
                done,
                total: insert_count,
            });
        };

        let insert_opts = pipeline::insert::InsertOptions {
            vault_root,
            session_id,
            write_manifest: true,
            source_root: Some(source_root),
            force,
            session_type: crate::verify::manifest::SessionType::Import,
        };

        let result =
            pipeline::insert::batch_insert(hash_results, db, insert_opts, Some(&progress_cb))?;

        // The DB transaction has committed: make the committed files visible
        // by atomically renaming them out of the staging area. A rename
        // failure is non-fatal — the file stays staged with a valid DB
        // record and the next import's reconcile finishes the rename.
        let mut deferred = 0usize;
        for (staged, dest) in &result.staged_commits {
            if let Err(e) = crate::fs::atomic_commit(staged, dest) {
                deferred += 1;
                sink.emit(&Event::Hint(Hint::StagedCommitDeferred {
                    staged: staged.clone(),
                    dest: dest.clone(),
                    error: e.to_string(),
                }));
            }
        }

        // Everything left in this session's staging subtree is residue of
        // files that never entered the transaction (copy/hash failures,
        // Stage-D duplicates): created by THIS still-running session, safe
        // to remove. Skip cleanup entirely when a rename was deferred —
        // those staged files hold the only copy of DB-recorded content and
        // must survive for the next reconcile. plan.json / manifest.json
        // always stay as the session's audit record.
        if deferred == 0 {
            let _ = fs::remove_dir_all(crate::session::staging_dir(session_dir));
        }

        let done = progress.load(std::sync::atomic::Ordering::Relaxed);
        if done < insert_count {
            sink.emit(&Event::Progress {
                phase: Phase::Insert,
                done,
                total: insert_count,
            });
        }
        sink.emit(&Event::PhaseFinished {
            phase: Phase::Insert,
        });

        let import_summary = ImportSummary {
            total: total_files,
            imported: result.added,
            duplicate: result.duplicate + likely_dup,
            failed: result.failed + copy_error_count,
            manifest_path: result.manifest_path.clone(),
            all_cache_hit: false,
        };

        sink.emit(&Event::Summary(Summary::Import(import_summary.clone())));

        Ok(import_summary)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ItemStatus;
    use std::sync::Mutex;

    /// A sink that records every event for assertions.
    #[derive(Debug, Default)]
    struct RecordingSink(Mutex<Vec<Event>>);

    impl EventSink for RecordingSink {
        fn emit(&self, event: &Event) {
            self.0.lock().unwrap().push(event.clone());
        }
    }

    #[test]
    fn test_recording_sink_captures_scan_items() {
        let sink = RecordingSink::default();
        sink.emit(&Event::ScanItem {
            path: PathBuf::from("/source/photo.jpg"),
            size: 1024,
            mtime_ms: 0,
            status: ItemStatus::New,
            error: None,
        });
        sink.emit(&Event::PhaseFinished { phase: Phase::Scan });

        let events = sink.0.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            Event::ScanItem {
                status: ItemStatus::New,
                ..
            }
        ));
        assert!(matches!(
            &events[1],
            Event::PhaseFinished { phase: Phase::Scan }
        ));
    }

    #[test]
    fn test_event_serializes_to_json() {
        let event = Event::Preflight {
            source: PathBuf::from("/source"),
            total: 10,
            new: 7,
            duplicate: 2,
            moved: 1,
            failed: 0,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"event\":\"preflight\""));
        assert!(json.contains("\"new\":7"));
    }

    #[test]
    fn test_summary_serializes_with_kind_tag() {
        let event = Event::Summary(Summary::Import(ImportSummary {
            total: 3,
            imported: 2,
            duplicate: 1,
            failed: 0,
            manifest_path: None,
            all_cache_hit: false,
        }));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"event\":\"summary\""));
        assert!(json.contains("\"kind\":\"import\""));
        assert!(json.contains("\"imported\":2"));
    }

    // ── normalize_path ─────────────────────────────────────────────────────

    #[test]
    fn test_normalize_path_preserves_unix_root() {
        assert_eq!(normalize_path(Path::new("/")), PathBuf::from("/"));
    }

    #[test]
    fn test_normalize_path_removes_trailing_slashes() {
        assert_eq!(
            normalize_path(Path::new("/home/user/")),
            PathBuf::from("/home/user")
        );
    }

    #[test]
    fn test_normalize_path_removes_trailing_backslashes() {
        assert_eq!(
            normalize_path(Path::new("/home/user\\")),
            PathBuf::from("/home/user")
        );
    }

    #[test]
    fn test_normalize_path_removes_quotes() {
        assert_eq!(
            normalize_path(Path::new("/home/user\"")),
            PathBuf::from("/home/user")
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_normalize_path_preserves_windows_drive_root() {
        let normalized = normalize_path(Path::new("C:\\"));
        assert!(normalized.to_string_lossy().starts_with("C:"));
        assert!(normalized.to_string_lossy().contains('\\'));
    }

    #[test]
    fn test_normalize_path_empty_becomes_unix_root() {
        assert_eq!(normalize_path(Path::new("")), PathBuf::from("/"));
    }

    // ── staging end-to-end ────────────────────────────────────────────────

    /// Run a full import over `source` into `vault` and return the summary.
    fn run_test_import(source: &Path, vault: &Path, db: &Db) -> ImportSummary {
        run_test_import_with_level(source, vault, db, crate::ops::types::CompareLevel::Fast)
    }

    fn run_test_import_with_level(
        source: &Path,
        vault: &Path,
        db: &Db,
        compare_level: crate::ops::types::CompareLevel,
    ) -> ImportSummary {
        let opts = ImportOptions {
            source: source.to_path_buf(),
            vault_root: vault.to_path_buf(),
            strategy: crate::config::SyncStrategy(vec![crate::config::TransferStrategyArg::Copy]),
            dry_run: false,
            yes: true,
            import_config: ImportConfig::default(),
            force: false,
            full_id: false,
            show_dup: false,
            files_from: None,
            max_depth: 0,
            include: Vec::new(),
            exclude: Vec::new(),
            compare_level,
        };
        opts.run_import(db, &crate::event::NoopSink, &crate::event::YesInteractor)
            .unwrap()
    }

    /// Locate the single imported file below the vault (outside `.svault`).
    fn find_imported_file(vault: &Path, name: &str) -> PathBuf {
        walkdir::WalkDir::new(vault)
            .into_iter()
            .filter_map(|e| e.ok())
            .map(|e| e.into_path())
            .find(|p| {
                p.file_name().map(|n| n == name).unwrap_or(false)
                    && !p.starts_with(vault.join(".svault"))
            })
            .unwrap_or_else(|| panic!("{name} not found in vault"))
    }

    /// All import session journal directories in the vault.
    fn import_session_dirs(vault: &Path) -> Vec<PathBuf> {
        let root = crate::session::sessions_root(vault).join("import");
        std::fs::read_dir(&root)
            .map(|rd| rd.flatten().map(|e| e.path()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn run_import_commits_staged_files_and_leaves_no_residue() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source");
        let vault = tmp.path().join("vault");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&vault).unwrap();
        fs::write(source.join("photo.jpg"), b"jpeg-bytes").unwrap();

        let db = Db::open_in_memory().unwrap();
        let summary = run_test_import(&source, &vault, &db);

        assert_eq!(summary.imported, 1);
        assert_eq!(summary.failed, 0);

        // The file is visible at its final path with intact content…
        let dest = find_imported_file(&vault, "photo.jpg");
        assert_eq!(fs::read(&dest).unwrap(), b"jpeg-bytes");

        // …its DB record points at that same path…
        let rel = dest.strip_prefix(&vault).unwrap().to_string_lossy();
        let rel_unix = rel.replace('\\', "/");
        let record = db
            .get_file_by_path(&rel_unix)
            .unwrap()
            .expect("DB record must exist for the committed file");
        assert_eq!(record.status, "imported");

        // …and the session journal holds plan + manifest with the staging
        // payload subtree fully cleaned up.
        let sessions = import_session_dirs(&vault);
        assert_eq!(sessions.len(), 1, "exactly one session journal");
        let session = &sessions[0];
        assert!(session.join(crate::session::PLAN_FILE).exists());
        assert!(session.join(crate::session::MANIFEST_FILE).exists());
        assert!(
            !crate::session::staging_dir(session).exists(),
            "staging payload must be cleaned up after a successful import"
        );

        // The plan records the intent: source, final destination, size.
        let plan: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(session.join(crate::session::PLAN_FILE)).unwrap(),
        )
        .unwrap();
        let files = plan["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert!(
            files[0]["src_path"]
                .as_str()
                .unwrap()
                .ends_with("photo.jpg")
        );
        assert_eq!(files[0]["dest_path"].as_str().unwrap(), rel_unix);
        assert_eq!(files[0]["size"].as_u64().unwrap(), 10);
    }

    #[test]
    fn rerun_import_is_idempotent_with_staging() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source");
        let vault = tmp.path().join("vault");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&vault).unwrap();
        fs::write(source.join("photo.jpg"), b"jpeg-bytes").unwrap();

        let db = Db::open_in_memory().unwrap();
        let first = run_test_import(&source, &vault, &db);
        assert_eq!(first.imported, 1);

        // Second run: CRC short-circuit — no copy, no insert, no new session.
        let second = run_test_import(&source, &vault, &db);
        assert_eq!(second.imported, 0);
        assert_eq!(second.duplicate, 1);
        assert!(second.all_cache_hit);
        assert_eq!(import_session_dirs(&vault).len(), 1);
    }

    #[test]
    fn compare_level_mid_catches_fingerprint_blind_spot_edit() {
        use crate::ops::types::CompareLevel;

        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source");
        let vault = tmp.path().join("vault");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&vault).unwrap();

        // 200 KiB file: the JPEG CRC fingerprint reads head+tail 64 KiB,
        // leaving a blind zone in [64 KiB, 136 KiB).
        let target = source.join("big.jpg");
        fs::write(&target, vec![0xABu8; 200 * 1024]).unwrap();

        let db = Db::open_in_memory().unwrap();
        let first = run_test_import(&source, &vault, &db);
        assert_eq!(first.imported, 1);

        // Edit inside the blind zone: size and fingerprint unchanged.
        let flip_middle = |byte: u8| {
            let mut data = fs::read(&target).unwrap();
            data[100 * 1024] = byte;
            fs::write(&target, data).unwrap();
        };
        flip_middle(0xCD);

        // fast: fingerprint hit → duplicate, no new import.
        let fast = run_test_import(&source, &vault, &db);
        assert_eq!(fast.imported, 0);
        assert_eq!(fast.duplicate, 1);

        flip_middle(0xCE);

        // mid: full XXH3 of the source mismatches the DB record → imported
        // as a new file (renamed destination).
        let mid = run_test_import_with_level(&source, &vault, &db, CompareLevel::Mid);
        assert_eq!(mid.imported, 1);
        let visible: Vec<_> = walkdir::WalkDir::new(&vault)
            .into_iter()
            .filter_map(|e| e.ok())
            .map(|e| e.into_path())
            .filter(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().starts_with("big"))
                    .unwrap_or(false)
                    && !p.starts_with(vault.join(".svault"))
            })
            .collect();
        assert_eq!(visible.len(), 2, "original + renamed second copy");
        assert_eq!(
            db.get_all_files().unwrap().len(),
            2,
            "both contents are recorded"
        );
    }
}
