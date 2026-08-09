//! Import staging area for crash-safe imports.
//!
//! `import` copies each file into `.svault/staging/import/<session_id>/`
//! first (mirroring the final relative path), fsyncs it, hashes the staged
//! copy, and records it in the DB. Only after the DB transaction commits is
//! the staged file atomically renamed to its final destination
//! ([`crate::fs::atomic_commit`]). This yields the invariant:
//!
//! **A file visible at its final vault path is always fully copied, hashed,
//! and DB-recorded.** Partially copied or unrecorded files never appear in
//! the user-visible vault tree — they stay inside `.svault/staging/`.
//!
//! Leftovers from an interrupted import are reconciled at the start of the
//! next import ([`reconcile`]):
//!
//! - staged file whose final path has an `imported` DB record → the process
//!   died between commit and rename; the rename is completed now;
//! - anything else → svault-internal residue (killed mid-copy, hash failure,
//!   Stage-D duplicate); it is purged.
//!
//! Purging staging residue does not violate the never-delete-user-files
//! rule: these files were created by svault itself inside `.svault/`, and
//! the user's source files are never touched.

use std::fs;
use std::path::{Path, PathBuf};

use crate::db::Db;
use crate::event::{Event, EventSink, Hint};

/// Root of the import staging area inside a vault.
pub fn staging_root(vault_root: &Path) -> PathBuf {
    vault_root.join(".svault").join("staging").join("import")
}

/// Staging directory of one import session.
pub fn session_dir(vault_root: &Path, session_id: &str) -> PathBuf {
    staging_root(vault_root).join(session_id)
}

/// Map a final vault destination to its staged path within `session_dir`.
pub fn staged_path_for(session_dir: &Path, vault_root: &Path, dest: &Path) -> PathBuf {
    let rel = dest.strip_prefix(vault_root).unwrap_or(dest);
    session_dir.join(rel)
}

/// Outcome of [`reconcile`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReconcileStats {
    /// Staged files whose DB record existed; the pending rename was finished.
    pub completed: usize,
    /// Staged residue without a DB record; purged.
    pub purged: usize,
}

/// Reconcile staging leftovers from an interrupted import.
///
/// Best-effort: per-file errors skip that file (it is retried on the next
/// run) and never abort the import. Emits [`Hint::StagingReconciled`] when
/// anything was done.
pub fn reconcile(vault_root: &Path, db: &Db, sink: &dyn EventSink) -> ReconcileStats {
    let root = staging_root(vault_root);
    let mut stats = ReconcileStats::default();
    if !root.is_dir() {
        return stats;
    }

    let sessions = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(_) => return stats,
    };

    for session_entry in sessions.flatten() {
        let session_dir = session_entry.path();
        if !session_dir.is_dir() {
            continue;
        }

        let mut staged_files = Vec::new();
        collect_files(&session_dir, &mut staged_files);

        let mut had_error = false;
        for staged in &staged_files {
            let Ok(rel) = staged.strip_prefix(&session_dir) else {
                had_error = true;
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
                match crate::fs::atomic_commit(staged, &dest) {
                    Ok(()) => stats.completed += 1,
                    Err(_) => had_error = true,
                }
            } else {
                // No DB record (killed before commit) or destination already
                // present: svault-internal residue — purge it.
                match fs::remove_file(staged) {
                    Ok(()) => stats.purged += 1,
                    Err(_) => had_error = true,
                }
            }
        }

        // Everything processed was either moved out or purged; drop the
        // (now empty) session tree unless an error left files behind.
        if !had_error {
            let _ = fs::remove_dir_all(&session_dir);
        }
    }

    // Best-effort: drop the staging root itself when nothing remains.
    let _ = fs::remove_dir(&root);

    if stats.completed > 0 || stats.purged > 0 {
        sink.emit(&Event::Hint(Hint::StagingReconciled {
            completed: stats.completed,
            purged: stats.purged,
        }));
    }
    stats
}

/// Recursively collect all files under `dir` (read errors are skipped).
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
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
    use crate::event::NoopSink;
    use crate::pipeline::types::{FileHash, HashResult};
    use crate::pipeline::insert::{InsertOptions, batch_insert};
    use crate::verify::manifest::SessionType;

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

    #[test]
    fn reconcile_completes_rename_when_db_record_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let db = Db::open_in_memory().unwrap();

        // Simulate: DB commit succeeded, process died before rename.
        insert_record(vault, &db, "2024/photo.jpg");
        let staged = session_dir(vault, "s1").join("2024/photo.jpg");
        fs::create_dir_all(staged.parent().unwrap()).unwrap();
        fs::write(&staged, b"abc").unwrap();

        let stats = reconcile(vault, &db, &NoopSink);

        assert_eq!(stats.completed, 1);
        assert_eq!(stats.purged, 0);
        assert!(!staged.exists(), "staged file must be moved out");
        assert_eq!(fs::read(vault.join("2024/photo.jpg")).unwrap(), b"abc");
        assert!(
            !staging_root(vault).exists(),
            "empty staging area must be removed"
        );
    }

    #[test]
    fn reconcile_purges_unrecorded_residue() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let db = Db::open_in_memory().unwrap();

        // Simulate: killed mid-copy, nothing committed to the DB.
        let staged = session_dir(vault, "s1").join("2024/partial.jpg");
        fs::create_dir_all(staged.parent().unwrap()).unwrap();
        fs::write(&staged, b"ab").unwrap();

        let stats = reconcile(vault, &db, &NoopSink);

        assert_eq!(stats.completed, 0);
        assert_eq!(stats.purged, 1);
        assert!(!staged.exists());
        assert!(!vault.join("2024/partial.jpg").exists());
        assert!(!staging_root(vault).exists());
    }

    #[test]
    fn reconcile_purges_staged_copy_when_destination_already_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let db = Db::open_in_memory().unwrap();

        // Simulate: rename completed, then the process died before the
        // staging dir was cleaned up.
        insert_record(vault, &db, "2024/photo.jpg");
        let dest = vault.join("2024/photo.jpg");
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(&dest, b"abc").unwrap();
        let staged = session_dir(vault, "s1").join("2024/photo.jpg");
        fs::create_dir_all(staged.parent().unwrap()).unwrap();
        fs::write(&staged, b"abc").unwrap();

        let stats = reconcile(vault, &db, &NoopSink);

        assert_eq!(stats.completed, 0);
        assert_eq!(stats.purged, 1);
        assert!(!staged.exists());
        assert_eq!(fs::read(&dest).unwrap(), b"abc");
    }

    #[test]
    fn reconcile_without_staging_area_is_a_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Db::open_in_memory().unwrap();
        let stats = reconcile(tmp.path(), &db, &NoopSink);
        assert_eq!(stats, ReconcileStats::default());
    }

    #[test]
    fn staged_path_mirrors_final_relative_path() {
        let vault = Path::new("/vault");
        let session = session_dir(vault, "42");
        let dest = Path::new("/vault/2024/01-01/photo.jpg");
        assert_eq!(
            staged_path_for(&session, vault, dest),
            Path::new("/vault/.svault/staging/import/42/2024/01-01/photo.jpg")
        );
    }
}
