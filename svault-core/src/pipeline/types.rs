//! Pipeline shared types for import/add commands.
//!
//! These types represent the data flow through pipeline stages:
//! FileEntry -> CrcEntry -> LookupResult -> HashResult -> InsertResult
//!
//! # Path Semantics
//!
//! This module carefully distinguishes between two types of paths:
//!
//! - **`path`** (in `FileEntry`, `HashResult`): The destination path within the vault
//!   (e.g., "2024/01-01/iPhone/photo.jpg"). For `import`, this is where the file is copied to;
//!   for `add`, this is the same as the source path since files are already in the vault.
//!
//! - **`src_path`** (in `CrcEntry`, `HashResult`, `CopyResult`): The original source path
//!   outside the vault (e.g., "/media/SDCARD/DCIM/photo.jpg"). Only populated during `import`
//!   command. Used for:
//!   - Writing accurate import manifests (recording where file came from)
//!   - Recheck operations (verifying source file integrity)
//!   - Error reporting (showing user-friendly paths)
//!
//! ## Path Flow in Import Pipeline
//!
//! ```text
//! Stage A (scan):     path = source path (e.g., /media/SDCARD/DCIM/photo.jpg)
//!                     
//! Stage B (crc):      file.path = source path
//!                     src_path = None (not needed yet)
//!                     
//! Stage C (copy):     src_path = source path
//!                     dest_path = vault path (e.g., 2024/01-01/iPhone/photo.jpg)
//!                     
//! Stage D (hash):     path = vault path
//!                     src_path = source path (preserved from Stage C)
//!                     
//! Stage E (insert):   DB: path (vault path)
//!                     Manifest: src_path (source path for recheck)
//! ```
//!
//! ## Path Flow in Add Pipeline
//!
//! ```text
//! Stage A (scan):     path = vault path (file already in vault)
//!
//! Stage B-D:          src_path = None (source is same as destination)
//!
//! Stage E (insert):   DB: path (vault path)
//!                     No manifest written (add has no external source)
//! ```

use std::path::PathBuf;

/// Stage A output: Basic file information from directory scan.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// File path - source path during scan, vault path after copy
    pub path: PathBuf,
    pub size: u64,
    pub mtime_ms: i64,
}

/// Stage B output: File entry with CRC32C fingerprint.
#[derive(Debug, Clone)]
pub struct CrcEntry {
    /// File metadata (path is vault destination for import)
    pub file: FileEntry,
    /// Original source path (outside vault). Populated during import,
    /// None for add command since source equals destination.
    pub src_path: Option<PathBuf>,
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
    /// Vault destination path (where file was copied to).
    /// For import: "2024/01-01/iPhone/photo.jpg"
    /// For add: same as the original path since file is already in vault
    pub path: PathBuf,
    /// Original source path (where file was copied from).
    /// Only populated for import command; None for add command.
    /// Used for manifest recording and recheck operations.
    pub src_path: Option<PathBuf>,
    pub size: u64,
    pub mtime_ms: i64,
    pub crc32c: u32,
    pub raw_unique_id: Option<String>,
    /// Hash bytes - format depends on algorithm:
    /// - XXH3-128: 16 bytes (little-endian)
    /// - SHA-256: 32 bytes
    pub hash_bytes: Vec<u8>,
    pub is_duplicate: bool,
    pub dup_reason: Option<String>,
}

/// Stage C output (import only): File copy result.
#[derive(Debug, Clone)]
pub struct CopyResult {
    /// Original source path outside vault
    pub src_path: PathBuf,
    /// Destination path within vault
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
