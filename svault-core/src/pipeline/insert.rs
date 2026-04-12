//! Stage E: Batch DB insertion.

use std::path::Path;

use crate::db::Db;
use crate::pipeline::types::{FileHash, HashResult, PipelineSummary};
use crate::verify::manifest::{ImportManifest, ImportRecord, ManifestManager};

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
/// * `progress_cb` - Optional callback for progress updates
///
/// # Returns
/// PipelineSummary with counts of added/duplicate/failed/skipped files.
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
            source_root: root.to_path_buf(),
            imported_at: now_ms,
            hash_algorithm: "xxh3_128".to_string(), // Primary hash is always xxh3_128
            files: Vec::new(),
        })
    } else {
        None
    };

    // Collect files to be inserted (filter duplicates first)
    let mut files_to_insert: Vec<HashResult> = Vec::with_capacity(results.len());

    for r in results {
        // Update progress (filtering phase)
        if let Some(cb) = progress_cb {
            cb();
        }

        let rel_path = r.path.strip_prefix(opts.vault_root).unwrap_or(&r.path);
        let rel_str = rel_path.to_string_lossy().into_owned();

        // Skip if already tracked by path (unless force mode or the existing file is 'missing')
        // 'missing' files should be allowed to recover (re-import with same path)
        if !opts.force {
            if let Ok(Some(existing)) = db.get_file_by_path(&rel_str) {
                if existing.status != "missing" {
                    summary.skipped += 1;
                    continue;
                }
                // 'missing' status: allow to proceed for recovery
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
            let src_path = r.src_path.clone().unwrap_or_else(|| r.path.clone());
            let (xxh3_hex, sha256_hex) = match &r.hash {
                FileHash::Fast(xxh3) => (Some(bytes_to_hex(xxh3)), None),
                FileHash::Full(xxh3, sha256) => {
                    (Some(bytes_to_hex(xxh3)), Some(bytes_to_hex(sha256)))
                }
            };
            m.files.push(ImportRecord {
                src_path,
                dest_path: rel_path.to_path_buf(),
                size: r.size,
                mtime_ms: r.mtime_ms,
                crc32c: r.crc32c,
                xxh3_128: xxh3_hex,
                sha256: sha256_hex,
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

                // Get identity hash for recovery check
                let (identity_hash, hash_col) = match &r.hash {
                    FileHash::Fast(xxh3) => (xxh3.as_slice(), "xxh3_128"),
                    FileHash::Full(_, sha256) => (sha256.as_slice(), "sha256"),
                };

                // Check if there's a 'missing' file with same hash to recover
                let missing_file: Option<i64> = conn.query_row(
                    &format!("SELECT id FROM files WHERE {} = ?1 AND status = 'missing' LIMIT 1", hash_col),
                    [identity_hash],
                    |row| row.get(0),
                ).ok();

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

    // Record single batch event (optimization #3)
    // Instead of N per-file events, record 1 summary event
    let payload = serde_json::json!({
        "session_id": opts.session_id,
        "source": opts.source_root.map(|p| p.to_string_lossy()).unwrap_or_default(),
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
        0, // No specific file ID for batch event
        &payload.to_string(),
        |_conn| Ok(()),
    )?;

    // Write manifest file if needed
    if let Some(m) = manifest
        && !m.files.is_empty()
    {
        let manager = ManifestManager::new(opts.vault_root);
        summary.manifest_path = Some(manager.save(&m)?);
    }

    Ok(summary)
}
