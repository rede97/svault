//! Import manifest for source verification.
//!
//! This module provides functionality to record detailed import manifests
//! that can be used to verify source file integrity after import.

use std::collections::HashMap;
use std::fs;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Detailed import record for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRecord {
    /// Source file path (absolute)
    pub src_path: PathBuf,
    /// Destination path in vault (relative)
    pub dest_path: PathBuf,
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
}

/// Import session manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportManifest {
    /// Session ID
    pub session_id: String,
    /// Source directory
    pub source_root: PathBuf,
    /// Import timestamp
    pub imported_at: i64,
    /// Hash algorithm used
    pub hash_algorithm: String,
    /// Imported files
    pub files: Vec<ImportRecord>,
}

impl ImportManifest {
    /// Save manifest to file (JSON format).
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

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
        self.files.iter().find(|f| f.dest_path == dest_path)
    }

    /// Get all source paths.
    pub fn source_paths(&self) -> Vec<&Path> {
        self.files.iter().map(|f| f.src_path.as_ref()).collect()
    }
}

/// Manifest manager for a vault.
pub struct ManifestManager {
    manifests_dir: PathBuf,
}

impl ManifestManager {
    /// Create manager for vault root.
    pub fn new(vault_root: &Path) -> Self {
        Self {
            manifests_dir: vault_root.join(".svault").join("manifests"),
        }
    }

    /// Ensure manifests directory exists.
    fn ensure_dir(&self) -> anyhow::Result<()> {
        fs::create_dir_all(&self.manifests_dir)?;
        Ok(())
    }

    /// Save manifest.
    pub fn save(&self, manifest: &ImportManifest) -> anyhow::Result<PathBuf> {
        self.ensure_dir()?;
        let path = self.manifests_dir.join(format!("import-{}.json", manifest.session_id));
        manifest.save(&path)?;
        Ok(path)
    }

    /// Load manifest by session ID.
    pub fn load(&self, session_id: &str) -> anyhow::Result<ImportManifest> {
        let path = self.manifests_dir.join(format!("import-{session_id}.json"));
        ImportManifest::load(&path)
    }

    /// List all manifests (newest first).
    pub fn list_all(&self) -> anyhow::Result<Vec<(PathBuf, ImportManifest)>> {
        self.ensure_dir()?;
        let mut manifests = Vec::new();

        for entry in fs::read_dir(&self.manifests_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                match ImportManifest::load(&path) {
                    Ok(m) => manifests.push((path, m)),
                    Err(e) => eprintln!("Warning: failed to load manifest {}: {}", path.display(), e),
                }
            }
        }

        // Sort by import time (newest first)
        manifests.sort_by(|a, b| b.1.imported_at.cmp(&a.1.imported_at));
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
    DiffersFromVault { vault_hash: String, source_hash: String },
    /// Cannot read source file.
    IoError(String),
}

/// Verify source files against manifest.
pub fn verify_source_files(
    manifest: &ImportManifest,
    progress_fn: Option<impl Fn(&str)>,
) -> anyhow::Result<HashMap<PathBuf, SourceVerifyResult>> {
    use crate::hash::{xxh3_128_file, sha256_file};

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
