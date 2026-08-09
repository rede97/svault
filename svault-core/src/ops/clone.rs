//! `svault clone` — export a subset of the vault to a plain directory.
//!
//! Clone is one-directional and conflict-free: the target is **not** a vault,
//! just a working directory that receives copies of the selected files plus
//! a `svault-clone-manifest.json` describing what was exported.
//!
//! See `docs/ARCHITECTURE.md` §6.1.

use std::path::{Path, PathBuf};

use crate::config::SyncStrategy;
use crate::db::Db;
use crate::event::{CloneSummary, Event, EventSink, Phase, PhaseContext, Summary};
use crate::fs::transfer_file;

/// Options for `svault clone`.
pub struct CloneOptions {
    /// Vault root (source).
    pub vault_root: PathBuf,
    /// Target directory (created if missing; must not be inside the vault).
    pub target: PathBuf,
    /// Optional mtime filter, Unix-ms range `[start, end)`.
    pub filter_date: Option<(i64, i64)>,
    /// File transfer strategy.
    pub strategy: SyncStrategy,
}

/// Parse a `--filter-date` value of the form `YYYY-MM-DD..YYYY-MM-DD`.
///
/// The range is inclusive of both endpoints; the returned range is
/// `[start_of_first_day, start_of_day_after_last_day)` in Unix ms.
pub fn parse_date_range(s: &str) -> anyhow::Result<(i64, i64)> {
    let (start, end) = s
        .split_once("..")
        .ok_or_else(|| anyhow::anyhow!("date range must be YYYY-MM-DD..YYYY-MM-DD, got '{}'", s))?;

    let parse_day = |d: &str| -> anyhow::Result<chrono::NaiveDate> {
        chrono::NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d")
            .map_err(|e| anyhow::anyhow!("invalid date '{}': {}", d.trim(), e))
    };

    let start_day = parse_day(start)?;
    let end_day = parse_day(end)?;
    if end_day < start_day {
        anyhow::bail!("date range end {} is before start {}", end, start);
    }

    let day_ms = 86_400_000i64;
    let start_ms = start_day
        .and_hms_milli_opt(0, 0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();
    let end_ms = end_day
        .and_hms_milli_opt(0, 0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis()
        + day_ms;

    Ok((start_ms, end_ms))
}

/// Run clone: copy matching vault files to `opts.target`.
pub fn run_clone(
    opts: CloneOptions,
    db: &Db,
    sink: &dyn EventSink,
) -> anyhow::Result<CloneSummary> {
    // ── Validation ──────────────────────────────────────────────────────────
    let vault_canon =
        dunce::canonicalize(&opts.vault_root).unwrap_or_else(|_| opts.vault_root.clone());
    std::fs::create_dir_all(&opts.target)?;
    let target_canon = dunce::canonicalize(&opts.target).unwrap_or_else(|_| opts.target.clone());

    if target_canon.ancestors().any(|p| p == vault_canon) {
        anyhow::bail!(
            "clone target '{}' is inside the vault — refusing to nest exports",
            target_canon.display()
        );
    }

    // ── Select files ────────────────────────────────────────────────────────
    let files: Vec<crate::db::FileRow> = db
        .get_all_files()?
        .into_iter()
        .filter(|f| f.status == "imported")
        .filter(|f| match opts.filter_date {
            Some((start, end)) => f.mtime >= start && f.mtime < end,
            None => true,
        })
        .collect();

    let total = files.len();
    let mut summary = CloneSummary {
        total,
        ..Default::default()
    };

    if total == 0 {
        sink.emit(&Event::Summary(Summary::Clone(summary.clone())));
        return Ok(summary);
    }

    // ── Copy phase ──────────────────────────────────────────────────────────
    let strategies = opts.strategy.to_transfer_strategies();

    sink.emit(&Event::PhaseStarted {
        phase: Phase::Copy,
        total: Some(total as u64),
        context: PhaseContext::both(vault_canon.clone(), target_canon.clone()),
    });

    let mut copied_paths: Vec<&crate::db::FileRow> = Vec::with_capacity(total);

    for file in &files {
        let rel = Path::new(&file.path);
        match transfer_file(
            &vault_canon,
            rel,
            &target_canon,
            rel,
            &strategies,
            Some(sink),
        ) {
            Ok(_) => {
                summary.copied += 1;
                summary.bytes += file.size.max(0) as u64;
                copied_paths.push(file);
            }
            Err(_) => {
                summary.failed += 1;
            }
        }
    }

    sink.emit(&Event::PhaseFinished { phase: Phase::Copy });

    // ── Manifest ────────────────────────────────────────────────────────────
    let manifest_path = target_canon.join("svault-clone-manifest.json");
    let manifest = serde_json::json!({
        "session_type": "clone",
        "cloned_at": chrono::Utc::now().to_rfc3339(),
        "vault_root": vault_canon,
        "filter_date_ms": opts.filter_date.map(|(s, e)| [s, e]),
        "files": copied_paths.iter().map(|f| {
            serde_json::json!({
                "path": f.path,
                "size": f.size,
                "mtime_ms": f.mtime,
                "xxh3_128": f.xxh3_128.as_deref().map(hex),
                "sha256": f.sha256.as_deref().map(hex),
            })
        }).collect::<Vec<_>>(),
        "summary": {
            "total": summary.total,
            "copied": summary.copied,
            "failed": summary.failed,
            "bytes": summary.bytes,
        }
    });
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;
    summary.manifest_path = Some(manifest_path);

    sink.emit(&Event::Summary(Summary::Clone(summary.clone())));
    Ok(summary)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_range() {
        let (start, end) = parse_date_range("2024-03-01..2024-03-31").unwrap();
        assert_eq!(start, 1_709_251_200_000); // 2024-03-01T00:00:00Z
        assert_eq!(end - start, 31 * 86_400_000); // 31 days
    }

    #[test]
    fn parse_rejects_inverted_range() {
        assert!(parse_date_range("2024-03-31..2024-03-01").is_err());
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_date_range("march").is_err());
        assert!(parse_date_range("2024-03-01").is_err());
        assert!(parse_date_range("2024-13-01..2024-14-01").is_err());
    }

    #[test]
    fn clone_copies_matching_files() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let target = tmp.path().join("export");
        std::fs::create_dir_all(vault.join("2024/01")).unwrap();
        std::fs::write(vault.join("2024/01/a.jpg"), b"content-a").unwrap();

        let db = Db::open_in_memory().unwrap();
        db.insert_file_row(
            "2024/01/a.jpg",
            9,
            1000,
            Some(42),
            None,
            Some(&[1u8; 16]),
            None,
            "imported",
            1000,
        )
        .unwrap();
        db.insert_file_row(
            "2024/01/missing.jpg",
            9,
            1000,
            None,
            None,
            None,
            None,
            "missing",
            1000,
        )
        .unwrap();

        let sink = crate::event::NoopSink;
        let summary = run_clone(
            CloneOptions {
                vault_root: vault.clone(),
                target: target.clone(),
                filter_date: None,
                strategy: SyncStrategy(vec![crate::config::TransferStrategyArg::Copy]),
            },
            &db,
            &sink,
        )
        .unwrap();

        assert_eq!(summary.total, 1); // 'missing' record excluded
        assert_eq!(summary.copied, 1);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.bytes, 9);
        assert!(target.join("2024/01/a.jpg").exists());
        assert!(target.join("svault-clone-manifest.json").exists());
    }

    #[test]
    fn clone_refuses_target_inside_vault() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        let db = Db::open_in_memory().unwrap();
        let sink = crate::event::NoopSink;

        let result = run_clone(
            CloneOptions {
                vault_root: vault.clone(),
                target: vault.join("export"),
                filter_date: None,
                strategy: SyncStrategy::default(),
            },
            &db,
            &sink,
        );
        assert!(result.is_err());
    }
}
