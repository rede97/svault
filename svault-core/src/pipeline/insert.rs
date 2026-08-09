//! Stage E: Batch DB insertion.

use std::path::Path;

use crate::db::Db;
use crate::pipeline::types::{FileHash, HashResult, PipelineSummary};
use crate::verify::manifest::{
    ImportManifest, ImportRecord, ItemStatus, ManifestManager, ManifestSummary, SessionType,
};

/// Convert a path to Unix-style string (forward slashes) for cross-platform storage.
///
/// On Windows, paths use backslash separators which are incompatible with Linux.
/// We store all paths with forward slashes to ensure the database is portable
/// between Windows and Linux.
fn path_to_unix_string(path: &Path) -> String {
    // First, get the path as a string, replacing any backslashes with forward slashes
    // This handles Windows paths that may contain backslashes
    let path_str = path.to_string_lossy();
    let normalized = path_str.replace('\\', "/");

    // Remove leading slash if present (from absolute paths)
    normalized
        .strip_prefix('/')
        .map(String::from)
        .unwrap_or(normalized)
}

/// Options for batch insertion.
pub struct InsertOptions<'a> {
    pub vault_root: &'a Path,
    pub session_id: &'a str,
    /// Whether to write manifest (import: true, add: false)
    pub write_manifest: bool,
    /// Source root (for manifest, import only)
    pub source_root: Option<&'a Path>,
    /// Force mode - overwrite existing files
    pub force: bool,
    /// Session type for manifest
    pub session_type: SessionType,
}

/// Convert hash bytes to hex string for manifest.
fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Insert all valid entries into DB using batch transaction.
///
/// Records all files (added/duplicate/failed/skipped) to manifest for history.
pub fn batch_insert(
    results: Vec<HashResult>,
    db: &Db,
    opts: InsertOptions,
    progress_cb: Option<&dyn Fn()>,
) -> anyhow::Result<PipelineSummary> {
    let mut summary = PipelineSummary::new(results.len());
    let now_ms = crate::ops::utils::unix_now_ms();

    // Prepare manifest if needed
    let mut manifest = if opts.write_manifest {
        opts.source_root.map(|root| ImportManifest {
            session_id: opts.session_id.to_string(),
            session_type: opts.session_type,
            source_root: root.to_path_buf(),
            imported_at: now_ms,
            hash_algorithm: "xxh3_128".to_string(),
            files: Vec::with_capacity(results.len()),
            summary: None,
        })
    } else {
        None
    };

    // Collect files for batch insert
    let mut files_to_insert: Vec<HashResult> = Vec::with_capacity(results.len());

    for r in results {
        if let Some(cb) = progress_cb {
            cb();
        }

        let rel_path = r.path.strip_prefix(opts.vault_root).unwrap_or(&r.path);
        // Use Unix-style paths for cross-platform database compatibility
        let rel_str = path_to_unix_string(rel_path);
        let src_path = r.src_path.clone().unwrap_or_else(|| r.path.clone());

        // Get hashes early for manifest recording
        let (xxh3_hex, sha256_hex) = match &r.hash {
            FileHash::Fast(xxh3) => (Some(bytes_to_hex(xxh3)), None),
            FileHash::Full(xxh3, sha256) => (Some(bytes_to_hex(xxh3)), Some(bytes_to_hex(sha256))),
        };

        // Skip if already tracked by path (unless force mode or the existing file is 'missing')
        // 'missing' files should be allowed to recover (re-import with same path)
        if !opts.force
            && let Ok(Some(existing)) = db.get_file_by_path(&rel_str)
            && existing.status != "missing"
        {
            summary.skipped += 1;
            // Record skipped file to manifest
            if let Some(ref mut m) = manifest {
                m.files.push(ImportRecord {
                    src_path,
                    dest_path: Some(rel_path.to_path_buf()),
                    size: r.size,
                    mtime_ms: r.mtime_ms,
                    fingerprint: crate::pipeline::insert::bytes_to_hex(&r.fingerprint),
                    xxh3_128: xxh3_hex,
                    sha256: sha256_hex,
                    imported_at: now_ms,
                    status: ItemStatus::Skipped,
                    error: None,
                });
            }
            continue;
        }

        // Handle hash computation errors (IO errors while reading the vault copy)
        if let Some(reason) = &r.hash_error {
            summary.failed += 1;
            // Record failed file to manifest
            if let Some(ref mut m) = manifest {
                m.files.push(ImportRecord {
                    src_path,
                    dest_path: None,
                    size: r.size,
                    mtime_ms: r.mtime_ms,
                    fingerprint: crate::pipeline::insert::bytes_to_hex(&r.fingerprint),
                    xxh3_128: xxh3_hex,
                    sha256: sha256_hex,
                    imported_at: now_ms,
                    status: ItemStatus::Failed,
                    error: Some(reason.clone()),
                });
            }
            continue;
        }

        // Handle duplicates (by CRC/lookup stage reason, e.g. "db (...)"/"batch (...)")
        if let Some(reason) = &r.dup_reason {
            summary.duplicate += 1;
            // Record duplicate file to manifest
            if let Some(ref mut m) = manifest {
                m.files.push(ImportRecord {
                    src_path,
                    dest_path: None,
                    size: r.size,
                    mtime_ms: r.mtime_ms,
                    fingerprint: crate::pipeline::insert::bytes_to_hex(&r.fingerprint),
                    xxh3_128: xxh3_hex,
                    sha256: sha256_hex,
                    imported_at: now_ms,
                    status: ItemStatus::Duplicate,
                    error: Some(reason.clone()),
                });
            }
            continue;
        }

        // Handle duplicates (by hash)
        if r.is_duplicate {
            summary.duplicate += 1;
            // Record duplicate file to manifest
            if let Some(ref mut m) = manifest {
                m.files.push(ImportRecord {
                    src_path,
                    dest_path: None,
                    size: r.size,
                    mtime_ms: r.mtime_ms,
                    fingerprint: crate::pipeline::insert::bytes_to_hex(&r.fingerprint),
                    xxh3_128: xxh3_hex,
                    sha256: sha256_hex,
                    imported_at: now_ms,
                    status: ItemStatus::Duplicate,
                    error: None,
                });
            }
            continue;
        }

        // Record added file to manifest
        if let Some(ref mut m) = manifest {
            m.files.push(ImportRecord {
                src_path: src_path.clone(),
                dest_path: Some(rel_path.to_path_buf()),
                size: r.size,
                mtime_ms: r.mtime_ms,
                fingerprint: crate::pipeline::insert::bytes_to_hex(&r.fingerprint),
                xxh3_128: xxh3_hex,
                sha256: sha256_hex,
                imported_at: now_ms,
                status: ItemStatus::Added,
                error: None,
            });
        }

        // Staged files become visible at their final path only after the
        // DB transaction commits; the caller performs the renames.
        if let Some(staged) = &r.staged_path {
            summary
                .staged_commits
                .push((staged.clone(), r.path.clone()));
        }

        files_to_insert.push(r);
    }

    // Batch insert using transaction
    if !files_to_insert.is_empty() {
        let mut updated_count = 0;
        let mut inserted_count = 0;

        db.with_transaction(|conn| {
            let mut insert_stmt = conn.prepare(
                "INSERT OR IGNORE INTO files \
                 (path, size, mtime, fingerprint, raw_unique_id, xxh3_128, sha256, status, imported_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'imported', ?8)"
            )?;

            let mut update_stmt = conn.prepare(
                "UPDATE files SET path = ?1, status = 'imported', mtime = ?2, imported_at = ?3 WHERE id = ?4"
            )?;

            for r in &files_to_insert {
                let rel_path = r.path.strip_prefix(opts.vault_root).unwrap_or(&r.path);
                // Use Unix-style paths for cross-platform database compatibility
                let rel_str = path_to_unix_string(rel_path);

                let (identity_hash, hash_col) = match &r.hash {
                    FileHash::Fast(xxh3) => (xxh3.as_slice(), "xxh3_128"),
                    FileHash::Full(_, sha256) => (sha256.as_slice(), "sha256"),
                };

                let missing_file: Option<i64> = conn.query_row(
                    &format!("SELECT id FROM files WHERE {} = ?1 AND status = 'missing' LIMIT 1", hash_col),
                    [identity_hash],
                    |row| row.get(0),
                ).ok();

                if let Some(file_id) = missing_file {
                    update_stmt.execute(rusqlite::params![
                        rel_str,
                        r.mtime_ms,
                        now_ms,
                        file_id,
                    ])?;
                    updated_count += 1;
                } else {
                    let (xxh3_bytes, sha256_bytes) = match &r.hash {
                        FileHash::Fast(xxh3) => (Some(xxh3.clone()), None),
                        FileHash::Full(xxh3, sha256) => (Some(xxh3.clone()), Some(sha256.clone())),
                    };

                    insert_stmt.execute(rusqlite::params![
                        rel_str,
                        r.size as i64,
                        r.mtime_ms,
                        r.fingerprint.clone(),
                        r.raw_unique_id.as_deref(),
                        xxh3_bytes,
                        sha256_bytes,
                        now_ms,
                    ])?;
                    inserted_count += 1;
                }
            }

            Ok(())
        })?;

        summary.added = inserted_count + updated_count;
    }

    // Write manifest with summary
    if let Some(ref mut m) = manifest
        && !m.files.is_empty()
    {
        m.summary = Some(ManifestSummary {
            total: summary.total,
            added: summary.added,
            duplicate: summary.duplicate,
            failed: summary.failed,
            skipped: summary.skipped,
        });
        let manager = ManifestManager::new(opts.vault_root);
        summary.manifest_path = Some(manager.save(m)?);
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_to_unix_string_unix_path() {
        let path = Path::new("/home/user/photos/file.jpg");
        let result = path_to_unix_string(path);
        assert_eq!(result, "home/user/photos/file.jpg");
    }

    #[test]
    fn test_path_to_unix_string_windows_style_path() {
        // Simulate Windows path components (as they would appear after strip_prefix)
        // On Windows: C:\Users\test\vault\2024\file.jpg -> rel_path = "2024\file.jpg"
        let path = Path::new("2024\\file.jpg");
        let result = path_to_unix_string(path);
        // Should convert backslash to forward slash
        assert_eq!(result, "2024/file.jpg");
    }

    #[test]
    fn test_path_to_unix_string_nested_windows_path() {
        // Simulate nested Windows directory structure
        let path = Path::new("2024\\03-15\\NIKON\\DSC_0001.JPG");
        let result = path_to_unix_string(path);
        assert_eq!(result, "2024/03-15/NIKON/DSC_0001.JPG");
    }

    #[test]
    fn test_path_to_unix_string_single_component() {
        let path = Path::new("file.jpg");
        let result = path_to_unix_string(path);
        assert_eq!(result, "file.jpg");
    }

    #[test]
    fn test_path_to_unix_string_empty() {
        let path = Path::new("");
        let result = path_to_unix_string(path);
        assert_eq!(result, "");
    }

    #[test]
    fn test_path_to_unix_string_cross_platform_compatibility() {
        // This test verifies that the same relative path structure
        // is stored identically regardless of platform

        // Unix-style input
        let unix_path = Path::new("2024/03/photo.jpg");
        let unix_result = path_to_unix_string(unix_path);

        // Windows-style input (simulated)
        let windows_path = Path::new("2024\\03\\photo.jpg");
        let windows_result = path_to_unix_string(windows_path);

        // Both should produce the same Unix-style output
        assert_eq!(unix_result, "2024/03/photo.jpg");
        assert_eq!(windows_result, "2024/03/photo.jpg");
        assert_eq!(unix_result, windows_result);
    }

    /// Regression test for BUG-1: hash IO errors were misclassified as
    /// duplicates because classification relied on a "hash error" string
    /// prefix that never matched the actual messages ("xxh3_128 error: …").
    /// The `hash_error` field now carries errors structurally.
    #[test]
    fn batch_insert_classifies_hash_error_as_failed_not_duplicate() {
        use std::path::PathBuf;

        let db = Db::open_in_memory().unwrap();
        let mk = |path: &str,
                  hash: FileHash,
                  dup_reason: Option<String>,
                  hash_error: Option<String>| HashResult {
            path: PathBuf::from(format!("/vault/2024/{path}")),
            src_path: Some(PathBuf::from(format!("/src/{path}"))),
            staged_path: None,
            size: 10,
            mtime_ms: 0,
            fingerprint: vec![1, 2, 3],
            raw_unique_id: None,
            hash,
            is_duplicate: false,
            dup_reason,
            hash_error,
        };

        let results = vec![
            // Hash IO error on the vault copy — must count as failed
            mk(
                "eio.jpg",
                FileHash::Fast(vec![]),
                None,
                Some("xxh3_128 error: EIO".to_string()),
            ),
            // Genuine duplicate found by the dedup stage
            mk(
                "dup.jpg",
                FileHash::Fast(vec![9, 9, 9]),
                Some("db (xxh3_128)".to_string()),
                None,
            ),
            // Normal new file
            mk("new.jpg", FileHash::Fast(vec![1, 2, 3]), None, None),
        ];

        let opts = InsertOptions {
            vault_root: Path::new("/vault"),
            session_id: "s1",
            write_manifest: false,
            source_root: None,
            force: false,
            session_type: SessionType::Import,
        };
        let summary = batch_insert(results, &db, opts, None).unwrap();

        assert_eq!(summary.failed, 1, "hash IO error must count as failed");
        assert_eq!(
            summary.duplicate, 1,
            "dedup-stage hit must count as duplicate"
        );
        assert_eq!(summary.added, 1, "new file must be inserted");

        // The failed file must not be tracked in the DB; the new file must be.
        assert!(db.get_file_by_path("2024/eio.jpg").unwrap().is_none());
        assert!(db.get_file_by_path("2024/new.jpg").unwrap().is_some());
    }

    /// BUG-1 契约的 manifest 层：哈希 IO 错误必须以 status=Failed + 错误消息
    /// 写入 manifest（此前被误写为 Duplicate）。
    #[test]
    fn batch_insert_writes_hash_error_to_manifest_as_failed() {
        use std::path::PathBuf;

        let tmp = tempfile::tempdir().unwrap();
        let vault_root = tmp.path();
        let db = Db::open_in_memory().unwrap();

        let results = vec![HashResult {
            path: vault_root.join("2024/eio.jpg"),
            src_path: Some(PathBuf::from("/src/eio.jpg")),
            staged_path: None,
            size: 10,
            mtime_ms: 0,
            fingerprint: vec![1, 2, 3],
            raw_unique_id: None,
            hash: FileHash::Fast(vec![]),
            is_duplicate: false,
            dup_reason: None,
            hash_error: Some("xxh3_128 error: EIO".to_string()),
        }];

        let opts = InsertOptions {
            vault_root,
            session_id: "s1",
            write_manifest: true,
            source_root: Some(Path::new("/src")),
            force: false,
            session_type: SessionType::Import,
        };
        let summary = batch_insert(results, &db, opts, None).unwrap();
        assert_eq!(summary.failed, 1);

        // Manifest 必须落盘且将该文件记为 Failed
        let manifest_path = summary
            .manifest_path
            .expect("write_manifest=true 应产生 manifest 路径");
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        let files = json["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["status"], "failed");
        assert_eq!(files[0]["error"].as_str().unwrap(), "xxh3_128 error: EIO");
        assert!(files[0]["dest_path"].is_null());
    }
}
