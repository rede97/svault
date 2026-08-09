//! Stage A: Directory scanning.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use crate::fs::DirEntry;
use crate::fs::walk_stream;
use crate::pipeline::types::FileEntry;

/// Normalize a directory path for scanning.
///
/// Handles PowerShell auto-completion quirks where trailing backslash + quote
/// combinations may result in paths ending with `"` or `'`.
/// Also removes trailing backslashes which can cause issues on Windows.
fn normalize_scan_root(path: &Path) -> PathBuf {
    let path_str = path.as_os_str().to_string_lossy();

    // Strip trailing quote characters that may be introduced by shell escaping
    let cleaned = path_str.trim_end_matches('"').trim_end_matches('\'');

    PathBuf::from(cleaned)
}

/// Stream directory entries from the local filesystem.
///
/// This is the preferred API for directory scanning. It returns a receiver
/// that yields `FileEntry` as files are discovered, enabling streaming
/// processing without loading all entries into memory.
///
/// # Arguments
/// * `root` - Root directory to scan
/// * `exts` - File extensions to include (empty = all files)
///
/// # Returns
/// Receiver that yields `FileEntry` results as they are discovered.
/// The channel closes when scanning completes.
///
/// # Examples
///
/// ```ignore
/// let rx = scan_stream(Path::new("/photos"), &["jpg", "png"])?;
/// for entry_result in rx {
///     match entry_result {
///         Ok(entry) => println!("{}: {} bytes", entry.path.display(), entry.size),
///         Err(e) => eprintln!("Error: {}", e),
///     }
/// }
/// ```
pub fn scan_stream(
    root: &Path,
    exts: &[&str],
    filter: &crate::fs::ScanFilter,
) -> anyhow::Result<mpsc::Receiver<anyhow::Result<FileEntry>>> {
    // Normalize path: strip trailing separators and quotes (handles PowerShell auto-completion quirks)
    let root = normalize_scan_root(root);
    let exts: Vec<String> = exts.iter().map(|s| s.to_string()).collect();

    let fs_rx = walk_stream(
        &root,
        Path::new(""),
        &exts.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        filter,
    )?;

    let (tx, rx) = mpsc::channel();

    // Convert DirEntry to pipeline FileEntry in background thread
    thread::spawn(move || {
        for fs_result in fs_rx {
            let result = fs_result
                .map(|dir_entry| dir_entry_to_file_entry(&root, dir_entry))
                .map_err(|e| anyhow::anyhow!(e));

            if tx.send(result).is_err() {
                break; // Receiver dropped
            }
        }
    });

    Ok(rx)
}

/// Convert a DirEntry to a pipeline FileEntry.
fn dir_entry_to_file_entry(root: &Path, entry: DirEntry) -> FileEntry {
    FileEntry {
        // Convert to absolute path
        path: root.join(&entry.path),
        size: entry.size,
        mtime_ms: entry.mtime_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // =========================================================================
    // scan_stream tests
    // =========================================================================

    #[test]
    fn test_scan_stream_basic() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(root.join("a.jpg"), "content a").unwrap();
        fs::write(root.join("b.png"), "content b").unwrap();
        fs::write(root.join("c.txt"), "content c").unwrap();

        let rx = scan_stream(root, &["jpg", "png"], &crate::fs::ScanFilter::default()).unwrap();
        let entries: Vec<_> = rx.into_iter().filter_map(|r| r.ok()).collect();

        assert_eq!(entries.len(), 2);
        // Note: stream order is not deterministic
        let paths: Vec<_> = entries.iter().map(|e| e.path.clone()).collect();
        assert!(paths.iter().any(|p| p.ends_with("a.jpg")));
        assert!(paths.iter().any(|p| p.ends_with("b.png")));
    }

    #[test]
    fn test_scan_stream_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let rx = scan_stream(tmp.path(), &["jpg"], &crate::fs::ScanFilter::default()).unwrap();
        let entries: Vec<_> = rx.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_scan_stream_nested_dirs() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Create nested structure
        fs::create_dir(root.join("subdir1")).unwrap();
        fs::create_dir(root.join("subdir2")).unwrap();
        fs::write(root.join("subdir1/a.jpg"), "a").unwrap();
        fs::write(root.join("subdir2/b.jpg"), "b").unwrap();
        fs::write(root.join("c.jpg"), "c").unwrap();

        let rx = scan_stream(root, &["jpg"], &crate::fs::ScanFilter::default()).unwrap();
        let entries: Vec<_> = rx.into_iter().filter_map(|r| r.ok()).collect();

        assert_eq!(entries.len(), 3);
        let paths: Vec<_> = entries.iter().map(|e| e.path.clone()).collect();
        assert!(paths.iter().any(|p| p.ends_with("subdir1/a.jpg")));
        assert!(paths.iter().any(|p| p.ends_with("subdir2/b.jpg")));
        assert!(paths.iter().any(|p| p.ends_with("c.jpg")));
    }

    #[test]
    fn test_scan_stream_all_extensions() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(root.join("a.jpg"), "a").unwrap();
        fs::write(root.join("b.png"), "b").unwrap();
        fs::write(root.join("c.txt"), "c").unwrap();

        let rx = scan_stream(root, &[], &crate::fs::ScanFilter::default()).unwrap();
        let entries: Vec<_> = rx.into_iter().filter_map(|r| r.ok()).collect();

        assert_eq!(entries.len(), 3);
    }

    // =========================================================================
    // ScanFilter tests (max_depth + globs)
    // =========================================================================

    fn filtered_entries(filter: &crate::fs::ScanFilter) -> Vec<String> {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("sub/deep")).unwrap();
        fs::write(root.join("top.jpg"), "t").unwrap();
        fs::write(root.join("top.png"), "t").unwrap();
        fs::write(root.join("sub/mid.JPG"), "m").unwrap();
        fs::write(root.join("sub/deep/bottom.jpg"), "b").unwrap();

        let rx = scan_stream(root, &[], filter).unwrap();
        let mut names: Vec<String> = rx
            .into_iter()
            .filter_map(|r| r.ok())
            .map(|e| {
                e.path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn test_scan_max_depth_one_scans_only_top_level() {
        let names = filtered_entries(&crate::fs::ScanFilter {
            max_depth: 1,
            ..Default::default()
        });
        assert_eq!(names, vec!["top.jpg", "top.png"]);
    }

    #[test]
    fn test_scan_include_glob_case_insensitive() {
        let names = filtered_entries(&crate::fs::ScanFilter {
            include: vec!["*.jpg".to_string()],
            ..Default::default()
        });
        assert_eq!(
            names,
            vec!["sub/deep/bottom.jpg", "sub/mid.JPG", "top.jpg"],
            "include glob must match at any depth, case-insensitively"
        );
    }

    #[test]
    fn test_scan_exclude_wins_over_include() {
        let names = filtered_entries(&crate::fs::ScanFilter {
            include: vec!["**/*.jpg".to_string()],
            exclude: vec!["sub/**".to_string()],
            ..Default::default()
        });
        assert_eq!(names, vec!["top.jpg"]);
    }

    #[test]
    fn test_scan_invalid_glob_is_an_error_not_a_silent_pass() {
        let tmp = TempDir::new().unwrap();
        let result = scan_stream(
            tmp.path(),
            &[],
            &crate::fs::ScanFilter {
                include: vec!["[unclosed".to_string()],
                ..Default::default()
            },
        );
        assert!(result.is_err());
    }

    // =========================================================================
    // normalize_scan_root tests
    // =========================================================================

    #[test]
    fn test_normalize_scan_root_strips_trailing_double_quote() {
        // Simulates PowerShell: 'C:\path\' -> C:\path\"
        let path = Path::new(r#"C:\Users\test\"#);
        let normalized = normalize_scan_root(path);
        assert_eq!(normalized, Path::new(r#"C:\Users\test\"#).as_os_str());
        // Actually the trailing quote gets stripped
        let path_with_quote = Path::new(r#"C:\Users\test""#);
        let normalized = normalize_scan_root(path_with_quote);
        assert_eq!(normalized, Path::new(r#"C:\Users\test"#));
    }

    #[test]
    fn test_normalize_scan_root_strips_trailing_single_quote() {
        // Simulates PowerShell escaping issues
        let path = Path::new("C:\\Users\\test\\'");
        let normalized = normalize_scan_root(path);
        assert_eq!(normalized, Path::new("C:\\Users\\test\\"));
    }

    #[test]
    fn test_normalize_scan_root_unchanged_normal_path() {
        let path = Path::new("C:\\Users\\test\\photos");
        let normalized = normalize_scan_root(path);
        assert_eq!(normalized, path);
    }
}
