//! Vault configuration (`svault.toml`).

#[cfg(feature = "cli")]
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
    /// Hash algorithm used for file identity, deduplication, and verification.
    /// Applies to all operations (import, sync, add, verify).
    /// Can be overridden per-command with -H / --hash.
    ///   xxh3_128 - XXH3-128 (high throughput, non-cryptographic, default)
    ///   sha256   - SHA-256  (cryptographic strength)
    #[serde(default)]
    pub hash: HashAlgorithm,

    /// Default file-transfer strategy for sync operations.
    /// Can be overridden per-command with --strategy.
    ///   auto     - pick the best available strategy automatically
    ///   reflink  - copy-on-write (btrfs/xfs only)
    ///   hardlink - hardlink (same filesystem only)
    ///   copy     - stream copy (always works)
    #[serde(default)]
    pub sync_strategy: SyncStrategy,
}

/// Hash algorithm used for file identity and deduplication.
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "cli", derive(ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum HashAlgorithm {
    /// XXH3-128 (high throughput, non-cryptographic, default)
    #[default]
    Xxh3_128,
    /// SHA-256 (cryptographic strength)
    Sha256,
}

impl std::fmt::Display for HashAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HashAlgorithm::Xxh3_128 => write!(f, "xxh3_128"),
            HashAlgorithm::Sha256 => write!(f, "sha256"),
        }
    }
}



/// File-transfer strategy used during sync operations.
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "cli", derive(ValueEnum))]
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

impl SyncStrategy {
    /// Resolve this strategy into a concrete [`TransferStrategy`].
    pub fn to_transfer_strategy(
        &self,
        src_caps: &crate::vfs::FsCapabilities,
        dst_caps: &crate::vfs::FsCapabilities,
    ) -> crate::vfs::TransferStrategy {
        match self {
            SyncStrategy::Auto => src_caps.best_strategy(dst_caps),
            SyncStrategy::Reflink => crate::vfs::TransferStrategy::Reflink,
            SyncStrategy::Hardlink => crate::vfs::TransferStrategy::Hardlink,
            SyncStrategy::Copy => crate::vfs::TransferStrategy::StreamCopy,
        }
    }
}

/// Settings that control how files are imported.
#[derive(Debug, Serialize, Deserialize)]
pub struct ImportConfig {
    /// Store full EXIF metadata in file_exif table (default: false).
    #[serde(default)]
    pub store_exif: bool,

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

impl Default for ImportConfig {
    fn default() -> Self {
        Self {
            store_exif: false,
            rename_template: "$filename.$n.$ext".to_string(),
            path_template: "$year/$mon-$day/$device/$filename".to_string(),
            allowed_extensions: vec![
                "jpg".to_string(), "jpeg".to_string(),
                "heic".to_string(), "heif".to_string(),
                "dng".to_string(),
                "cr2".to_string(), "cr3".to_string(),
                "nef".to_string(), "nrw".to_string(),
                "arw".to_string(),
                "raf".to_string(),
                "orf".to_string(),
                "rw2".to_string(),
                "pef".to_string(),
                "iiq".to_string(),
                "png".to_string(), "tiff".to_string(), "tif".to_string(),
                "mp4".to_string(), "mov".to_string(), "avi".to_string(), "mkv".to_string(),
            ],
        }
    }
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
                hash: HashAlgorithm::Xxh3_128,
                sync_strategy: SyncStrategy::Auto,
            },
            import: ImportConfig {
                store_exif: false,
                rename_template: ImportConfig::default_rename_template(),
                path_template: "$year/$mon-$day/$device/$filename".to_string(),
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
