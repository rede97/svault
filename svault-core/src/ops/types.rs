//! Import pipeline types and options.

use std::path::PathBuf;

use serde::Serialize;

use crate::config::{ImportConfig, SyncStrategy};

/// Options controlling a single import run.
pub struct ImportOptions {
    /// Source directory to scan.
    pub source: PathBuf,
    /// Vault root directory (contains `.svault/`).
    pub vault_root: PathBuf,
    /// File transfer strategy.
    pub strategy: SyncStrategy,
    /// If true, scan and report but do not copy files or write to DB.
    pub dry_run: bool,
    /// If true, skip the interactive y/N confirmation after Stage B.
    pub yes: bool,
    /// Import configuration from `svault.toml`.
    pub import_config: ImportConfig,
    /// Force import even if the file is a confirmed duplicate.
    /// Also computes SHA-256 for definitive identity.
    pub force: bool,
    /// Compute SHA-256 for definitive identity verification.
    /// When present, SHA-256 serves as the definitive file identity.
    pub full_id: bool,
    /// Show duplicate files that were skipped during import.
    pub show_dup: bool,
    /// Pre-parsed list of paths to import.
    ///
    /// When `Some`, the source directory is not scanned; only these paths are processed.
    /// The CLI layer is responsible for reading and parsing any file-list input before
    /// constructing `ImportOptions`.
    /// When `None`, `source` is scanned recursively.
    pub files_from: Option<Vec<PathBuf>>,
    /// Maximum scan depth below the source root: 0 = unlimited (default),
    /// 1 = only files directly inside the source directory.
    /// Ignored when `files_from` is set.
    pub max_depth: usize,
    /// Include globs matched against the source-relative path
    /// (case-insensitive; empty = everything, subject to `allowed_extensions`).
    /// Ignored when `files_from` is set.
    pub include: Vec<String>,
    /// Exclude globs; exclusions win over inclusions.
    /// Ignored when `files_from` is set.
    pub exclude: Vec<String>,
    /// How thoroughly fingerprint-suspected duplicates are verified
    /// against the DB (see [`CompareLevel`]). Default: fast.
    pub compare_level: CompareLevel,
}

/// Final summary returned to the caller.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportSummary {
    pub total: usize,
    pub imported: usize,
    pub duplicate: usize,
    pub failed: usize,
    pub manifest_path: Option<PathBuf>,
    /// Set when all files were cache hits and import exited early.
    pub all_cache_hit: bool,
}

/// How thoroughly `import` verifies fingerprint-suspected duplicates.
///
/// The CRC32C fingerprint only reads head/tail regions, so a hit means
/// "probably identical". Higher levels re-hash the source file and compare
/// against the DB record, replacing the removed `recheck` command's
/// source-audit role: re-running `import --compare-level mid|high` verifies
/// suspected duplicates instead of trusting the fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompareLevel {
    /// Fingerprint only (size + CRC32C + extension). Hit = identical.
    #[default]
    Fast,
    /// CRC hits are re-verified with a full XXH3-128 of the source file.
    Mid,
    /// CRC hits are re-verified with SHA-256 when the DB record has one,
    /// otherwise with XXH3-128.
    High,
}
