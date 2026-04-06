//! Stage A: Directory scanning.

use std::path::Path;

use crate::pipeline::types::FileEntry;
use crate::vfs::system::SystemFs;
use crate::vfs::VfsBackend;

/// Scan a directory for files with given extensions.
///
/// # Arguments
/// * `root` - Directory to scan
/// * `exts` - Allowed extensions (empty = all)
/// * `exclude_vault` - If true, filter out paths under vault_root
/// * `vault_root` - Vault root path (used for filtering)
///
/// # Returns
/// List of file entries sorted by path for deterministic order.
pub fn scan_files(
    root: &Path,
    exts: &[&str],
    exclude_vault: bool,
    vault_root: Option<&Path>,
) -> anyhow::Result<Vec<FileEntry>> {
    let fs = SystemFs::open(root)?;
    let entries = fs.walk(Path::new(""), exts)?;

    let vault_canon = vault_root
        .and_then(|p| std::fs::canonicalize(p).ok());

    let mut result: Vec<FileEntry> = entries
        .into_iter()
        .filter_map(|e| {
            // Filter out vault paths if requested
            // e.path is relative to root, convert to absolute for comparison
            if exclude_vault {
                if let Some(ref v) = vault_canon {
                    let abs_path = root.join(&e.path);
                    if abs_path.starts_with(v) {
                        return None;
                    }
                }
            }

            Some(FileEntry {
                path: e.path,
                size: e.size,
                mtime_ms: e.mtime_ms,
            })
        })
        .collect();

    // Sort by path for deterministic order
    result.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(result)
}

/// Scan files without vault filtering (for add command).
pub fn scan_files_simple(root: &Path, exts: &[&str]) -> anyhow::Result<Vec<FileEntry>> {
    scan_files(root, exts, false, None)
}
