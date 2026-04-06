//! Stage E: Batch DB insertion.

use std::path::Path;

use indicatif::ProgressBar;

use crate::config::HashAlgorithm;
use crate::db::Db;
use crate::pipeline::types::{HashResult, PipelineSummary};
use crate::verify::manifest::{ImportManifest, ImportRecord};

/// Options for batch insertion.
pub struct InsertOptions<'a> {
    pub vault_root: &'a Path,
    pub hash_algo: &'a HashAlgorithm,
    pub session_id: &'a str,
    /// Whether to write manifest (import: true, add: false)
    pub write_manifest: bool,
    /// Source root (for manifest, import only)
    pub source_root: Option<&'a Path>,
    /// Force mode - overwrite existing files
    pub force: bool,
}

/// Convert hash bytes to hex string for manifest.
fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Insert all valid entries into DB using batch transaction.
///
/// Optimizations:
/// 1. **Batch transaction** - All inserts in single transaction (major speedup)
/// 2. **Prepared statement** - SQL compiled once, reused for all rows
/// 3. **Single event** - One "batch.imported" event per command instead of per-file events
///
/// # Arguments
/// * `results` - Hash results (with is_duplicate and dup_reason flags)
/// * `db` - Database handle
/// * `opts` - Insert options
/// * `progress` - Optional progress bar
///
/// # Returns
/// PipelineSummary with counts of added/duplicate/failed/skipped files.
pub fn batch_insert(
    results: Vec<HashResult>,
    db: &Db,
    opts: InsertOptions,
    progress: Option<&ProgressBar>,
) -> anyhow::Result<PipelineSummary> {
    let mut summary = PipelineSummary::new(results.len());
    let now_ms = crate::import::utils::unix_now_ms();

    // Prepare manifest if needed
    let mut manifest = if opts.write_manifest && opts.source_root.is_some() {
        Some(ImportManifest {
            session_id: opts.session_id.to_string(),
            source_root: opts.source_root.unwrap().to_path_buf(),
            imported_at: now_ms,
            hash_algorithm: format!("{:?}", opts.hash_algo).to_lowercase(),
            files: Vec::new(),
        })
    } else {
        None
    };

    // Collect files to be inserted (filter duplicates first)
    let mut files_to_insert: Vec<HashResult> = Vec::with_capacity(results.len());
    
    for r in results {
        // Update progress bar (filtering phase)
        if let Some(pb) = progress {
            pb.inc(1);
        }
        
        let rel_path = r.path.strip_prefix(opts.vault_root).unwrap_or(&r.path);
        let rel_str = rel_path.to_string_lossy().into_owned();

        // Skip if already tracked by path (unless force mode)
        if !opts.force {
            if let Ok(Some(_)) = db.get_file_by_path(&rel_str) {
                summary.skipped += 1;
                continue;
            }
        }

        // Handle errors
        if let Some(ref reason) = r.dup_reason {
            if reason.starts_with("hash error") {
                summary.failed += 1;
            } else {
                summary.duplicate += 1;
            }
            continue;
        }

        // Skip duplicates
        if r.is_duplicate {
            summary.duplicate += 1;
            continue;
        }

        // Add to manifest if needed
        if let Some(ref mut m) = manifest {
            let (xxh3, sha256) = match opts.hash_algo {
                HashAlgorithm::Xxh3_128 => (Some(r.hash_bytes.as_slice()), None),
                HashAlgorithm::Sha256 => (None, Some(r.hash_bytes.as_slice())),
            };
            let src_path = r.src_path.clone().unwrap_or_else(|| r.path.clone());
            m.files.push(ImportRecord {
                src_path,
                dest_path: rel_path.to_path_buf(),
                size: r.size,
                mtime_ms: r.mtime_ms,
                crc32c: r.crc32c,
                xxh3_128: xxh3.map(bytes_to_hex),
                sha256: sha256.map(bytes_to_hex),
                imported_at: now_ms,
            });
        }

        files_to_insert.push(r);
    }

    // Batch insert using transaction and prepared statement
    if !files_to_insert.is_empty() {
        let mut updated_count = 0;
        let mut inserted_count = 0;
        
        db.with_transaction(|conn| {
            // Prepare statements once
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
                let rel_str = rel_path.to_string_lossy();

                let (xxh3, sha256) = match opts.hash_algo {
                    HashAlgorithm::Xxh3_128 => (Some(r.hash_bytes.as_slice()), None),
                    HashAlgorithm::Sha256 => (None, Some(r.hash_bytes.as_slice())),
                };
                
                // Check if there's a 'missing' file with same hash to recover
                let hash_bytes = xxh3.or(sha256).map(|b| b.to_vec());
                let missing_file: Option<i64> = if let Some(ref hash) = hash_bytes {
                    let hash_col = match opts.hash_algo {
                        HashAlgorithm::Xxh3_128 => "xxh3_128",
                        HashAlgorithm::Sha256 => "sha256",
                    };
                    conn.query_row(
                        &format!("SELECT id FROM files WHERE {} = ?1 AND status = 'missing' LIMIT 1", hash_col),
                        [hash],
                        |row| row.get(0),
                    ).ok()
                } else {
                    None
                };

                if let Some(file_id) = missing_file {
                    // Recover missing file: update path and status
                    update_stmt.execute(rusqlite::params![
                        rel_str,
                        r.mtime_ms,
                        now_ms,
                        file_id,
                    ])?;
                    updated_count += 1;
                } else {
                    // Normal insert
                    insert_stmt.execute(rusqlite::params![
                        rel_str,
                        r.size as i64,
                        r.mtime_ms,
                        r.crc32c as i64,
                        r.raw_unique_id.as_deref(),
                        xxh3.map(|b| b.to_vec()),
                        sha256.map(|b| b.to_vec()),
                        now_ms,
                    ])?;
                    inserted_count += 1;
                }
            }
            
            Ok(())
        })?;

        summary.added = inserted_count + updated_count;
    }

    // Record single batch event (optimization #3)
    // Instead of N per-file events, record 1 summary event
    let batch_payload = serde_json::json!({
        "session_id": opts.session_id,
        "file_count": summary.added,
        "total_count": summary.total,
        "duplicate_count": summary.duplicate,
        "skipped_count": summary.skipped,
        "failed_count": summary.failed,
        "hash_algorithm": format!("{:?}", opts.hash_algo).to_lowercase(),
    });
    
    db.append_event(
        "batch.imported",
        "import",
        0,
        &batch_payload.to_string(),
        |_conn| Ok(()),
    )?;

    // Write manifest file
    if let Some(m) = manifest {
        let manifest_dir = opts.vault_root.join(".svault").join("manifests");
        std::fs::create_dir_all(&manifest_dir)?;
        let manifest_path = manifest_dir.join(format!("import-{}.json", opts.session_id));
        m.save(&manifest_path)?;
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::pipeline::types::HashResult;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn test_batch_insert_empty() {
        let tmp = TempDir::new().unwrap();
        let db = Db::open_in_memory().unwrap();
        
        let results = vec![];
        let opts = InsertOptions {
            vault_root: tmp.path(),
            hash_algo: &HashAlgorithm::Xxh3_128,
            session_id: "test-001",
            write_manifest: false,
            source_root: None,
            force: false,
        };

        let summary = batch_insert(results, &db, opts, None).unwrap();
        
        assert_eq!(summary.total, 0);
        assert_eq!(summary.added, 0);
    }

    #[test]
    fn test_batch_insert_skips_duplicates() {
        let tmp = TempDir::new().unwrap();
        let db = Db::open_in_memory().unwrap();
        
        let results = vec![
            HashResult {
                path: PathBuf::from("/vault/photo.jpg"),
                src_path: Some(PathBuf::from("/source/photo.jpg")),
                size: 1000,
                mtime_ms: 12345,
                crc32c: 999,
                raw_unique_id: None,
                hash_bytes: vec![1, 2, 3, 4],
                is_duplicate: true,
                dup_reason: Some("db".to_string()),
            },
        ];
        
        let opts = InsertOptions {
            vault_root: Path::new("/vault"),
            hash_algo: &HashAlgorithm::Xxh3_128,
            session_id: "test-001",
            write_manifest: false,
            source_root: None,
            force: false,
        };

        let summary = batch_insert(results, &db, opts, None).unwrap();
        
        assert_eq!(summary.duplicate, 1);
        assert_eq!(summary.added, 0);
    }

    #[test]
    fn test_batch_insert_multiple_files() {
        let tmp = TempDir::new().unwrap();
        let db = Db::open_in_memory().unwrap();
        
        let results = vec![
            HashResult {
                path: tmp.path().join("photo1.jpg"),
                src_path: Some(PathBuf::from("/source/photo1.jpg")),
                size: 1000,
                mtime_ms: 12345,
                crc32c: 111,
                raw_unique_id: None,
                hash_bytes: vec![1, 2, 3, 4],
                is_duplicate: false,
                dup_reason: None,
            },
            HashResult {
                path: tmp.path().join("photo2.jpg"),
                src_path: Some(PathBuf::from("/source/photo2.jpg")),
                size: 2000,
                mtime_ms: 12346,
                crc32c: 222,
                raw_unique_id: None,
                hash_bytes: vec![5, 6, 7, 8],
                is_duplicate: false,
                dup_reason: None,
            },
        ];
        
        let opts = InsertOptions {
            vault_root: tmp.path(),
            hash_algo: &HashAlgorithm::Xxh3_128,
            session_id: "test-001",
            write_manifest: false,
            source_root: None,
            force: false,
        };

        let summary = batch_insert(results, &db, opts, None).unwrap();
        
        assert_eq!(summary.total, 2);
        assert_eq!(summary.added, 2);
        assert_eq!(summary.duplicate, 0);
        
        // Verify files were actually inserted
        let file1 = db.get_file_by_path("photo1.jpg").unwrap();
        let file2 = db.get_file_by_path("photo2.jpg").unwrap();
        assert!(file1.is_some());
        assert!(file2.is_some());
    }
}
