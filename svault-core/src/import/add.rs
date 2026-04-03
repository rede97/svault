//! `svault add` — register files already inside the vault.
//!
//! Scans a directory within the vault, computes hashes for any files not
//! already tracked in the database, and inserts them as `file.imported` events.

use std::fs;
use std::path::Path;

use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::config::{Config, HashAlgorithm};
use crate::db::Db;
use crate::hash::{crc32c_region, sha256_file, xxh3_128_file};
use crate::vfs::system::SystemFs;
use crate::vfs::VfsBackend;

/// Summary of an `add` operation.
#[derive(Debug, Default)]
pub struct AddSummary {
    pub total: usize,
    pub added: usize,
    pub duplicate: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Options for `svault add`.
pub struct AddOptions {
    pub path: std::path::PathBuf,
    pub vault_root: std::path::PathBuf,
    pub hash: HashAlgorithm,
}

/// Run `add` on a directory inside the vault.
pub fn run_add(opts: AddOptions, db: &Db) -> anyhow::Result<AddSummary> {
    let config = Config::load(&opts.vault_root)?;
    let exts: Vec<&str> = config
        .import
        .allowed_extensions
        .iter()
        .map(|s| s.as_str())
        .collect();

    let fs = SystemFs::open(&opts.path)?;
    let entries = fs.walk(Path::new(""), &exts)?;
    let total = entries.len();

    if total == 0 {
        eprintln!("{} No files found in {}", style("Warning:").yellow().bold(), opts.path.display());
        return Ok(AddSummary::default());
    }

    eprintln!(
        "{} Scanning {} files in {}",
        style("Adding:").bold().cyan(),
        style(total).cyan(),
        style(opts.path.display())
    );

    let bar = ProgressBar::new(total as u64);
    bar.set_style(
        ProgressStyle::with_template("{prefix:.bold.cyan} [{bar:40}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("=> "),
    );
    bar.set_prefix("Hashing  ");

    let hashed: Vec<_> = entries
        .into_par_iter()
        .filter_map(|e| {
            let filename = e.path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            bar.set_message(filename.clone());

            let meta = match fs::metadata(&e.path) {
                Ok(m) => m,
                Err(_) => {
                    bar.inc(1);
                    return Some((e.path, 0u64, 0i64, 0u32, Err("metadata failed")));
                }
            };
            let size = meta.len();
            let mtime_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);

            let crc = match crc32c_region(&e.path, 0, 65536) {
                Ok(v) => v,
                Err(_) => {
                    bar.inc(1);
                    return Some((e.path, size, mtime_ms, 0, Err("crc32c failed")));
                }
            };

            let hash_result = match opts.hash {
                HashAlgorithm::Xxh3_128 => xxh3_128_file(&e.path)
                    .map(|h| h.to_bytes().to_vec()),
                HashAlgorithm::Sha256 => sha256_file(&e.path)
                    .map(|h| h.to_bytes().to_vec()),
            };

            bar.inc(1);
            Some((e.path, size, mtime_ms, crc, hash_result.map_err(|_| "hash failed")))
        })
        .collect();

    bar.finish_and_clear();

    let mut summary = AddSummary { total, ..Default::default() };
    let now_ms = crate::import::utils::unix_now_ms();

    for (path, size, mtime_ms, crc, hash_result) in hashed {
        let rel_path = path.strip_prefix(&opts.vault_root).unwrap_or(&path);
        let rel_str = rel_path.to_string_lossy().into_owned();

        // Skip if already tracked by path
        if let Ok(Some(_)) = db.get_file_by_path(&rel_str) {
            summary.skipped += 1;
            continue;
        }

        let hash_bytes = match hash_result {
            Ok(b) => b,
            Err(msg) => {
                eprintln!("  {} {} - {}", style("Error").red(), style(rel_path.display()), msg);
                summary.failed += 1;
                continue;
            }
        };

        // Check hash duplicate
        let dup = db.lookup_by_hash(&hash_bytes, &opts.hash).unwrap_or(None);
        if dup.is_some() {
            summary.duplicate += 1;
            continue;
        }

        let (xxh3, sha256) = match &opts.hash {
            HashAlgorithm::Xxh3_128 => (Some(hash_bytes.as_slice()), None),
            HashAlgorithm::Sha256 => (None, Some(hash_bytes.as_slice())),
        };

        let payload = serde_json::json!({
            "path": rel_str,
            "size": size,
            "mtime": mtime_ms,
        }).to_string();

        if let Err(e) = db.append_event(
            "file.imported", "file", 0, &payload,
            |conn| {
                conn.execute(
                    "INSERT INTO files \
                     (path, size, mtime, crc32c_val, xxh3_128, sha256, status, imported_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'imported', ?7)",
                    rusqlite::params![rel_str, size as i64, mtime_ms, crc as i64, xxh3, sha256, now_ms],
                )?;
                Ok(())
            },
        ) {
            eprintln!("  {} {} - {}", style("Error").red(), style(rel_path.display()), e);
            summary.failed += 1;
            continue;
        }

        summary.added += 1;
        eprintln!("  {} {}", style("Added").green(), style(rel_path.display()));
    }

    eprintln!();
    eprintln!("{} {} file(s) added", style("Finished:").bold().green(), style(summary.added).green());
    if summary.duplicate > 0 {
        eprintln!("         {} duplicate(s) skipped", style(summary.duplicate).yellow());
    }
    if summary.skipped > 0 {
        eprintln!("         {} already tracked", style(summary.skipped));
    }
    if summary.failed > 0 {
        eprintln!("         {} file(s) failed", style(summary.failed).red());
    }

    Ok(summary)
}
