//! Session journal directories (`.svault/sessions/<kind>/<ts-id>/`).
//!
//! Every vault operation that copies files or produces an audit record gets
//! one session directory:
//!
//! ```text
//! .svault/sessions/
//! ├── import/<ts-id>/
//! │   ├── plan.json            # pre-copy intent (atomically written)
//! │   ├── staging/…            # staged payload, mirrors final relative paths
//! │   └── manifest.json        # outcome (atomically written after DB commit)
//! ├── sync/<ts-id>/
//! │   ├── plan.json            # diff plan
//! │   └── manifest.json
//! └── recheck/<ts-id>/
//!     └── report.json
//! ```
//!
//! Directory content IS the state — no extra state machine:
//!
//! - `manifest.json` present  → committed (audit record, kept permanently);
//!   staging residue may still await renames (finished by [`reconcile`]).
//! - `manifest.json` missing  → interrupted; reconcile reports the leftover
//!   directory to the user and NEVER deletes it (svault only removes staging
//!   residue created by its own, still-running session).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::db::Db;
use crate::event::{Event, EventSink, Hint};
use crate::verify::manifest::SessionType;

/// Pre-copy intent of one file, recorded in `plan.json` before Stage C.
#[derive(Debug, Clone, Serialize)]
pub struct PlanEntry {
    /// Absolute source path.
    pub src_path: PathBuf,
    /// Vault-relative destination (Unix-style separators).
    pub dest_path: String,
    pub size: u64,
    pub crc32c: u32,
}

/// Pre-copy intent of an import session, atomically written to `plan.json`
/// before any file is transferred. The plan is a **hint for post-mortem
/// inspection** — the database remains the single source of truth.
#[derive(Debug, Clone, Serialize)]
pub struct ImportPlan {
    pub session_id: String,
    pub session_type: SessionType,
    pub source_root: PathBuf,
    /// Unix milliseconds when the plan was written.
    pub created_at: i64,
    pub files: Vec<PlanEntry>,
}

/// Pre-copy intent file inside a session directory.
pub const PLAN_FILE: &str = "plan.json";
/// Outcome manifest file inside a session directory.
pub const MANIFEST_FILE: &str = "manifest.json";
/// Staged payload subdirectory of an import session.
pub const STAGING_DIR: &str = "staging";
/// Recheck report file inside a recheck session directory.
pub const REPORT_FILE: &str = "report.json";

/// Root of all session journals inside a vault.
pub fn sessions_root(vault_root: &Path) -> PathBuf {
    vault_root.join(".svault").join("sessions")
}

/// Session directory of one operation: `sessions/<kind>/<session_id>`.
pub fn session_dir(vault_root: &Path, kind: SessionType, session_id: &str) -> PathBuf {
    sessions_root(vault_root)
        .join(kind.to_string())
        .join(session_id)
}

/// Staged payload subdirectory of an import session directory.
pub fn staging_dir(session_dir: &Path) -> PathBuf {
    session_dir.join(STAGING_DIR)
}

/// Map a final vault destination to its staged path within `session_dir`
/// (`<session>/staging/<final relative path>`).
pub fn staged_path_for(session_dir: &Path, vault_root: &Path, dest: &Path) -> PathBuf {
    let rel = dest.strip_prefix(vault_root).unwrap_or(dest);
    staging_dir(session_dir).join(rel)
}

/// Serialize `value` as pretty JSON and atomically write it to `path`
/// (creating parent directories). See [`crate::fs::atomic_write`].
pub fn write_json_atomic(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(value)?;
    crate::fs::atomic_write(path, json.as_bytes()).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

/// Outcome of [`reconcile`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReconcileStats {
    /// Staged files whose DB record existed; the pending rename was finished.
    pub completed: usize,
    /// Session directories that still hold residue afterwards.
    pub residue_sessions: usize,
    /// Leftover staged files (reported to the user, never deleted).
    pub residue_files: usize,
    /// Total size of leftover staged files in bytes.
    pub residue_bytes: u64,
}

/// Reconcile import-session leftovers from an interrupted run.
///
/// For every `sessions/import/<ts-id>/staging/` tree:
///
/// - staged file whose final path has an `imported` DB record and no file at
///   the destination → the process died between DB commit and rename; the
///   rename is completed now ([`Hint::StagingReconciled`] aggregates these);
/// - anything else → residue from an interrupted session. It is **reported
///   ([`Hint::SessionResidue`]) but never deleted**: the user reviews
///   `plan.json` inside the session directory and removes it manually.
///
/// Best-effort: per-file errors skip that file and never abort the import.
pub fn reconcile(vault_root: &Path, db: &Db, sink: &dyn EventSink) -> ReconcileStats {
    let mut stats = ReconcileStats::default();
    let import_root = sessions_root(vault_root).join(SessionType::Import.to_string());
    let Ok(sessions) = std::fs::read_dir(&import_root) else {
        return stats;
    };

    for session_entry in sessions.flatten() {
        let session_dir = session_entry.path();
        let staging = staging_dir(&session_dir);
        if !staging.is_dir() {
            continue;
        }

        let mut staged_files = Vec::new();
        collect_files(&staging, &mut staged_files);

        let mut residue_files = 0usize;
        let mut residue_bytes = 0u64;
        for staged in &staged_files {
            let Ok(rel) = staged.strip_prefix(&staging) else {
                continue;
            };
            // DB paths are stored Unix-style (forward slashes).
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let dest = vault_root.join(rel);
            let recorded = matches!(
                db.get_file_by_path(&rel_str),
                Ok(Some(record)) if record.status == "imported"
            );

            if recorded && !dest.exists() {
                // Killed between DB commit and rename: finish the rename.
                if crate::fs::atomic_commit(staged, &dest).is_ok() {
                    stats.completed += 1;
                    continue;
                }
            }
            residue_files += 1;
            residue_bytes += std::fs::metadata(staged).map(|m| m.len()).unwrap_or(0);
        }

        if residue_files > 0 {
            stats.residue_sessions += 1;
            stats.residue_files += residue_files;
            stats.residue_bytes += residue_bytes;
            sink.emit(&Event::Hint(Hint::SessionResidue {
                dir: session_dir.clone(),
                files: residue_files,
                bytes: residue_bytes,
            }));
        } else {
            // All staged files were committed; drop the now-empty staging
            // subtree (plan.json / manifest.json stay). Empty only — files
            // that failed to rename keep the subtree in place.
            let _ = std::fs::remove_dir_all(&staging);
        }
    }

    if stats.completed > 0 {
        sink.emit(&Event::Hint(Hint::StagingReconciled {
            completed: stats.completed,
        }));
    }
    stats
}

/// Recursively collect all files under `dir` (read errors are skipped).
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::insert::{InsertOptions, batch_insert};
    use crate::pipeline::types::{FileHash, HashResult};
    use std::sync::Mutex;

    /// A sink that records every event for assertions.
    #[derive(Debug, Default)]
    struct RecordingSink(Mutex<Vec<Event>>);

    impl EventSink for RecordingSink {
        fn emit(&self, event: &Event) {
            self.0.lock().unwrap().push(event.clone());
        }
    }

    impl RecordingSink {
        fn hints(&self) -> Vec<Hint> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter_map(|e| match e {
                    Event::Hint(h) => Some(h.clone()),
                    _ => None,
                })
                .collect()
        }
    }

    /// Insert one DB record for `rel` via the real Stage-E code path.
    fn insert_record(vault_root: &Path, db: &Db, rel: &str) {
        let results = vec![HashResult {
            path: vault_root.join(rel),
            src_path: None,
            staged_path: None,
            size: 3,
            mtime_ms: 0,
            crc32c: 7,
            raw_unique_id: None,
            hash: FileHash::Fast(vec![1, 2, 3]),
            is_duplicate: false,
            dup_reason: None,
            hash_error: None,
        }];
        let opts = InsertOptions {
            vault_root,
            session_id: "s1",
            write_manifest: false,
            source_root: None,
            force: false,
            session_type: SessionType::Import,
        };
        let summary = batch_insert(results, db, opts, None).unwrap();
        assert_eq!(summary.added, 1);
    }

    /// Create `<vault>/sessions/import/<id>/staging/<rel>` with `content`.
    fn staged_file(vault: &Path, session_id: &str, rel: &str, content: &[u8]) -> PathBuf {
        let dir = session_dir(vault, SessionType::Import, session_id);
        let path = staging_dir(&dir).join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn session_paths_follow_the_journal_layout() {
        let vault = Path::new("/vault");
        let dir = session_dir(vault, SessionType::Import, "20260809T153012-a1b2c");
        assert_eq!(
            dir,
            Path::new("/vault/.svault/sessions/import/20260809T153012-a1b2c")
        );
        assert_eq!(
            staged_path_for(&dir, vault, Path::new("/vault/2024/photo.jpg")),
            Path::new(
                "/vault/.svault/sessions/import/20260809T153012-a1b2c/staging/2024/photo.jpg"
            )
        );
    }

    #[test]
    fn write_json_atomic_roundtrips_and_replaces() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested/plan.json");

        write_json_atomic(&path, &serde_json::json!({"v": 1})).unwrap();
        write_json_atomic(&path, &serde_json::json!({"v": 2})).unwrap();

        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["v"], 2);
        // No temp residue left behind.
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn reconcile_completes_rename_when_db_record_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let db = Db::open_in_memory().unwrap();

        // Simulate: DB commit succeeded, process died before rename.
        insert_record(vault, &db, "2024/photo.jpg");
        let staged = staged_file(vault, "s1", "2024/photo.jpg", b"abc");

        let stats = reconcile(vault, &db, &RecordingSink::default());

        assert_eq!(stats.completed, 1);
        assert_eq!(stats.residue_files, 0);
        assert!(!staged.exists(), "staged file must be moved out");
        assert_eq!(
            std::fs::read(vault.join("2024/photo.jpg")).unwrap(),
            b"abc"
        );
        // Empty staging subtree is removed; the session dir itself stays.
        let dir = session_dir(vault, SessionType::Import, "s1");
        assert!(!staging_dir(&dir).exists());
        assert!(dir.exists());
    }

    #[test]
    fn reconcile_reports_unrecorded_residue_without_deleting() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let db = Db::open_in_memory().unwrap();
        let sink = RecordingSink::default();

        // Simulate: killed mid-copy, nothing committed to the DB.
        let staged = staged_file(vault, "s1", "2024/partial.jpg", b"ab");

        let stats = reconcile(vault, &db, &sink);

        assert_eq!(stats.completed, 0);
        assert_eq!(stats.residue_files, 1);
        assert_eq!(stats.residue_bytes, 2);
        assert_eq!(stats.residue_sessions, 1);
        // Reported, NOT deleted (user reviews plan.json and decides).
        assert!(staged.exists());
        assert!(!vault.join("2024/partial.jpg").exists());
        assert!(sink
            .hints()
            .iter()
            .any(|h| matches!(h, Hint::SessionResidue { files: 1, bytes: 2, .. })));
    }

    #[test]
    fn reconcile_reports_staged_copy_when_destination_already_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let db = Db::open_in_memory().unwrap();

        // Simulate: rename completed, then the process died before the
        // staging subtree was cleaned up.
        insert_record(vault, &db, "2024/photo.jpg");
        let dest = vault.join("2024/photo.jpg");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"abc").unwrap();
        let staged = staged_file(vault, "s1", "2024/photo.jpg", b"abc");

        let stats = reconcile(vault, &db, &RecordingSink::default());

        assert_eq!(stats.completed, 0);
        assert_eq!(stats.residue_files, 1);
        assert!(staged.exists(), "residue is reported, never deleted");
        assert_eq!(std::fs::read(&dest).unwrap(), b"abc");
    }

    #[test]
    fn reconcile_without_sessions_area_is_a_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Db::open_in_memory().unwrap();
        let stats = reconcile(tmp.path(), &db, &RecordingSink::default());
        assert_eq!(stats, ReconcileStats::default());
    }
}
