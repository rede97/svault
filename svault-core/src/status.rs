//! Vault status report — pure data, no rendering.
//!
//! This is a **pull-model** query (see `docs/ARCHITECTURE.md` §2.2): core
//! returns a serializable [`StatusReport`]; formatting (terminal tables or
//! JSON) is the responsibility of the presentation layer (`svault-ui`).

use std::path::Path;

use serde::Serialize;

use crate::db::{Db, ExtensionStats, VaultStats};

/// Status report for a vault.
#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    /// Path to the vault root.
    pub vault_root: std::path::PathBuf,
    /// Database file path.
    pub db_path: std::path::PathBuf,
    /// Overall statistics.
    pub stats: VaultStats,
    /// Top file extensions by size.
    pub top_extensions: Vec<ExtensionStats>,
    /// Files imported in the last 24 hours.
    pub imports_last_24h: i64,
    /// Files imported in the last 7 days.
    pub imports_last_7d: i64,
    /// Files imported in the last 30 days.
    pub imports_last_30d: i64,
    /// Interrupted operation sessions (import/sync dirs without a manifest),
    /// with their staging residue. Empty when every session completed.
    pub incomplete_sessions: Vec<crate::session::IncompleteSession>,
    /// Git-style working-tree status (see [`WorkingTreeStatus`]).
    pub working_tree: WorkingTreeStatus,
}

/// Git-style working-tree status: disk vs DB divergence.
///
/// Detection strategy keeps `status` cheap: files whose path is in the DB
/// with a matching size are assumed unchanged and never re-hashed; only
/// on-disk files with no DB record at their path are hashed (full XXH3-128)
/// to distinguish moves from genuinely untracked content.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct WorkingTreeStatus {
    /// On disk, not in the DB (not yet `add`ed / imported at that path).
    pub untracked: Vec<String>,
    /// `(old_db_path, new_disk_path)`: DB record's file vanished, but the
    /// same content hash was found at another path (what `update` fixes).
    pub moved: Vec<(String, String)>,
    /// DB record exists but the file is gone from disk and its content was
    /// not found elsewhere.
    pub missing: Vec<String>,
    /// Path is in the DB but the on-disk size differs (content changed
    /// in place; size change proves it without hashing).
    pub modified: Vec<String>,
}

/// Options for generating a status report.
#[derive(Debug, Clone)]
pub struct StatusOptions {
    /// Number of top extensions to show.
    pub top_extensions_limit: i64,
}

/// Which working-tree categories to display (all four = default when unset).
#[derive(Debug, Clone, Copy, Default)]
pub struct WorkingTreeFilter {
    pub untracked: bool,
    pub moved: bool,
    pub missing: bool,
    pub modified: bool,
}

impl WorkingTreeFilter {
    /// True when no category was explicitly selected (show everything).
    pub fn is_default(&self) -> bool {
        !self.untracked && !self.moved && !self.missing && !self.modified
    }

    fn show(&self, category: bool) -> bool {
        self.is_default() || category
    }

    pub fn show_untracked(&self) -> bool {
        self.show(self.untracked)
    }
    pub fn show_moved(&self) -> bool {
        self.show(self.moved)
    }
    pub fn show_missing(&self) -> bool {
        self.show(self.missing)
    }
    pub fn show_modified(&self) -> bool {
        self.show(self.modified)
    }
}

impl Default for StatusOptions {
    fn default() -> Self {
        Self {
            top_extensions_limit: 10,
        }
    }
}

/// Generates a status report for the vault at `vault_root`.
pub fn generate_report(
    vault_root: &Path,
    db: &Db,
    opts: StatusOptions,
) -> anyhow::Result<StatusReport> {
    let stats = db.vault_stats()?;
    let top_extensions = db.extension_stats(opts.top_extensions_limit)?;

    let imports_last_24h = db.recent_imports(1)?;
    let imports_last_7d = db.recent_imports(7)?;
    let imports_last_30d = db.recent_imports(30)?;
    let incomplete_sessions = crate::session::find_incomplete_sessions(vault_root);
    let working_tree = compute_working_tree(vault_root, db);

    Ok(StatusReport {
        vault_root: vault_root.to_path_buf(),
        db_path: vault_root.join(".svault").join("vault.db"),
        stats,
        top_extensions,
        imports_last_24h,
        imports_last_7d,
        imports_last_30d,
        incomplete_sessions,
        working_tree,
    })
}

/// Compute the git-style working-tree status by comparing the vault's
/// on-disk files against the DB.
///
/// Cost control: files at a DB path with matching size are never re-hashed
/// (git's stat-cache shortcut). Only unknown paths are hashed (full
/// XXH3-128) to distinguish `moved` from `untracked`.
pub fn compute_working_tree(vault_root: &Path, db: &Db) -> WorkingTreeStatus {
    use std::collections::{HashMap, HashSet};

    let mut status = WorkingTreeStatus::default();
    let rows = match db.get_all_files() {
        Ok(r) => r,
        Err(_) => return status,
    };

    // path -> size; identity hash -> row paths (a hash may have several rows)
    let mut size_by_path: HashMap<&str, i64> = HashMap::new();
    let mut paths_by_hash: HashMap<Vec<u8>, Vec<&str>> = HashMap::new();
    for row in &rows {
        size_by_path.insert(row.path.as_str(), row.size);
        if let Some(h) = row.sha256.clone().or_else(|| row.xxh3_128.clone()) {
            paths_by_hash.entry(h).or_default().push(row.path.as_str());
        }
    }

    let rx = match crate::fs::walk_stream(
        vault_root,
        Path::new(""),
        &[],
        &crate::fs::ScanFilter::default(),
    ) {
        Ok(rx) => rx,
        Err(_) => return status,
    };

    // Pass 1: collect the disk state (two passes avoid depending on walk order).
    let mut disk: Vec<(String, u64)> = Vec::new();
    for entry in rx.into_iter().flatten() {
        let rel = entry.path.to_string_lossy().replace('\\', "/");
        if rel == crate::config::CONFIG_FILE {
            continue;
        }
        disk.push((rel, entry.size));
    }
    let disk_paths: HashSet<&str> = disk.iter().map(|(p, _)| p.as_str()).collect();

    // Pass 2: categorize.
    let mut matched_old_paths: HashSet<&str> = HashSet::new();
    for (rel, size) in &disk {
        match size_by_path.get(rel.as_str()) {
            Some(db_size) if *db_size == *size as i64 => {} // unchanged
            Some(_) => status.modified.push(rel.clone()),   // same path, size changed
            None => {
                // Unknown path: hash it, then look for a DB row whose file is
                // gone from its recorded path (a move). A hash that only
                // matches rows still present on disk is duplicate content —
                // reported as untracked, not moved.
                let abs = vault_root.join(rel);
                let identity = crate::hash::xxh3_128_file(&abs)
                    .ok()
                    .map(|d| d.to_bytes().to_vec());
                let moved_from = identity.as_ref().and_then(|h| {
                    paths_by_hash.get(h).and_then(|candidates| {
                        candidates
                            .iter()
                            .find(|p| !disk_paths.contains(**p) && !matched_old_paths.contains(**p))
                            .copied()
                    })
                });
                match moved_from {
                    Some(old) => {
                        matched_old_paths.insert(old);
                        status.moved.push((old.to_string(), rel.clone()));
                    }
                    None => status.untracked.push(rel.clone()),
                }
            }
        }
    }

    // DB rows never seen on disk and not matched by a moved file → missing.
    for row in &rows {
        if !disk_paths.contains(row.path.as_str()) && !matched_old_paths.contains(row.path.as_str())
        {
            status.missing.push(row.path.clone());
        }
    }

    status.untracked.sort();
    status.moved.sort();
    status.missing.sort();
    status.modified.sort();
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vault dir with files + matching DB rows.
    fn setup() -> (tempfile::TempDir, Db) {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let db = Db::open_in_memory().unwrap();

        // tracked + on disk, unchanged
        std::fs::create_dir_all(vault.join("2026")).unwrap();
        std::fs::write(vault.join("2026/same.jpg"), b"same").unwrap();
        let h = crate::hash::xxh3_128_bytes(b"same").to_bytes();
        db.insert_file_row(
            "2026/same.jpg",
            4,
            0,
            None,
            None,
            Some(&h),
            None,
            "imported",
            0,
        )
        .unwrap();
        (tmp, db)
    }

    #[test]
    fn working_tree_categories() {
        let (tmp, db) = setup();
        let vault = tmp.path();

        // moved: DB row (imported) whose file is gone, content at new path
        let moved_hash = crate::hash::xxh3_128_bytes(b"moved-content").to_bytes();
        db.insert_file_row(
            "2026/old.jpg",
            13,
            0,
            None,
            None,
            Some(&moved_hash),
            None,
            "imported",
            0,
        )
        .unwrap();
        std::fs::write(vault.join("2026/new-name.jpg"), b"moved-content").unwrap();

        // missing: DB row, file gone, content nowhere
        db.insert_file_row(
            "2026/gone.jpg",
            9,
            0,
            None,
            None,
            Some(&[9u8; 16]),
            None,
            "imported",
            0,
        )
        .unwrap();

        // untracked: on disk, hash unknown to DB
        std::fs::write(vault.join("2026/stray.jpg"), b"stray").unwrap();

        // modified: path in DB, size differs on disk
        db.insert_file_row(
            "2026/edited.jpg",
            100,
            0,
            None,
            None,
            Some(&[3u8; 16]),
            None,
            "imported",
            0,
        )
        .unwrap();
        std::fs::write(vault.join("2026/edited.jpg"), b"short").unwrap();

        // svault.toml must never appear as untracked
        std::fs::write(vault.join("svault.toml"), b"# config").unwrap();

        let wt = compute_working_tree(vault, &db);

        assert_eq!(wt.untracked, vec!["2026/stray.jpg".to_string()]);
        assert_eq!(
            wt.moved,
            vec![("2026/old.jpg".to_string(), "2026/new-name.jpg".to_string())]
        );
        assert_eq!(wt.missing, vec!["2026/gone.jpg".to_string()]);
        assert_eq!(wt.modified, vec!["2026/edited.jpg".to_string()]);
    }

    #[test]
    fn clean_vault_reports_nothing() {
        let (tmp, db) = setup();
        let wt = compute_working_tree(tmp.path(), &db);
        assert_eq!(wt, WorkingTreeStatus::default());
    }
}
