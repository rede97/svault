//! Stage A: Directory scanning.

use std::path::Path;

use crate::pipeline::types::FileEntry;
use crate::vfs::system::SystemFs;
use crate::vfs::VfsBackend;

/// Scan a directory for files with given extensions.
///
/// Returns entries with absolute paths (root.join(relative_path)).
/// Paths are sorted for deterministic order.
///
/// # Example
/// ```ignore
/// let entries = scan_files(Path::new("/photos"), &["jpg", "png"])?;
/// for e in entries {
///     println!("{}: {} bytes", e.path.display(), e.size);
/// }
/// ```
pub fn scan_files(root: &Path, exts: &[&str]) -> anyhow::Result<Vec<FileEntry>> {
    let fs = SystemFs::open(root)?;
    let vfs_entries = fs.walk(Path::new(""), exts)?;

    let mut result: Vec<FileEntry> = vfs_entries
        .into_iter()
        .map(|e| FileEntry {
            // Convert to absolute path
            path: root.join(&e.path),
            size: e.size,
            mtime_ms: e.mtime_ms,
        })
        .collect();

    // Sort by path for deterministic order
    result.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_scan_files_basic() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Create test files
        fs::write(root.join("a.jpg"), "content a").unwrap();
        fs::write(root.join("b.png"), "content b").unwrap();
        fs::write(root.join("c.txt"), "content c").unwrap();

        let entries = scan_files(root, &["jpg", "png"]).unwrap();

        assert_eq!(entries.len(), 2);
        assert!(entries[0].path.ends_with("a.jpg"));
        assert!(entries[1].path.ends_with("b.png"));
    }

    #[test]
    fn test_scan_files_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let entries = scan_files(tmp.path(), &["jpg"]).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_scan_files_all_extensions() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(root.join("a.jpg"), "a").unwrap();
        fs::write(root.join("b.png"), "b").unwrap();
        fs::write(root.join("c.txt"), "c").unwrap();

        // Empty extension list = all files
        let entries = scan_files(root, &[]).unwrap();
        assert_eq!(entries.len(), 3);
    }
}
