//! Stage E: Batch DB insertion.

use std::path::Path;

use crate::db::Db;
use crate::pipeline::types::{FileHash, HashResult, PipelineSummary};
use crate::verify::manifest::{ImportManifest, ImportRecord, ManifestManager, ItemStatus, SessionType, ManifestSummary};

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
    normalized.strip_prefix('/').map(String::from).unwrap_or(normalized)
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

impl Default for SessionType {
    fn default() -> Self {
        SessionType::Import
    }
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
    let now_ms = crate::import::utils::unix_now_ms();

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
            FileHash::Full(xxh3, sha256) => {
                (Some(bytes_to_hex(xxh3)), Some(bytes_to_hex(sha256)))
            }
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
                    crc32c: r.crc32c,
                    xxh3_128: xxh3_hex,
                    sha256: sha256_hex,
                    imported_at: now_ms,
                    status: ItemStatus::Skipped,
                    error: None,
                });
            }
            continue;
        }

        // Handle errors (hash errors etc.)
        if let Some(ref reason) = r.dup_reason {
            if reason.starts_with("hash error") {
                summary.failed += 1;
                // Record failed file to manifest
                if let Some(ref mut m) = manifest {
                    m.files.push(ImportRecord {
                        src_path,
                        dest_path: None,
                        size: r.size,
                        mtime_ms: r.mtime_ms,
                        crc32c: r.crc32c,
                        xxh3_128: xxh3_hex,
                        sha256: sha256_hex,
                        imported_at: now_ms,
                        status: ItemStatus::Failed,
                        error: Some(reason.clone()),
                    });
                }
            } else {
                summary.duplicate += 1;
                // Record duplicate file to manifest
                if let Some(ref mut m) = manifest {
                    m.files.push(ImportRecord {
                        src_path,
                        dest_path: None,
                        size: r.size,
                        mtime_ms: r.mtime_ms,
                        crc32c: r.crc32c,
                        xxh3_128: xxh3_hex,
                        sha256: sha256_hex,
                        imported_at: now_ms,
                        status: ItemStatus::Duplicate,
                        error: Some(reason.clone()),
                    });
                }
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
                    crc32c: r.crc32c,
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
                crc32c: r.crc32c,
                xxh3_128: xxh3_hex,
                sha256: sha256_hex,
                imported_at: now_ms,
                status: ItemStatus::Added,
                error: None,
            });
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
                 (path, size, mtime, crc32c, raw_unique_id, xxh3_128, sha256, status, imported_at) \
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
                        r.crc32c as i64,
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

    // Record batch event with session_type
    let payload = serde_json::json!({
        "session_id": opts.session_id,
        "session_type": opts.session_type.to_string(),
        "source": opts.source_root.map(|p| path_to_unix_string(p)).unwrap_or_default(),
        "total_files": summary.total,
        "added": summary.added,
        "duplicate": summary.duplicate,
        "failed": summary.failed,
        "skipped": summary.skipped,
        "manifest": manifest.as_ref().map(|m| m.files.len()).unwrap_or(0),
    });

    db.append_event(
        "batch.imported",
        "batch",
        0,
        &payload.to_string(),
        |_conn| Ok(()),
    )?;

    // Write manifest with summary
    if let Some(ref mut m) = manifest {
        if !m.files.is_empty() {
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
}
