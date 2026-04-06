//! Stage E: Batch DB insertion.

use std::path::Path;

use console::style;

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
}

/// Convert hash bytes to hex string for manifest.
fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Insert all valid entries into DB and optionally write manifest.
pub fn batch_insert(
    results: Vec<HashResult>,
    db: &Db,
    opts: InsertOptions,
) -> anyhow::Result<PipelineSummary> {
    let mut summary = PipelineSummary::new(results.len());
    let now_ms = crate::import::utils::unix_now_ms();
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

    for r in results {
        // Compute relative path from vault root
        let rel_path = r.path.strip_prefix(opts.vault_root).unwrap_or(&r.path);
        let rel_str = rel_path.to_string_lossy().into_owned();

        // Skip if already tracked by path (add command only)
        if let Ok(Some(_)) = db.get_file_by_path(&rel_str) {
            summary.skipped += 1;
            continue;
        }

        // Handle errors and duplicates
        if let Some(reason) = &r.dup_reason {
            if reason.starts_with("hash error") {
                summary.failed += 1;
                eprintln!(
                    "  {} {} - {}",
                    style("Error").red(),
                    style(rel_path.display()),
                    reason
                );
            } else {
                summary.duplicate += 1;
                eprintln!(
                    "  {} {} ({})",
                    style("Duplicate").yellow(),
                    style(rel_path.display()),
                    reason
                );
            }
            continue;
        }

        // Skip duplicates
        if r.is_duplicate {
            summary.duplicate += 1;
            continue;
        }

        // Prepare hash fields
        let (xxh3, sha256) = match opts.hash_algo {
            HashAlgorithm::Xxh3_128 => (Some(r.hash_bytes.as_slice()), None),
            HashAlgorithm::Sha256 => (None, Some(r.hash_bytes.as_slice())),
        };

        // Insert into DB via event
        let payload = serde_json::json!({
            "path": rel_str,
            "size": r.size as i64,
            "mtime_ms": r.mtime_ms,
            "crc32c": r.crc32c as i64,
            "xxh3_128": xxh3.map(|b| b.to_vec()),
            "sha256": sha256.map(|b| b.to_vec()),
            "raw_unique_id": r.raw_unique_id,
        });

        db.append_event(
            "file.imported",
            "file",
            0, // entity_id placeholder
            &payload.to_string(),
            |_conn| Ok(()),
        )?;

        // Add to manifest
        if let Some(ref mut m) = manifest {
            m.files.push(ImportRecord {
                src_path: r.path.clone(),
                dest_path: rel_path.to_path_buf(),
                size: r.size,
                mtime_ms: r.mtime_ms,
                crc32c: r.crc32c,
                xxh3_128: xxh3.map(bytes_to_hex),
                sha256: sha256.map(bytes_to_hex),
                imported_at: now_ms,
            });
        }

        summary.added += 1;
    }

    // Write manifest file
    if let Some(m) = manifest {
        let manifest_dir = opts.vault_root.join(".svault").join("manifests");
        std::fs::create_dir_all(&manifest_dir)?;
        let manifest_path = manifest_dir.join(format!("import-{}.json", opts.session_id));
        m.save(&manifest_path)?;

        // Also record import.completed event
        let completed_payload = serde_json::json!({
            "session_id": opts.session_id,
            "total_files": summary.total,
            "new_files": summary.added,
            "duplicate_files": summary.duplicate,
            "manifest_path": manifest_path.to_string_lossy().to_string(),
        });
        db.append_event(
            "import.completed",
            "import",
            0,
            &completed_payload.to_string(),
            |_conn| Ok(()),
        )?;
    }

    Ok(summary)
}
