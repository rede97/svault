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

use crate::verify::manifest::SessionType;

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

#[cfg(test)]
mod tests {
    use super::*;

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
            Path::new("/vault/.svault/sessions/import/20260809T153012-a1b2c/staging/2024/photo.jpg")
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
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }
}
