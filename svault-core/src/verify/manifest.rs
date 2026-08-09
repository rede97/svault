//! Import manifest for source verification.
//!
//! This module provides functionality to record detailed import manifests
//! that can be used to verify source file integrity after import.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Item status in manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    #[default]
    /// Successfully imported/added
    Added,
    /// Duplicate (already exists)
    Duplicate,
    /// Failed (hash error or other issue)
    Failed,
    /// Skipped (force not set, already tracked)
    Skipped,
    /// Missing (detected during update)
    Missing,
    /// Moved (detected during update)
    Moved,
    /// Relinked (hardlink converted to copy)
    Relinked,
    /// Unchanged (update check passed)
    Unchanged,
}

impl std::fmt::Display for ItemStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ItemStatus::Added => "added",
            ItemStatus::Duplicate => "duplicate",
            ItemStatus::Failed => "failed",
            ItemStatus::Skipped => "skipped",
            ItemStatus::Missing => "missing",
            ItemStatus::Moved => "moved",
            ItemStatus::Relinked => "relinked",
            ItemStatus::Unchanged => "unchanged",
        };
        write!(f, "{}", s)
    }
}

/// Detailed import record for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRecord {
    /// Source file path (absolute)
    pub src_path: PathBuf,
    /// Destination path in vault (relative), may be empty for failed/duplicate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest_path: Option<PathBuf>,
    /// File size in bytes
    pub size: u64,
    /// Modification time (Unix timestamp ms)
    pub mtime_ms: i64,
    /// Head/tail XXH3-128 fingerprint as hex (format-specific regions)
    pub fingerprint: String,
    /// XXH3-128 hash (if computed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xxh3_128: Option<String>,
    /// SHA-256 hash (if computed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Import timestamp
    pub imported_at: i64,
    /// Item status
    #[serde(default)]
    pub status: ItemStatus,
    /// Error message for failed items
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Session type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionType {
    #[default]
    Import,
    Add,
    Update,
    Sync,
    Clone,
}

impl std::fmt::Display for SessionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SessionType::Import => "import",
            SessionType::Add => "add",
            SessionType::Update => "update",
            SessionType::Sync => "sync",
            SessionType::Clone => "clone",
        };
        write!(f, "{}", s)
    }
}

/// Import session manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportManifest {
    /// Session ID
    pub session_id: String,
    /// Session type
    #[serde(default)]
    pub session_type: SessionType,
    /// Source directory
    pub source_root: PathBuf,
    /// Import timestamp
    pub imported_at: i64,
    /// Hash algorithm used
    pub hash_algorithm: String,
    /// All files in this session (including duplicate/failed)
    pub files: Vec<ImportRecord>,
    /// Summary counts
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<ManifestSummary>,
}

/// Summary counts for manifest.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ManifestSummary {
    pub total: usize,
    pub added: usize,
    pub duplicate: usize,
    pub failed: usize,
    pub skipped: usize,
}

impl ImportManifest {}

/// Manifest manager for a vault.
///
/// Manifests live inside session journal directories:
/// `.svault/sessions/<kind>/<ts-id>/manifest.json` (see [`crate::session`]).
pub struct ManifestManager {
    vault_root: PathBuf,
}

impl ManifestManager {
    /// Create manager for vault root.
    pub fn new(vault_root: &Path) -> Self {
        Self {
            vault_root: vault_root.to_path_buf(),
        }
    }

    /// Save manifest into its session directory
    /// (`sessions/<type>/<session_id>/manifest.json`), atomically.
    pub fn save(&self, manifest: &ImportManifest) -> anyhow::Result<PathBuf> {
        let dir = crate::session::session_dir(
            &self.vault_root,
            manifest.session_type,
            &manifest.session_id,
        );
        let path = dir.join(crate::session::MANIFEST_FILE);
        crate::session::write_json_atomic(&path, manifest)?;
        Ok(path)
    }
}
