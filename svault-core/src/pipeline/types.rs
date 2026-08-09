//! Pipeline shared types for import/add commands.
//!
//! These types represent the data flow through pipeline stages:
//! FileEntry -> FingerprintEntry -> LookupResult -> HashResult -> InsertResult
//!
//! # Path Semantics
//!
//! This module carefully distinguishes between two types of paths:
//!
//! - **`path`** (in `FileEntry`, `HashResult`): The destination path within the vault
//!   (e.g., "2024/01-01/iPhone/photo.jpg"). For `import`, this is where the file is copied to;
//!   for `add`, this is the same as the source path since files are already in the vault.
//!
//! - **`src_path`** (in `FingerprintEntry`, `HashResult`, `CopyResult`): The original source path
//!   outside the vault (e.g., "/media/SDCARD/DCIM/photo.jpg"). Only populated during `import`
//!   command. Used for:
//!   - Writing accurate import manifests (recording where file came from)
//!   - Error reporting (showing user-friendly paths)
//!
//! ## Path Flow in Import Pipeline
//!
//! ```text
//! Stage A (scan):     path = source path (e.g., /media/SDCARD/DCIM/photo.jpg)
//!                     
//! Stage B (fingerprint):      file.path = source path
//!                     src_path = None (not needed yet)
//!                     
//! Stage C (copy):     src_path = source path
//!                     file.path = final vault path (e.g., 2024/01-01/iPhone/photo.jpg)
//!                     staged_path = staging copy under .svault/staging/import/<session>/
//!
//! Stage D (hash):     reads staged_path; path = final vault path
//!                     src_path = source path (preserved from Stage C)
//!
//! Stage E (insert):   DB: path (final vault path)
//!                     Manifest: src_path (source path)
//!                     After commit: staged_path atomically renamed to path
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

/// Stage B output: File entry with its XXH3-128 head/tail fingerprint.
#[derive(Debug, Clone)]
pub struct FingerprintEntry {
    /// File metadata (path is vault destination for import)
    pub file: FileEntry,
    /// Original source path (outside vault). Populated during import,
    /// None for add command since source equals destination.
    pub src_path: Option<PathBuf>,
    /// Staged copy location under the session staging dir.
    ///
    /// Set by Stage C (import only): the file is copied there first and
    /// only renamed to `file.path` after the DB transaction commits, so a
    /// partially copied file never appears at its final path. Stage D reads
    /// from this path. `None` for `add` (file is already in the vault).
    pub staged_path: Option<PathBuf>,
    /// XXH3-128 over format-specific head/tail regions (16 bytes).
    /// Fast duplicate triage only — NOT a full-file identity.
    pub fingerprint: Vec<u8>,
    pub raw_unique_id: Option<String>,
    /// Pre-computed strong hash (if --hash fast mode enabled).
    /// When present, Stage D can skip re-computing the hash.
    pub precomputed_hash: Option<Vec<u8>>,
}

impl FingerprintEntry {
    /// Create a new FingerprintEntry with precomputed_hash defaulting to None.
    pub fn new(file: FileEntry, fingerprint: Vec<u8>) -> Self {
        Self {
            file,
            src_path: None,
            staged_path: None,
            fingerprint,
            raw_unique_id: None,
            precomputed_hash: None,
        }
    }
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

/// Result of duplicate check with optional move detection.
/// Used by both `import` and `add` commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResult {
    /// New file, not in DB
    New,
    /// Exact duplicate (same content, same path or existing file)
    Duplicate,
    /// Vault-internal move (same content, different path, old file missing)
    /// Contains the old path from DB
    Moved { old_path: String },
    /// Recovery from missing state (DB has 'missing' status, re-import allowed)
    /// Contains the old path and file ID to update
    Recover { old_path: String, file_id: i64 },
}

/// Stage B2 output: Entry with duplicate lookup result.
#[derive(Debug, Clone)]
pub struct LookupResult {
    pub entry: FingerprintEntry,
    pub status: FileStatus,
}

/// Stage D output: Entry with strong hash computed.
///
/// Uses an enum to distinguish between fast (xxh3 only) and full (xxh3 + sha256) hashes.
/// This provides type-level guarantees about which hashes are available.
#[derive(Debug, Clone)]
pub struct HashResult {
    /// Vault destination path (where file was copied to).
    /// For import: "2024/01-01/iPhone/photo.jpg"
    /// For add: same as the original path since file is already in vault
    pub path: PathBuf,
    /// Original source path (where file was copied from).
    /// Only populated for import command; None for add command.
    /// Used for manifest recording.
    pub src_path: Option<PathBuf>,
    /// Staged copy location (import only), carried through from `FingerprintEntry`.
    /// Stage E renames it to `path` only after the DB transaction commits.
    pub staged_path: Option<PathBuf>,
    pub size: u64,
    pub mtime_ms: i64,
    /// Head/tail XXH3-128 fingerprint carried from Stage B (16 bytes).
    pub fingerprint: Vec<u8>,
    pub raw_unique_id: Option<String>,
    /// Hash identity - either fast (xxh3 only) or full (xxh3 + sha256)
    pub hash: FileHash,
    pub is_duplicate: bool,
    /// Duplicate detection reason (e.g. "db (xxh3_128)", "batch (sha256)").
    pub dup_reason: Option<String>,
    /// Hash computation error (e.g. IO error while reading the vault copy).
    ///
    /// Kept separate from `dup_reason` so error classification does not rely
    /// on fragile string prefix matching (formerly hash IO errors were
    /// misclassified as duplicates — BUG-1).
    pub hash_error: Option<String>,
}

/// File hash identity.
///
/// - `Fast`: XXH3-128 only (fast computation, used for deduplication)
/// - `Full`: XXH3-128 + SHA-256 (cryptographic identity, used when --full-id or --force)
#[derive(Debug, Clone)]
pub enum FileHash {
    /// Fast hash (XXH3-128) for deduplication.
    /// 16 bytes, fast to compute.
    Fast(Vec<u8>),
    /// Full hash with both XXH3-128 and SHA-256.
    /// XXH3-128 for dedup, SHA-256 for definitive identity.
    Full(Vec<u8>, Vec<u8>),
}

impl FileHash {
    /// Get the XXH3-128 hash bytes (always available).
    pub fn xxh3_128(&self) -> &[u8] {
        match self {
            FileHash::Fast(xxh3) => xxh3,
            FileHash::Full(xxh3, _) => xxh3,
        }
    }

    /// Get the SHA-256 hash bytes if available.
    pub fn sha256(&self) -> Option<&[u8]> {
        match self {
            FileHash::Fast(_) => None,
            FileHash::Full(_, sha256) => Some(sha256),
        }
    }

    /// Check if this is a full hash (has SHA-256).
    pub fn is_full(&self) -> bool {
        matches!(self, FileHash::Full(_, _))
    }

    /// Get the identity hash for database lookup.
    /// Returns SHA-256 if available (definitive), otherwise XXH3-128.
    pub fn identity(&self) -> (&[u8], HashAlgorithm) {
        match self {
            FileHash::Fast(xxh3) => (xxh3, HashAlgorithm::Xxh3_128),
            FileHash::Full(_, sha256) => (sha256, HashAlgorithm::Sha256),
        }
    }
}

use crate::config::HashAlgorithm;

/// Stage C output (import only): File copy result.
#[derive(Debug, Clone)]
pub struct CopyResult {
    /// Original source path outside vault
    pub src_path: PathBuf,
    /// Destination path within vault
    pub dest_path: PathBuf,
    pub size: u64,
    pub mtime_ms: i64,
    /// Head/tail XXH3-128 fingerprint (16 bytes).
    pub fingerprint: Vec<u8>,
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
    pub manifest_path: Option<PathBuf>,
    /// All files were cache hits (no new files)
    pub all_cache_hit: bool,
    /// `(staged, final)` pairs whose DB records were committed by Stage E.
    ///
    /// The caller MUST atomically rename each staged file to its final path
    /// (see [`crate::fs::atomic_commit`]); only then does the file become
    /// visible in the vault tree. Empty for `add`/`sync` (no staging).
    pub staged_commits: Vec<(PathBuf, PathBuf)>,
}

impl PipelineSummary {
    pub fn new(total: usize) -> Self {
        Self {
            total,
            ..Default::default()
        }
    }
}
