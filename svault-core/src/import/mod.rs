//! Import pipeline (Stages A–E).
//!
//! Stage A: directory scan via VFS walk
//! Stage B: CRC32C fingerprint + DB cache lookup → likely_new / likely_duplicate
//! Stage C: copy likely_new files to vault
//! Stage D: strong hash (xxh3_128 or sha256) + three-layer dedup
//! Stage E: batch DB write + manifest

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use console::style;
use dashmap::DashMap;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::{
    config::{HashAlgorithm, ImportConfig, RecheckMode},
    db::Db,
    hash::{crc32c_region, sha256_file, xxh3_128_file},
    vfs::system::SystemFs,
    vfs::VfsBackend,
};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Options controlling a single import run.
pub struct ImportOptions {
    /// Source directory to scan.
    pub source: PathBuf,
    /// Vault root directory (contains `.svault/`).
    pub vault_root: PathBuf,
    /// Hash algorithm to use for Stage D (strong hash).
    pub hash: HashAlgorithm,
    /// Recheck mode for all-cache-hit scenario.
    pub recheck: RecheckMode,
    /// If true, scan and report but do not copy files or write to DB.
    pub dry_run: bool,
    /// If true, skip the interactive y/N confirmation after Stage B.
    pub yes: bool,
    /// If true, print skipped (likely-duplicate) files during Stage B scan.
    pub show_skip: bool,
    /// Import configuration from `svault.toml`.
    pub import_config: ImportConfig,
}

/// Per-file status after Stage B.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    /// CRC32C cache miss — probably a new file.
    LikelyNew,
    /// CRC32C cache hit — probably already in vault.
    LikelyCacheDuplicate,
    /// Confirmed imported (Stage E complete).
    Imported,
    /// Confirmed duplicate (Stage D dedup).
    Duplicate,
    /// Processing failed.
    Failed(String),
}

/// Per-file scan result from Stage B.
#[derive(Debug, Clone)]
pub struct ScanEntry {
    pub src_path: PathBuf,
    pub size: u64,
    pub mtime_ms: i64,
    pub crc32c: u32,
    pub status: FileStatus,
}

/// Final summary returned to the caller.
#[derive(Debug, Default)]
pub struct ImportSummary {
    pub total: usize,
    pub imported: usize,
    pub duplicate: usize,
    pub failed: usize,
    pub manifest_path: Option<PathBuf>,
    /// Set when all files were cache hits and import exited early.
    pub all_cache_hit: bool,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run the full import pipeline (Stages A–E).
pub fn run(opts: ImportOptions, db: &Db) -> anyhow::Result<ImportSummary> {
    let session_id = session_id_now();

    // ------------------------------------------------------------------
    // Stage A: directory scan
    // ------------------------------------------------------------------
    let exts: Vec<&str> = opts.import_config.allowed_extensions
        .iter().map(|s| s.as_str()).collect();

    eprintln!("{} {}",
        style("Scanning").bold().cyan(),
        style(opts.source.display()).dim());
    let src_fs = SystemFs::open(&opts.source)
        .map_err(|e| anyhow::anyhow!("cannot open source: {e}"))?;
    let dir_entries = src_fs.walk(Path::new(""), &exts)
        .map_err(|e| anyhow::anyhow!("scan failed: {e}"))?;
    let total = dir_entries.len();
    eprintln!("{} {}{}\n",
        style("Scanning").bold().cyan(),
        style(opts.source.display()).dim(),
        style(format!(" {total} files")).green());

    if total == 0 {
        return Ok(ImportSummary { total: 0, ..Default::default() });
    }

    // ------------------------------------------------------------------
    // Stage B: CRC32C fingerprint
    // ------------------------------------------------------------------
    // CRC32C IO runs in a rayon thread pool; DB lookups run on the calling
    // thread afterwards (rusqlite Connection is !Sync and cannot cross threads).

    // Step B1: compute CRC32C for every file in parallel, with progress bar
    let scan_bar = ProgressBar::new(total as u64);
    scan_bar.set_style(
        ProgressStyle::with_template(
            "{prefix:.bold.cyan} [{bar:40.cyan/blue}] {pos}/{len}  {wide_msg:.dim}",
        )
        .unwrap()
        .progress_chars("=> "),
    );
    scan_bar.set_prefix("Scanning");

    let crcs: Vec<(crate::vfs::DirEntry, Result<u32, String>)> = dir_entries
        .into_par_iter()
        .map(|e| {
            let abs = opts.source.join(&e.path);
            // Print per-file line above the bar (like `cargo` does)
            scan_bar.println(format!(
                "  {} {}",
                style("Found").green(),
                style(e.path.display()).dim(),
            ));
            let result = crc32c_region(&abs, 0, 65536)
                .map_err(|err| err.to_string());
            scan_bar.inc(1);
            scan_bar.set_message(e.path.display().to_string());
            (e, result)
        })
        .collect();
    scan_bar.finish_and_clear();

    // Step B2: DB lookup on calling thread (single-threaded, cheap)
    let scan_entries: Vec<ScanEntry> = crcs
        .into_iter()
        .map(|(e, crc_result)| {
            let abs = opts.source.join(&e.path);
            let crc = match crc_result {
                Err(err) => {
                    return ScanEntry {
                        src_path: abs, size: e.size, mtime_ms: e.mtime_ms,
                        crc32c: 0,
                        status: FileStatus::Failed(err),
                    };
                }
                Ok(v) => v,
            };
            let cached = db.lookup_by_crc32c(e.size as i64, crc).unwrap_or(None);
            let status = if cached.is_some() {
                FileStatus::LikelyCacheDuplicate
            } else {
                FileStatus::LikelyNew
            };
            if opts.show_skip && status == FileStatus::LikelyCacheDuplicate {
                eprintln!("  {} {}",
                    style("Skip ").yellow(),
                    style(e.path.display()).dim());
            }
            ScanEntry {
                src_path: abs, size: e.size, mtime_ms: e.mtime_ms, crc32c: crc,
                status,
            }
        })
        .collect();

    let likely_new: Vec<&ScanEntry> = scan_entries.iter()
        .filter(|e| e.status == FileStatus::LikelyNew).collect();
    let likely_dup = scan_entries.iter()
        .filter(|e| e.status == FileStatus::LikelyCacheDuplicate).count();
    let failed_b = scan_entries.iter()
        .filter(|e| matches!(e.status, FileStatus::Failed(_))).count();

    // Pre-flight summary
    eprintln!();
    eprintln!("{}", style("Pre-flight:").bold());
    eprintln!("  {}  {}",
        style(format!("Likely new:       {:>6}", likely_new.len())).green(),
        style("").dim());
    eprintln!("  {}  {}",
        style(format!("Likely duplicate: {:>6}", likely_dup)).yellow(),
        style("(cache hit)").dim());
    if failed_b > 0 {
        eprintln!("  {}",
            style(format!("Errors:           {:>6}", failed_b)).red());
    }

    // Early exit: all cache hits
    if likely_new.is_empty() {
        eprintln!();
        eprintln!("All {} files matched cache (no new files detected).", total);
        eprintln!("To verify duplicates, re-run with:");
        eprintln!("  {} EXIF binary comparison (recommended)", style("-R exif ").cyan());
        eprintln!("  {} full-file hash comparison", style("-R hash ").cyan());
        return Ok(ImportSummary {
            total, duplicate: likely_dup, failed: failed_b,
            all_cache_hit: true, ..Default::default()
        });
    }

    // ------------------------------------------------------------------
    // Write staging file + interactive confirmation
    // ------------------------------------------------------------------
    let staging_dir = opts.vault_root.join(".svault").join("staging");
    fs::create_dir_all(&staging_dir)?;
    let staging_path = staging_dir.join(format!("import-{session_id}.txt"));
    write_staging(&staging_path, &opts.source, &session_id, &scan_entries)?;
    eprintln!();
    eprintln!("{} {}",
        style("Staging list:").bold(),
        style(staging_path.display()).dim());

    if !opts.yes && !opts.dry_run {
        eprint!("{}", style("Proceed with import? [y/N] ").bold());
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)
            .map_err(|e| anyhow::anyhow!("failed to read stdin: {e}"))?;
        if !matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
            eprintln!("{}", style("Aborted. Staging list kept at:").yellow());
            eprintln!("  {}", staging_path.display());
            return Ok(ImportSummary {
                total,
                duplicate: likely_dup,
                failed: failed_b,
                all_cache_hit: false,
                ..Default::default()
            });
        }
    }

    let pending_path = opts.vault_root
        .join(".svault")
        .join(format!("import-{session_id}.pending"));
    write_pending(&pending_path, &opts.source, &session_id, &scan_entries)?;

    if opts.dry_run {
        eprintln!("\n(dry-run: no files copied)");
        return Ok(ImportSummary {
            total,
            duplicate: likely_dup,
            failed: failed_b,
            ..Default::default()
        });
    }

    // ------------------------------------------------------------------
    // Stage C: copy likely_new files
    // ------------------------------------------------------------------
    let vault_archive = opts.vault_root.clone();
    let dst_fs = SystemFs::open(&vault_archive)
        .map_err(|e| anyhow::anyhow!("cannot open vault: {e}"))?;

    // Shared error collector for Stage C
    let copy_errors: Arc<Mutex<HashMap<PathBuf, String>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let copy_bar = ProgressBar::new(likely_new.len() as u64);
    copy_bar.set_style(
        ProgressStyle::with_template(
            "{prefix:.bold.cyan} [{bar:40.cyan/blue}] {pos}/{len}  {wide_msg:.dim}",
        )
        .unwrap()
        .progress_chars("=> "),
    );
    copy_bar.set_prefix("Copying ");

    // Compute dest paths and copy
    let copied: Vec<(PathBuf, PathBuf, u64, i64)> = likely_new
        .par_iter()
        .filter_map(|e| {
            let rel = e.src_path.strip_prefix(&opts.source).unwrap_or(&e.src_path);
            let dest_rel = resolve_dest_path(
                &opts.import_config.path_template,
                rel,
                e.mtime_ms,
            );
            let dest_abs = vault_archive.join(&dest_rel);

            // Create parent dirs
            if let Some(parent) = dest_abs.parent() {
                if let Err(err) = fs::create_dir_all(parent) {
                    copy_errors.lock().unwrap()
                        .insert(e.src_path.clone(), err.to_string());
                    copy_bar.inc(1);
                    return None;
                }
            }

            // Copy (best strategy)
            match src_fs.copy_to(rel, &dst_fs, &dest_rel) {
                Ok(_) => {
                    copy_bar.inc(1);
                    copy_bar.set_message(rel.display().to_string());
                    Some((e.src_path.clone(), dest_abs, e.size, e.mtime_ms))
                }
                Err(err) => {
                    copy_errors.lock().unwrap()
                        .insert(e.src_path.clone(), err.to_string());
                    copy_bar.inc(1);
                    None
                }
            }
        })
        .collect();
    copy_bar.finish_and_clear();

    let copy_err_count = copy_errors.lock().unwrap().len();
    let copied_len = copied.len();
    eprintln!("{} {}/{} files",
        style("Copying ").bold().cyan(),
        style(copied_len).green(),
        likely_new.len());

    // ------------------------------------------------------------------
    // Stage D: strong hash + three-layer dedup
    // ------------------------------------------------------------------
    // Pass D1: parallel strong hash (IO-bound, no DB access)
    #[allow(dead_code)]
    #[derive(Debug)]
    struct HashResult {
        src: PathBuf,
        dest: PathBuf,
        size: u64,
        mtime_ms: i64,
        hash_bytes: Vec<u8>,
        is_duplicate: bool,
        dup_reason: Option<String>,
    }

    let hash_bar = ProgressBar::new(copied_len as u64);
    hash_bar.set_style(
        ProgressStyle::with_template(
            "{prefix:.bold.cyan} [{bar:40.cyan/blue}] {pos}/{len}  {wide_msg:.dim}",
        )
        .unwrap()
        .progress_chars("=> "),
    );
    hash_bar.set_prefix("Hashing  ");

    let hashed: Vec<HashResult> = copied
        .into_par_iter()
        .map(|(src, dest, size, mtime_ms)| {
            let hash_bytes = match &opts.hash {
                HashAlgorithm::Xxh3_128 => {
                    match xxh3_128_file(&dest) {
                        Ok(d) => d.to_bytes().to_vec(),
                        Err(e) => {
                            hash_bar.inc(1);
                            return HashResult {
                                src, dest, size, mtime_ms,
                                hash_bytes: vec![],
                                is_duplicate: false,
                                dup_reason: Some(format!("hash error: {e}")),
                            };
                        }
                    }
                }
                HashAlgorithm::Sha256 => {
                    match sha256_file(&dest) {
                        Ok(d) => d.to_bytes().to_vec(),
                        Err(e) => {
                            hash_bar.inc(1);
                            return HashResult {
                                src, dest, size, mtime_ms,
                                hash_bytes: vec![],
                                is_duplicate: false,
                                dup_reason: Some(format!("hash error: {e}")),
                            };
                        }
                    }
                }
            };
            hash_bar.inc(1);
            hash_bar.set_message(dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            HashResult { src, dest, size, mtime_ms, hash_bytes, is_duplicate: false, dup_reason: None }
        })
        .collect();
    hash_bar.finish_and_clear();

    // Pass D2: sequential DB lookup + DashMap dedup (calling thread, !Sync safe)
    // Layer 1: in-memory DashMap for same-batch dedup
    let seen: DashMap<Vec<u8>, PathBuf> = DashMap::new();
    let hash_results: Vec<HashResult> = hashed
        .into_iter()
        .map(|mut r| {
            if r.dup_reason.is_some() {
                return r; // already failed at hash stage
            }
            // Layer 2: DB lookup (cross-session dedup)
            let existing = db.lookup_by_hash(&r.hash_bytes, &opts.hash).unwrap_or(None);
            if existing.is_some() {
                r.is_duplicate = true;
                r.dup_reason = Some("db".to_string());
                return r;
            }
            // Layer 1: DashMap (same-batch dedup)
            use dashmap::mapref::entry::Entry;
            match seen.entry(r.hash_bytes.clone()) {
                Entry::Vacant(v) => { v.insert(r.dest.clone()); }
                Entry::Occupied(_) => {
                    r.is_duplicate = true;
                    r.dup_reason = Some("batch".to_string());
                    return r;
                }
            }
            r
        })
        .collect();

    eprintln!("{} {}/{} files",
        style("Hashing  ").bold().cyan(),
        style(hash_results.len()).green(),
        likely_new.len());

    // ------------------------------------------------------------------
    // Stage E: batch DB write + manifest
    // ------------------------------------------------------------------
    let now_ms = unix_now_ms();
    let mut imported_count = 0usize;
    let mut dup_count = likely_dup;
    let mut fail_count = failed_b + copy_err_count;

    for r in &hash_results {
        if let Some(reason) = &r.dup_reason {
            if reason != "hash error" {
                dup_count += 1;
                // Remove the copied file (it's a duplicate)
                let _ = fs::remove_file(&r.dest);
            } else {
                fail_count += 1;
            }
            continue;
        }

        let path_str = r.dest.to_string_lossy().into_owned();
        let (xxh3, sha256) = match &opts.hash {
            HashAlgorithm::Xxh3_128 => (Some(r.hash_bytes.as_slice()), None),
            HashAlgorithm::Sha256 => (None, Some(r.hash_bytes.as_slice())),
        };
        let payload = serde_json::json!({
            "path": path_str,
            "size": r.size,
            "mtime": r.mtime_ms,
        }).to_string();

        let result = db.append_event(
            "file.imported", "file", 0, &payload,
            |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO files \
                     (path, size, mtime, crc32c_val, xxh3_128, sha256, status, imported_at) \
                     VALUES (?1, ?2, ?3, NULL, ?4, ?5, 'imported', ?6)",
                    rusqlite::params![
                        path_str,
                        r.size as i64,
                        r.mtime_ms,
                        xxh3,
                        sha256,
                        now_ms,
                    ],
                )?;
                Ok(())
            },
        );

        match result {
            Ok(()) => imported_count += 1,
            Err(e) => {
                // Layer 3: UNIQUE constraint — it's a duplicate
                let msg = e.to_string();
                if msg.contains("UNIQUE constraint") {
                    dup_count += 1;
                    let _ = fs::remove_file(&r.dest);
                } else {
                    fail_count += 1;
                    eprintln!("  error inserting {}: {msg}", r.dest.display());
                }
            }
        }
    }

    // Write manifest
    let manifest_path = write_manifest(
        &opts.vault_root, &session_id, &scan_entries, &hash_results
            .iter().filter(|r| r.dup_reason.is_none()).map(|r| r.dest.clone()).collect::<Vec<_>>(),
    )?;

    // Delete .pending
    let _ = fs::remove_file(&pending_path);

    eprintln!();
    eprintln!("{} {}  {}  {}",
        style("Import complete:").bold().green(),
        style(format!("{imported_count} imported")).green(),
        style(format!("{dup_count} duplicates")).yellow(),
        if fail_count > 0 {
            style(format!("{fail_count} failed")).red().to_string()
        } else {
            format!("{fail_count} failed")
        },
    );
    eprintln!("{} {}",
        style("Manifest:").bold(),
        style(manifest_path.display()).dim());

    Ok(ImportSummary {
        total,
        imported: imported_count,
        duplicate: dup_count,
        failed: fail_count,
        manifest_path: Some(manifest_path),
        all_cache_hit: false,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn session_id_now() -> String {
    let ms = unix_now_ms();
    // Format: YYYYMMDDTHHMMSS (seconds precision is fine for session IDs)
    let secs = ms / 1000;
    // Use a simple numeric ID since we don't have a date library
    format!("{secs}")
}

/// Resolve the destination path from the template and file metadata.
/// Supported tokens: `{year}`, `{month}`, `{day}`, `{filename}`, `{ext}`
fn resolve_dest_path(template: &str, rel: &Path, mtime_ms: i64) -> PathBuf {
    let secs = mtime_ms / 1000;
    // Simple date extraction from Unix timestamp (no external date library)
    let (year, month, day) = secs_to_ymd(secs);
    let filename = rel.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = rel.extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = rel.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let rendered = template
        .replace("{year}", &format!("{year:04}"))
        .replace("{month}", &format!("{month:02}"))
        .replace("{day}", &format!("{day:02}"))
        .replace("{filename}", &filename)
        .replace("{stem}", &stem)
        .replace("{ext}", &ext);

    PathBuf::from(rendered)
}

/// Naive Unix timestamp → (year, month, day) without external crates.
fn secs_to_ymd(secs: i64) -> (i32, u32, u32) {
    // Days since 1970-01-01
    let days = (secs / 86400) as i32;
    // Shift epoch to 1 Mar 2000 for the leap-year algorithm
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Write the .pending file listing all likely_new entries.
fn write_pending(path: &Path, source: &Path, session_id: &str, entries: &[ScanEntry]) -> anyhow::Result<()> {
    use std::fmt::Write;
    let mut buf = String::new();
    writeln!(buf, "source={}", source.display())?;
    writeln!(buf, "session={session_id}")?;
    let new_count = entries.iter().filter(|e| e.status == FileStatus::LikelyNew).count();
    let dup_count = entries.iter().filter(|e| e.status == FileStatus::LikelyCacheDuplicate).count();
    writeln!(buf, "total={} new={} duplicate={}", entries.len(), new_count, dup_count)?;
    for e in entries.iter().filter(|e| e.status == FileStatus::LikelyNew) {
        writeln!(buf, "{}\t{}", e.src_path.display(), e.size)?;
    }
    fs::write(path, buf)?;
    Ok(())
}

/// Write the staging file listing all likely_new entries with their resolved
/// destination paths. Lives at `.svault/staging/import-<session>.txt`.
/// Format (plain text, one entry per line):
///   # source=<path>  session=<id>  total=N new=N duplicate=N
///   <src_path>\t<dest_path>\t<size>
fn write_staging(path: &Path, source: &Path, session_id: &str, entries: &[ScanEntry]) -> anyhow::Result<()> {
    use std::fmt::Write;
    let mut buf = String::new();
    let new_count = entries.iter().filter(|e| e.status == FileStatus::LikelyNew).count();
    let dup_count = entries.iter().filter(|e| e.status == FileStatus::LikelyCacheDuplicate).count();
    writeln!(buf, "# source={}  session={}  total={}  new={}  duplicate={}",
        source.display(), session_id, entries.len(), new_count, dup_count)?;
    for e in entries.iter().filter(|e| e.status == FileStatus::LikelyNew) {
        writeln!(buf, "{}\t{}", e.src_path.display(), e.size)?;
    }
    fs::write(path, buf)?;
    Ok(())
}

/// Write the final manifest file and return its path.
fn write_manifest(
    vault_root: &Path,
    session_id: &str,
    scan: &[ScanEntry],
    imported_dests: &[PathBuf],
) -> anyhow::Result<PathBuf> {
    let manifests_dir = vault_root.join(".svault").join("manifests");
    fs::create_dir_all(&manifests_dir)?;
    let manifest_path = manifests_dir.join(format!("import-{session_id}.txt"));
    use std::fmt::Write;
    let mut buf = String::new();
    writeln!(buf, "session={session_id}")?;
    writeln!(buf, "source=(multiple)")?;
    writeln!(buf, "total={} imported={} duplicate={}",
        scan.len(),
        imported_dests.len(),
        scan.iter().filter(|e| e.status == FileStatus::LikelyCacheDuplicate).count(),
    )?;
    for dest in imported_dests {
        writeln!(buf, "{}", dest.display())?;
    }
    fs::write(&manifest_path, buf)?;
    Ok(manifest_path)
}

