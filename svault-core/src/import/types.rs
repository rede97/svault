//! Import pipeline types and options.

use std::path::PathBuf;

use crate::config::{ImportConfig, SyncStrategy};
use crate::config::HashAlgorithm;

/// Options controlling a single import run.
pub struct ImportOptions {
    /// Source directory to scan.
    pub source: PathBuf,
    /// Vault root directory (contains `.svault/`).
    pub vault_root: PathBuf,
    /// Hash algorithm to use for Stage D (strong hash).
    pub hash: HashAlgorithm,
    /// File transfer strategy.
    pub strategy: SyncStrategy,
    /// If true, scan and report but do not copy files or write to DB.
    pub dry_run: bool,
    /// If true, skip the interactive y/N confirmation after Stage B.
    pub yes: bool,
    /// Import configuration from `svault.toml`.
    pub import_config: ImportConfig,
    /// Force import even if the file is a confirmed duplicate.
    pub force: bool,
    /// Show duplicate files that were skipped during import.
    pub show_dup: bool,
    /// Optional file containing list of paths to import (one per line).
    /// If provided, source is treated as the base directory and only these files are imported.
    pub files_from: Option<PathBuf>,
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
    /// RAW unique ID for precise duplicate detection (camera serial + image ID)
    pub raw_unique_id: Option<String>,
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
