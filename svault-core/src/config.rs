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
    ///   reflink  - copy-on-write (btrfs/xfs only)
    ///   hardlink - hardlink (same filesystem only)
    ///   copy     - stream copy (always works)
    /// Multiple strategies can be combined: ["reflink", "hardlink"]
    #[serde(default)]
    pub sync_strategy: SyncStrategy,
}

/// Hash algorithm used for file identity and deduplication.
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "cli", derive(ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum HashAlgorithm {
    /// Fast hash (XXH3-128, default)
    #[default]
    #[cfg_attr(feature = "cli", clap(name = "fast"))]
    Xxh3_128,
    /// Secure hash (SHA-256)
    #[cfg_attr(feature = "cli", clap(name = "secure"))]
    Sha256,
}

impl std::fmt::Display for HashAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HashAlgorithm::Xxh3_128 => write!(f, "fast"),
            HashAlgorithm::Sha256 => write!(f, "secure"),
        }
    }
}

/// A single strategy argument that can appear in a `--strategy` list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "cli", derive(ValueEnum))]
#[cfg_attr(feature = "cli", clap(rename_all = "snake_case"))]
pub enum TransferStrategyArg {
    /// Copy-on-write clone (btrfs, xfs, APFS, ReFS).
    Reflink,
    /// Hard link (same filesystem only).
    Hardlink,
    /// Plain stream copy (always works).
    Copy,
}

impl TransferStrategyArg {
    pub fn to_transfer_strategy(&self) -> crate::vfs::TransferStrategy {
        match self {
            TransferStrategyArg::Reflink => crate::vfs::TransferStrategy::Reflink,
            TransferStrategyArg::Hardlink => crate::vfs::TransferStrategy::Hardlink,
            TransferStrategyArg::Copy => crate::vfs::TransferStrategy::StreamCopy,
        }
    }
}

impl Serialize for TransferStrategyArg {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let s = match self {
            TransferStrategyArg::Reflink => "reflink",
            TransferStrategyArg::Hardlink => "hardlink",
            TransferStrategyArg::Copy => "copy",
        };
        serializer.serialize_str(s)
    }
}

impl<'de> Deserialize<'de> for TransferStrategyArg {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?.to_lowercase();
        match s.as_str() {
            "reflink" => Ok(TransferStrategyArg::Reflink),
            "hardlink" => Ok(TransferStrategyArg::Hardlink),
            "copy" => Ok(TransferStrategyArg::Copy),
            _ => Err(serde::de::Error::custom(format!("unknown strategy: {}", s))),
        }
    }
}

/// File-transfer strategy used during sync/import operations.
///
/// Defaults to `[Reflink]`. `StreamCopy` is always attempted as the final
/// fallback so that transfers never fail completely.
#[derive(Debug, Clone)]
pub struct SyncStrategy(pub Vec<TransferStrategyArg>);

impl SyncStrategy {
    /// Resolve this strategy into a list of concrete [`TransferStrategy`]
    /// values to be attempted in order.
    pub fn to_transfer_strategies(&self) -> Vec<crate::vfs::TransferStrategy> {
        self.0.iter().map(|a| a.to_transfer_strategy()).collect()
    }
}

impl Default for SyncStrategy {
    fn default() -> Self {
        Self(vec![TransferStrategyArg::Reflink])
    }
}

impl Serialize for SyncStrategy {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for arg in &self.0 {
            seq.serialize_element(arg)?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for SyncStrategy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{self, Visitor};
        use std::fmt;

        struct SyncStrategyVisitor;

        impl<'de> Visitor<'de> for SyncStrategyVisitor {
            type Value = SyncStrategy;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a strategy string or a list of strategy strings")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let strategies: Result<Vec<_>, _> = value
                    .split(',')
                    .map(|s| {
                        TransferStrategyArg::deserialize(serde::de::value::StrDeserializer::<'_, serde::de::value::Error>::new(s.trim()))
                            .map_err(de::Error::custom)
                    })
                    .collect();
                Ok(SyncStrategy(strategies?))
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let mut strategies = Vec::new();
                while let Some(arg) = seq.next_element::<TransferStrategyArg>()? {
                    strategies.push(arg);
                }
                Ok(SyncStrategy(strategies))
            }
        }

        deserializer.deserialize_any(SyncStrategyVisitor)
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
                sync_strategy: SyncStrategy::default(),
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
