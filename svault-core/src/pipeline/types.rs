//! Pipeline shared types for import/add commands.
//!
//! These types represent the data flow through pipeline stages:
//! FileEntry -> CrcEntry -> LookupResult -> HashResult -> InsertResult

use std::path::PathBuf;

/// Stage A output: Basic file information from directory scan.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub mtime_ms: i64,
}

/// Stage B output: File entry with CRC32C fingerprint.
#[derive(Debug, Clone)]
pub struct CrcEntry {
    pub file: FileEntry,
    pub crc32c: u32,
    pub raw_unique_id: Option<String>,
}

/// File status after DB lookup (Stage B2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    /// New file, not in cache
    LikelyNew,
    /// Possible duplicate (cache hit)
    LikelyCacheDuplicate,
    /// Failed to process (e.g., CRC error)
    Failed(String),
}

/// Stage B2 output: Entry with duplicate lookup result.
#[derive(Debug, Clone)]
pub struct LookupResult {
    pub entry: CrcEntry,
    pub status: FileStatus,
}

/// Stage D output: Entry with strong hash computed.
#[derive(Debug, Clone)]
pub struct HashResult {
    pub path: PathBuf,
    pub size: u64,
    pub mtime_ms: i64,
    pub crc32c: u32,
    pub raw_unique_id: Option<String>,
    pub hash_bytes: Vec<u8>,
    pub is_duplicate: bool,
    pub dup_reason: Option<String>,
}

/// Stage C output (import only): File copy result.
#[derive(Debug, Clone)]
pub struct CopyResult {
    pub src_path: PathBuf,
    pub dest_path: PathBuf,
    pub size: u64,
    pub mtime_ms: i64,
    pub crc32c: u32,
    pub raw_unique_id: Option<String>,
}

/// Summary of pipeline execution.
#[derive(Debug, Default)]
pub struct PipelineSummary {
    pub total: usize,
    pub added: usize,
    pub duplicate: usize,
    pub skipped: usize,
    pub failed: usize,
    /// All files were cache hits (no new files)
    pub all_cache_hit: bool,
}

impl PipelineSummary {
    pub fn new(total: usize) -> Self {
        Self {
            total,
            ..Default::default()
        }
    }
}
