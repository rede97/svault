//! Vault configuration (`svault.toml`).

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const CONFIG_FILE: &str = "svault.toml";

/// Top-level configuration for a vault.
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub global: GlobalConfig,
    pub import: ImportConfig,
}

/// Global settings that apply to all operations.
#[derive(Debug, Serialize, Deserialize)]
pub struct GlobalConfig {
    /// Default hash algorithm used for collision resolution and verification
    /// across all operations (import, sync, add).
    /// Can be overridden per-command with --compare-level.
    ///   fast   - XXH3-128 (high throughput, non-cryptographic)
    ///   sha256 - SHA-256  (cryptographic strength)
    pub compare_level: CompareLevel,

    /// Default file-transfer strategy for sync operations.
    /// Can be overridden per-command with --strategy.
    ///   auto     - pick the best available strategy automatically
    ///   reflink  - copy-on-write (btrfs/xfs only)
    ///   hardlink - hardlink (same filesystem only)
    ///   copy     - stream copy (always works)
    #[serde(default)]
    pub sync_strategy: SyncStrategy,
}

/// Hash algorithm used for full-file comparison.
#[derive(Debug, Default, Serialize, Deserialize, Clone, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum CompareLevel {
    /// XXH3-128 (high throughput, non-cryptographic)
    Fast,
    /// SHA-256 (cryptographic strength, default)
    #[default]
    Sha256,
}

/// File-transfer strategy used during sync operations.
#[derive(Debug, Default, Serialize, Deserialize, Clone, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum SyncStrategy {
    /// Pick the best available strategy automatically (default).
    #[default]
    Auto,
    /// Use reflink / copy-on-write (btrfs/xfs only).
    Reflink,
    /// Use hardlinks (same filesystem only).
    Hardlink,
    /// Plain stream copy (always works).
    Copy,
}

/// Settings that control how files are imported.
#[derive(Debug, Serialize, Deserialize)]
pub struct ImportConfig {
    /// Template used when a filename conflict occurs during import.
    /// Supported placeholders: `$filename` (stem), `$ext` (extension with dot), `$n` (counter).
    /// Default: "$filename.$n.$ext"
    #[serde(default = "ImportConfig::default_rename_template")]
    pub rename_template: String,
    /// Path template for imported files, relative to the vault root.
    /// Supported placeholders:
    ///   $year   - 4-digit year from EXIF DateTimeOriginal (or file mtime)
    ///   $mon    - 2-digit month
    ///   $day    - 2-digit day
    ///   $device - camera model from EXIF Make/Model, or "Unknown Device"
    pub path_template: String,

    /// File extensions (lower-case, without leading dot) that are imported.
    /// Everything else is silently skipped.
    pub allowed_extensions: Vec<String>,
}

impl ImportConfig {
    fn default_rename_template() -> String {
        "$filename.$n.$ext".to_string()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            global: GlobalConfig {
                compare_level: CompareLevel::Sha256,
                sync_strategy: SyncStrategy::Auto,
            },
            import: ImportConfig {
                rename_template: ImportConfig::default_rename_template(),
                path_template: "$year/$mon-$day/$device".to_string(),
                allowed_extensions: vec![
                    // JPEG
                    "jpg".to_string(),
                    "jpeg".to_string(),
                    // Apple / HEIF
                    "heic".to_string(),
                    "heif".to_string(),
                    // Adobe DNG
                    "dng".to_string(),
                    // Canon
                    "cr2".to_string(),
                    "cr3".to_string(),
                    // Nikon
                    "nef".to_string(),
                    "nrw".to_string(),
                    // Sony
                    "arw".to_string(),
                    // Fujifilm
                    "raf".to_string(),
                    // Olympus / OM System
                    "orf".to_string(),
                    // Panasonic
                    "rw2".to_string(),
                    // Pentax
                    "pef".to_string(),
                    // Leica / Panasonic / Sigma
                    "raw".to_string(),
                    // Video
                    "mov".to_string(),
                    "mp4".to_string(),
                ],
            },
        }
    }
}

impl Config {
    /// Write the default `svault.toml` into `vault_root`.
    pub fn write_default(vault_root: &Path) -> anyhow::Result<()> {
        let cfg = Config::default();
        let content = toml::to_string_pretty(&cfg)?;
        let path = vault_root.join(CONFIG_FILE);
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Load `svault.toml` from `vault_root`.
    pub fn load(vault_root: &Path) -> anyhow::Result<Self> {
        let path = vault_root.join(CONFIG_FILE);
        let text = std::fs::read_to_string(&path)?;
        let cfg = toml::from_str(&text)?;
        Ok(cfg)
    }
}
