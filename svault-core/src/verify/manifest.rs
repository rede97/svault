//! Import manifest for source verification.
//!
//! This module provides functionality to record detailed import manifests
//! that can be used to verify source file integrity after import.

use std::collections::HashMap;
use std::fs;

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
    /// CRC32C hash (first 64KB)
    pub crc32c: u32,
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
    Recheck,
    Sync,
    Clone,
}

impl std::fmt::Display for SessionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SessionType::Import => "import",
            SessionType::Add => "add",
            SessionType::Update => "update",
            SessionType::Recheck => "recheck",
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

impl ImportManifest {
    /// Load manifest from file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let json = fs::read_to_string(path)?;
        let manifest: Self = serde_json::from_str(&json)?;
        Ok(manifest)
    }

    /// Find record by source path.
    pub fn find_by_src(&self, src_path: &Path) -> Option<&ImportRecord> {
        self.files.iter().find(|f| f.src_path == src_path)
    }

    /// Find record by destination path.
    pub fn find_by_dest(&self, dest_path: &Path) -> Option<&ImportRecord> {
        self.files
            .iter()
            .find(|f| f.dest_path.as_deref() == Some(dest_path))
    }

    /// Get files filtered by status.
    pub fn files_by_status(&self, status: ItemStatus) -> Vec<&ImportRecord> {
        self.files.iter().filter(|f| f.status == status).collect()
    }

    /// Calculate summary from files if not already set.
    pub fn calculate_summary(&self) -> ManifestSummary {
        if let Some(s) = self.summary {
            return s;
        }
        let mut summary = ManifestSummary {
            total: self.files.len(),
            added: 0,
            duplicate: 0,
            failed: 0,
            skipped: 0,
        };
        for f in &self.files {
            match f.status {
                ItemStatus::Added => summary.added += 1,
                ItemStatus::Duplicate => summary.duplicate += 1,
                ItemStatus::Failed => summary.failed += 1,
                ItemStatus::Skipped => summary.skipped += 1,
                _ => {}
            }
        }
        summary
    }

    /// Get all source paths.
    pub fn source_paths(&self) -> Vec<&Path> {
        self.files.iter().map(|f| f.src_path.as_ref()).collect()
    }
}

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

    /// Iterate all session directories (any kind) that contain a manifest.
    fn session_dirs_with_manifest(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        let root = crate::session::sessions_root(&self.vault_root);
        let Ok(kinds) = fs::read_dir(&root) else {
            return dirs;
        };
        for kind_entry in kinds.flatten() {
            let kind_dir = kind_entry.path();
            if !kind_dir.is_dir() {
                continue;
            }
            let Ok(sessions) = fs::read_dir(&kind_dir) else {
                continue;
            };
            for session_entry in sessions.flatten() {
                let dir = session_entry.path();
                if dir.is_dir() && dir.join(crate::session::MANIFEST_FILE).is_file() {
                    dirs.push(dir);
                }
            }
        }
        dirs
    }

    /// Load manifest by session ID or unique prefix.
    ///
    /// An exact directory-name match wins; otherwise the prefix must match
    /// exactly one session directory. Interrupted sessions (no manifest yet)
    /// and ambiguous prefixes produce a clear error.
    pub fn load(&self, session_prefix: &str) -> anyhow::Result<ImportManifest> {
        let dirs = self.session_dirs_with_manifest();
        let dir_name = |d: &PathBuf| {
            d.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };

        let exact: Vec<&PathBuf> = dirs.iter().filter(|d| dir_name(d) == session_prefix).collect();
        let matches: Vec<&PathBuf> = if exact.is_empty() {
            dirs.iter()
                .filter(|d| dir_name(d).starts_with(session_prefix))
                .collect()
        } else {
            exact
        };

        match matches.len() {
            0 => anyhow::bail!("no session manifest found matching '{session_prefix}'"),
            1 => ImportManifest::load(matches[0].join(crate::session::MANIFEST_FILE).as_path()),
            _ => {
                let names: Vec<String> = matches.iter().map(|d| dir_name(d)).collect();
                anyhow::bail!(
                    "session prefix '{session_prefix}' is ambiguous: {}",
                    names.join(", ")
                )
            }
        }
    }

    /// List all manifests (newest first).
    pub fn list_all(&self) -> anyhow::Result<Vec<(PathBuf, ImportManifest)>> {
        let mut manifests = Vec::new();
        for dir in self.session_dirs_with_manifest() {
            let path = dir.join(crate::session::MANIFEST_FILE);
            if let Ok(m) = ImportManifest::load(&path) {
                manifests.push((path, m));
                // Silently skip invalid manifests — core does not print to terminal.
            }
        }

        // Sort by import time (newest first)
        manifests.sort_by_key(|b| std::cmp::Reverse(b.1.imported_at));
        Ok(manifests)
    }

    /// Get the most recent manifest.
    pub fn latest(&self) -> anyhow::Result<Option<ImportManifest>> {
        let all = self.list_all()?;
        Ok(all.into_iter().next().map(|(_, m)| m))
    }

    /// Find manifest containing a specific destination path.
    pub fn find_by_dest(&self, dest_path: &Path) -> anyhow::Result<Option<ImportManifest>> {
        for (_, manifest) in self.list_all()? {
            if manifest.find_by_dest(dest_path).is_some() {
                return Ok(Some(manifest));
            }
        }
        Ok(None)
    }
}

/// Result of source verification.
#[derive(Debug, Clone)]
pub enum SourceVerifyResult {
    /// Source file unchanged (matches manifest).
    Unchanged,
    /// Source file modified (size or mtime different).
    Modified { reason: String },
    /// Source file deleted.
    Deleted,
    /// Source file is readable and matches vault copy.
    MatchesVault,
    /// Source file differs from vault copy.
    DiffersFromVault {
        vault_hash: String,
        source_hash: String,
    },
    /// Cannot read source file.
    IoError(String),
}

/// Verify source files against manifest.
pub fn verify_source_files(
    manifest: &ImportManifest,
    progress_fn: Option<impl Fn(&str)>,
) -> anyhow::Result<HashMap<PathBuf, SourceVerifyResult>> {
    use crate::hash::{sha256_file, xxh3_128_file};

    let mut results = HashMap::new();

    for record in &manifest.files {
        if let Some(ref callback) = progress_fn {
            callback(&record.src_path.to_string_lossy());
        }

        // Check if source exists
        if !record.src_path.exists() {
            results.insert(record.src_path.clone(), SourceVerifyResult::Deleted);
            continue;
        }

        // Get current metadata
        let meta = match fs::metadata(&record.src_path) {
            Ok(m) => m,
            Err(e) => {
                results.insert(
                    record.src_path.clone(),
                    SourceVerifyResult::IoError(e.to_string()),
                );
                continue;
            }
        };

        let current_size = meta.len();
        let current_mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        // Quick check: size and mtime
        if current_size != record.size {
            results.insert(
                record.src_path.clone(),
                SourceVerifyResult::Modified {
                    reason: format!("size changed: {} -> {}", record.size, current_size),
                },
            );
            continue;
        }

        if current_mtime != record.mtime_ms {
            // mtime changed but size same - may be metadata change
            // Need to check hash
        }

        // Compute current hash
        let result = match manifest.hash_algorithm.as_str() {
            "xxh3_128" => {
                if let Ok(hash) = xxh3_128_file(&record.src_path) {
                    let hash_str = format!("{:x}", hash);
                    if let Some(ref expected) = record.xxh3_128 {
                        if hash_str == *expected {
                            SourceVerifyResult::Unchanged
                        } else {
                            SourceVerifyResult::Modified {
                                reason: "hash mismatch".to_string(),
                            }
                        }
                    } else {
                        SourceVerifyResult::IoError("no hash in manifest".to_string())
                    }
                } else {
                    SourceVerifyResult::IoError("failed to compute hash".to_string())
                }
            }
            "sha256" => {
                if let Ok(hash) = sha256_file(&record.src_path) {
                    let hash_str = hash.to_hex();
                    if let Some(ref expected) = record.sha256 {
                        if hash_str == *expected {
                            SourceVerifyResult::Unchanged
                        } else {
                            SourceVerifyResult::Modified {
                                reason: "hash mismatch".to_string(),
                            }
                        }
                    } else {
                        SourceVerifyResult::IoError("no hash in manifest".to_string())
                    }
                } else {
                    SourceVerifyResult::IoError("failed to compute hash".to_string())
                }
            }
            _ => SourceVerifyResult::IoError(format!(
                "unknown hash algorithm: {}",
                manifest.hash_algorithm
            )),
        };

        results.insert(record.src_path.clone(), result);
    }

    Ok(results)
}
