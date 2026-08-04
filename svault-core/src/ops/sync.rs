//! `svault sync` — copy files that exist in a peer vault but not in this one.
//!
//! Beyond Compare style: compare both vaults' database records (hash
//! accelerated, no full re-hashing), show a diff plan, copy what's missing,
//! and record the arrival as a normal `Sync` session in the local vault.
//! The peer vault is opened read-only and is never modified.
//!
//! See `docs/ARCHITECTURE.md` §6.2.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::SyncStrategy;
use crate::db::Db;
use crate::event::{Event, EventSink, Interactor, Phase, PhaseContext, Summary, SyncSummary};
use crate::fs::transfer_file;
use crate::pipeline;
use crate::pipeline::types::{FileHash, HashResult};
use crate::sync::{FileRecord, diff_vaults};

/// Scope of post-sync integrity verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncVerifyScope {
    /// No post-sync verification.
    None,
    /// Verify only files added in this sync (default).
    #[default]
    Norm,
    /// Verify every file in the local vault database.
    Full,
}

impl std::str::FromStr for SyncVerifyScope {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "none" => Ok(SyncVerifyScope::None),
            "norm" => Ok(SyncVerifyScope::Norm),
            "full" => Ok(SyncVerifyScope::Full),
            other => Err(format!("unknown verify scope: {other}")),
        }
    }
}

impl std::fmt::Display for SyncVerifyScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SyncVerifyScope::None => "none",
            SyncVerifyScope::Norm => "norm",
            SyncVerifyScope::Full => "full",
        };
        write!(f, "{}", s)
    }
}

/// Options for `svault sync`.
pub struct SyncOptions {
    /// Root of the source (peer) vault — opened read-only.
    pub source_vault: PathBuf,
    /// Root of the local (destination) vault.
    pub dest_vault_root: PathBuf,
    /// File transfer strategy.
    pub strategy: SyncStrategy,
    /// Post-sync verification scope.
    pub verify: SyncVerifyScope,
    /// Skip the confirmation prompt.
    pub yes: bool,
}

/// Run sync: diff against the source vault, copy missing files, record them.
pub fn run_sync(
    opts: SyncOptions,
    dest_db: &Db,
    sink: &dyn EventSink,
    interactor: &dyn Interactor,
) -> anyhow::Result<SyncSummary> {
    // ── Open the peer vault read-only ───────────────────────────────────────
    let source_canon = dunce::canonicalize(&opts.source_vault)
        .map_err(|e| anyhow::anyhow!("cannot access source vault: {}", e))?;
    let dest_canon =
        dunce::canonicalize(&opts.dest_vault_root).unwrap_or_else(|_| opts.dest_vault_root.clone());

    if source_canon == dest_canon {
        anyhow::bail!("source and destination are the same vault");
    }

    let source_db_path = source_canon.join(".svault").join("vault.db");
    if !source_db_path.exists() {
        anyhow::bail!(
            "'{}' is not a svault vault (no .svault/vault.db)",
            source_canon.display()
        );
    }
    let source_db = Db::open_readonly(&source_db_path)
        .map_err(|e| anyhow::anyhow!("cannot open source vault database: {}", e))?;

    // ── Compare (hash-accelerated: DB records only, no file IO) ─────────────
    sink.emit(&Event::PhaseStarted {
        phase: Phase::Compare,
        total: None,
        context: PhaseContext::both(source_canon.clone(), dest_canon.clone()),
    });

    let source_records: Vec<FileRecord> = source_db
        .get_all_files()?
        .iter()
        .filter(|f| f.status == "imported")
        .map(FileRecord::from)
        .collect();
    let dest_records: Vec<FileRecord> = dest_db
        .get_all_files()?
        .iter()
        .filter(|f| f.status == "imported")
        .map(FileRecord::from)
        .collect();

    let plan = diff_vaults(&source_records, &dest_records);

    sink.emit(&Event::PhaseFinished {
        phase: Phase::Compare,
    });

    sink.emit(&Event::SyncPlan {
        source_vault: source_canon.clone(),
        identical: plan.identical(),
        to_copy: plan.to_copy.len(),
        copy_bytes: plan.copy_bytes(),
        moved: plan.moved(),
        only_dest: plan.only_dest(),
        conflicts: plan.conflicts(),
    });

    let mut summary = SyncSummary {
        identical: plan.identical(),
        skipped: plan.skipped_hashless,
        only_dest: plan.only_dest(),
        moved: plan.moved(),
        conflicts: plan.conflicts(),
        conflict_paths: plan.conflict_paths(),
        ..Default::default()
    };

    if plan.to_copy.is_empty() {
        sink.emit(&Event::Summary(Summary::Sync(summary.clone())));
        return Ok(summary);
    }

    if !opts.yes && !interactor.confirm("Proceed with sync?") {
        return Ok(summary);
    }

    // ── Copy phase ──────────────────────────────────────────────────────────
    let strategies = opts.strategy.to_transfer_strategies();
    let copy_total = plan.to_copy.len() as u64;

    sink.emit(&Event::PhaseStarted {
        phase: Phase::Copy,
        total: Some(copy_total),
        context: PhaseContext::both(source_canon.clone(), dest_canon.clone()),
    });

    let mut hash_results: Vec<HashResult> = Vec::with_capacity(plan.to_copy.len());

    for record in &plan.to_copy {
        let rel = Path::new(&record.path);
        let src_abs = source_canon.join(rel);
        let dest_abs = dest_canon.join(rel);

        // transfer_file creates parent directories internally for every
        // strategy and emits CopyStarted/CopyFinished (with error) itself.
        match transfer_file(
            &source_canon,
            rel,
            &dest_canon,
            rel,
            &strategies,
            Some(sink),
        ) {
            Ok(_) => {
                summary.copied += 1;
                summary.bytes += record.size.max(0) as u64;

                let hash = match (&record.xxh3_128, &record.sha256) {
                    (Some(x), Some(s)) => FileHash::Full(x.clone(), s.clone()),
                    (Some(x), None) => FileHash::Fast(x.clone()),
                    (None, Some(s)) => {
                        // Rare: source record has SHA-256 but no XXH3-128.
                        // Compute XXH3 from the freshly copied file so the
                        // dest record gets a complete identity (otherwise the
                        // copied file would sit on disk with no DB record).
                        match crate::hash::xxh3_128_file(&dest_abs) {
                            Ok(h) => FileHash::Full(h.to_bytes().to_vec(), s.clone()),
                            Err(e) => {
                                sink.emit(&Event::ApplyError {
                                    path: record.path.clone(),
                                    message: format!("hash failed after copy: {e}"),
                                });
                                summary.failed += 1;
                                continue;
                            }
                        }
                    }
                    (None, None) => {
                        // Filtered out by diff (skipped_hashless).
                        continue;
                    }
                };

                hash_results.push(HashResult {
                    path: dest_abs,
                    src_path: Some(src_abs),
                    size: record.size as u64,
                    mtime_ms: record.mtime,
                    crc32c: record.crc32c.map(|v| v as u32).unwrap_or(0),
                    raw_unique_id: record.raw_unique_id.clone(),
                    hash,
                    is_duplicate: false,
                    dup_reason: None,
                });
            }
            Err(_) => {
                summary.failed += 1;
            }
        }
    }

    sink.emit(&Event::PhaseFinished { phase: Phase::Copy });

    // ── Insert phase (dest vault records the arrival as a Sync session) ─────
    if !hash_results.is_empty() {
        let session_id = crate::ops::utils::session_id_now();
        let insert_count = hash_results.len() as u64;

        sink.emit(&Event::PhaseStarted {
            phase: Phase::Insert,
            total: Some(insert_count),
            context: PhaseContext::both(source_canon.clone(), dest_canon.clone()),
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
            vault_root: &dest_canon,
            session_id: &session_id,
            write_manifest: true,
            source_root: Some(&source_canon),
            force: false,
            session_type: crate::verify::manifest::SessionType::Sync,
        };

        let result =
            pipeline::insert::batch_insert(hash_results, dest_db, insert_opts, Some(&progress_cb))?;
        summary.failed += result.failed;
        summary.manifest_path = result.manifest_path.clone();

        sink.emit(&Event::PhaseFinished {
            phase: Phase::Insert,
        });
    }

    // ── Post-sync verification ──────────────────────────────────────────────
    match opts.verify {
        SyncVerifyScope::None => {}
        SyncVerifyScope::Norm => {
            // Verify only the files added in this sync.
            let copied_rel: Vec<String> = plan.to_copy.iter().map(|r| r.path.clone()).collect();
            verify_paths(&dest_canon, dest_db, &copied_rel, sink)?;
        }
        SyncVerifyScope::Full => {
            crate::verify::verify_all(&dest_canon, dest_db, sink)?;
        }
    }

    sink.emit(&Event::Summary(Summary::Sync(summary.clone())));
    Ok(summary)
}

/// Verify a list of vault-relative paths, emitting VerifyItem events.
fn verify_paths(
    vault_root: &Path,
    db: &Db,
    rel_paths: &[String],
    sink: &dyn EventSink,
) -> anyhow::Result<()> {
    sink.emit(&Event::PhaseStarted {
        phase: Phase::Verify,
        total: Some(rel_paths.len() as u64),
        context: PhaseContext::vault(vault_root.to_path_buf()),
    });

    for rel in rel_paths {
        if let Some(row) = db.get_file_by_path(rel)? {
            let result = crate::verify::verify_file(vault_root, &row);
            sink.emit(&Event::VerifyItem {
                path: PathBuf::from(rel),
                result,
            });
        }
    }

    sink.emit(&Event::PhaseFinished {
        phase: Phase::Verify,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build two on-disk vaults with DBs; source has files {a, b}, dest has {a}.
    fn make_vaults() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src_vault");
        let dst = tmp.path().join("dst_vault");
        for root in [&src, &dst] {
            std::fs::create_dir_all(root.join(".svault")).unwrap();
            std::fs::create_dir_all(root.join("2024")).unwrap();
            std::fs::write(root.join("2024/a.jpg"), "content-a.jpg").unwrap();

            let db = Db::open(&root.join(".svault/vault.db")).unwrap();
            db.insert_file_row(
                "2024/a.jpg",
                13,
                1000,
                None,
                None,
                Some(&[1u8; 16]),
                None,
                "imported",
                1000,
            )
            .unwrap();
        }
        // Source additionally has b.jpg
        std::fs::write(src.join("2024/b.jpg"), "content-b.jpg").unwrap();
        let src_db = Db::open(&src.join(".svault/vault.db")).unwrap();
        src_db
            .insert_file_row(
                "2024/b.jpg",
                13,
                1000,
                None,
                None,
                Some(&[2u8; 16]),
                None,
                "imported",
                1000,
            )
            .unwrap();

        (tmp, src, dst)
    }

    #[test]
    fn sync_copies_missing_files() {
        let (_tmp, src, dst) = make_vaults();
        let dest_db = Db::open(&dst.join(".svault/vault.db")).unwrap();

        let sink = crate::event::NoopSink;
        let summary = run_sync(
            SyncOptions {
                source_vault: src,
                dest_vault_root: dst.clone(),
                strategy: SyncStrategy(vec![crate::config::TransferStrategyArg::Copy]),
                verify: SyncVerifyScope::Norm,
                yes: true,
            },
            &dest_db,
            &sink,
            &crate::event::YesInteractor,
        )
        .unwrap();

        assert_eq!(summary.identical, 1);
        assert_eq!(summary.copied, 1);
        assert_eq!(summary.failed, 0);
        assert!(dst.join("2024/b.jpg").exists());

        // b.jpg recorded in dest DB
        let row = dest_db.get_file_by_path("2024/b.jpg").unwrap();
        assert!(row.is_some());
    }

    #[test]
    fn sync_refuses_same_vault() {
        let (_tmp, src, _dst) = make_vaults();
        let db = Db::open(&src.join(".svault/vault.db")).unwrap();
        let result = run_sync(
            SyncOptions {
                source_vault: src.clone(),
                dest_vault_root: src,
                strategy: SyncStrategy::default(),
                verify: SyncVerifyScope::None,
                yes: true,
            },
            &db,
            &crate::event::NoopSink,
            &crate::event::YesInteractor,
        );
        assert!(result.is_err());
    }

    #[test]
    fn sync_refuses_non_vault_source() {
        let tmp = tempfile::tempdir().unwrap();
        let not_vault = tmp.path().join("plain");
        std::fs::create_dir_all(&not_vault).unwrap();
        let (_t, _s, dst) = make_vaults();
        let db = Db::open(&dst.join(".svault/vault.db")).unwrap();
        let result = run_sync(
            SyncOptions {
                source_vault: not_vault,
                dest_vault_root: dst,
                strategy: SyncStrategy::default(),
                verify: SyncVerifyScope::None,
                yes: true,
            },
            &db,
            &crate::event::NoopSink,
            &crate::event::YesInteractor,
        );
        assert!(result.is_err());
    }
}
