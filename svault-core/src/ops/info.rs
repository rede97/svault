//! `svault info` — inspect a single file: DB record, EXIF, ffprobe metadata.
//!
//! Pull model (see ARCHITECTURE.md §2.2): returns a serializable
//! [`InfoReport`]; rendering lives in `svault-ui`.
//!
//! Lookup modes: vault-relative path (or absolute path inside the vault)
//! via [`info_by_path`], or content hash via [`info_by_hash`] (full hex or
//! a unique prefix, matched against both `xxh3_128` and `sha256`).
//!
//! Video metadata comes from the external `ffprobe` binary (JSON output).
//! When ffprobe is not installed the report degrades gracefully:
//! `video` is `None` and `ffprobe_available` is `false`.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::db::Db;

/// The DB record portion of an info report.
#[derive(Debug, Clone, Serialize)]
pub struct DbFacts {
    pub path: String,
    pub size: i64,
    pub mtime: i64,
    pub xxh3_128: Option<String>,
    pub sha256: Option<String>,
    pub status: String,
}

/// File info report.
#[derive(Debug, Clone, Serialize)]
pub struct InfoReport {
    /// The resolved absolute path on disk.
    pub path: PathBuf,
    /// Whether the file exists on disk right now.
    pub on_disk: bool,
    /// The DB record (None when the file is not tracked).
    pub db: Option<DbFacts>,
    /// EXIF fields as (tag, display value) pairs (images; empty otherwise).
    pub exif: Vec<(String, String)>,
    /// ffprobe output for video files: `{"format": ..., "streams": [...]}`.
    pub video: Option<serde_json::Value>,
    /// False when ffprobe was needed but is not installed.
    pub ffprobe_available: bool,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn db_facts(row: &crate::db::FileRow) -> DbFacts {
    DbFacts {
        path: row.path.clone(),
        size: row.size,
        mtime: row.mtime,
        xxh3_128: row.xxh3_128.as_deref().map(hex),
        sha256: row.sha256.as_deref().map(hex),
        status: row.status.clone(),
    }
}

/// Read all EXIF fields as display pairs. Returns an empty list for
/// non-EXIF files (videos, unknown formats).
fn read_exif_fields(path: &Path) -> Vec<(String, String)> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut reader = std::io::BufReader::new(file);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut reader) else {
        return Vec::new();
    };
    exif.fields()
        .map(|f| {
            (
                format!("{}", f.tag),
                f.display_value().with_unit(&exif).to_string(),
            )
        })
        .collect()
}

/// Probe a video file with ffprobe; `None` when ffprobe is unavailable or
/// the file cannot be probed.
fn ffprobe(path: &Path) -> Option<serde_json::Value> {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

fn is_video(path: &Path) -> bool {
    matches!(
        crate::media::MediaFormat::from_path(path),
        Ok(crate::media::MediaFormat::Mov
            | crate::media::MediaFormat::Mp4
            | crate::media::MediaFormat::Avi
            | crate::media::MediaFormat::Mkv)
    )
}

fn build_report(vault_root: &Path, row: Option<crate::db::FileRow>, rel: &str) -> InfoReport {
    let abs = vault_root.join(rel);
    let on_disk = abs.is_file();
    let video = if on_disk && is_video(&abs) {
        ffprobe(&abs)
    } else {
        None
    };
    InfoReport {
        on_disk,
        ffprobe_available: !is_video(&abs) || video.is_some() || ffprobe_present(),
        video,
        exif: if on_disk && !is_video(&abs) {
            read_exif_fields(&abs)
        } else {
            Vec::new()
        },
        db: row.as_ref().map(db_facts),
        path: abs,
    }
}

fn ffprobe_present() -> bool {
    std::process::Command::new("ffprobe")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Info by file path (vault-relative DB path, or absolute path inside the vault).
pub fn info_by_path(db: &Db, vault_root: &Path, input: &str) -> anyhow::Result<InfoReport> {
    let p = Path::new(input);
    let rel = if p.is_absolute() {
        p.strip_prefix(vault_root).unwrap_or(p)
    } else {
        p
    };
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    let row = db.get_file_by_path(&rel_str)?;
    Ok(build_report(vault_root, row, &rel_str))
}

/// Info by content hash: full hex or unique prefix, matched against both
/// `xxh3_128` and `sha256`.
pub fn info_by_hash(db: &Db, vault_root: &Path, hash: &str) -> anyhow::Result<InfoReport> {
    let needle = hash.trim().to_lowercase();
    if needle.len() < 4 || !needle.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("hash must be at least 4 hex characters");
    }

    let matches: Vec<crate::db::FileRow> = db
        .get_all_files()?
        .into_iter()
        .filter(|row| {
            [&row.xxh3_128, &row.sha256]
                .into_iter()
                .flatten()
                .any(|h| hex(h).starts_with(&needle))
        })
        .collect();

    match matches.len() {
        0 => anyhow::bail!("no file matches hash '{hash}'"),
        1 => {
            let row = matches.into_iter().next().unwrap();
            let rel = row.path.clone();
            Ok(build_report(vault_root, Some(row), &rel))
        }
        _ => anyhow::bail!(
            "hash prefix '{hash}' is ambiguous ({} matches); use a longer prefix",
            matches.len()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db_with_two_files() -> Db {
        let db = Db::open_in_memory().unwrap();
        db.insert_file_row(
            "2026/a.jpg",
            10,
            0,
            None,
            None,
            Some(&[0xAA; 16]),
            None,
            "imported",
            0,
        )
        .unwrap();
        db.insert_file_row(
            "2026/b.jpg",
            10,
            0,
            None,
            None,
            Some(&[0xBB; 16]),
            None,
            "imported",
            0,
        )
        .unwrap();
        db
    }

    #[test]
    fn hash_lookup_full_and_prefix() {
        let db = db_with_two_files();
        let tmp = tempfile::tempdir().unwrap();

        let full = "aa".repeat(16);
        let report = info_by_hash(&db, tmp.path(), &full).unwrap();
        assert_eq!(report.db.unwrap().path, "2026/a.jpg");

        // aa… / bb… 前缀各自唯一
        let report = info_by_hash(&db, tmp.path(), "bbbb").unwrap();
        assert_eq!(report.db.unwrap().path, "2026/b.jpg");
    }

    #[test]
    fn hash_lookup_errors() {
        let db = db_with_two_files();
        let tmp = tempfile::tempdir().unwrap();

        assert!(info_by_hash(&db, tmp.path(), "cccc").is_err());
        assert!(info_by_hash(&db, tmp.path(), "zz").is_err());
        assert!(info_by_hash(&db, tmp.path(), "abc").is_err()); // too short
    }
}
