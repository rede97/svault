//! Staging files and manifest generation for import pipeline.

use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::Path;

use crate::import::types::{FileStatus, ScanEntry};
use console::style;

/// Write the .pending file listing all likely_new entries.
pub fn write_pending(
    path: &Path,
    source: &Path,
    session_id: &str,
    entries: &[ScanEntry],
) -> anyhow::Result<()> {
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
pub fn write_staging(
    path: &Path,
    source: &Path,
    session_id: &str,
    entries: &[ScanEntry],
) -> anyhow::Result<()> {
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
pub fn write_manifest(
    vault_root: &Path,
    session_id: &str,
    scan: &[ScanEntry],
    imported_dests: &[std::path::PathBuf],
) -> anyhow::Result<std::path::PathBuf> {
    let manifests_dir = vault_root.join(".svault").join("manifests");
    fs::create_dir_all(&manifests_dir)?;
    let manifest_path = manifests_dir.join(format!("import-{session_id}.txt"));
    
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
    
    eprintln!("{} {}",
        style("Manifest:").bold(),
        style(manifest_path.display()).dim());
    
    Ok(manifest_path)
}
