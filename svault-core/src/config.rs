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
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
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
    pub fn to_transfer_strategy(&self) -> crate::fs::TransferStrategy {
        match self {
            TransferStrategyArg::Reflink => crate::fs::TransferStrategy::Reflink,
            TransferStrategyArg::Hardlink => crate::fs::TransferStrategy::Hardlink,
            TransferStrategyArg::Copy => crate::fs::TransferStrategy::StreamCopy,
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
    pub fn to_transfer_strategies(&self) -> Vec<crate::fs::TransferStrategy> {
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
                        TransferStrategyArg::deserialize(serde::de::value::StrDeserializer::<
                            '_,
                            serde::de::value::Error,
                        >::new(s.trim()))
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
                "jpg".to_string(),
                "jpeg".to_string(),
                "heic".to_string(),
                "heif".to_string(),
                "dng".to_string(),
                "cr2".to_string(),
                "cr3".to_string(),
                "nef".to_string(),
                "nrw".to_string(),
                "arw".to_string(),
                "raf".to_string(),
                "orf".to_string(),
                "rw2".to_string(),
                "pef".to_string(),
                "iiq".to_string(),
                "png".to_string(),
                "tiff".to_string(),
                "tif".to_string(),
                "mp4".to_string(),
                "mov".to_string(),
                "avi".to_string(),
                "mkv".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Default config tests
    // -------------------------------------------------------------------------

    #[test]
    fn default_config_has_expected_values() {
        let cfg = Config::default();

        // Global config defaults
        assert_eq!(cfg.global.sync_strategy.0.len(), 1);
        assert!(matches!(
            cfg.global.sync_strategy.0[0],
            TransferStrategyArg::Reflink
        ));

        // Import config defaults
        assert!(!cfg.import.store_exif);
        assert_eq!(cfg.import.rename_template, "$filename.$n.$ext");
        assert_eq!(
            cfg.import.path_template,
            "$year/$mon-$day/$device/$filename"
        );

        // Should have default extensions
        assert!(!cfg.import.allowed_extensions.is_empty());
        assert!(cfg.import.allowed_extensions.contains(&"jpg".to_string()));
        assert!(cfg.import.allowed_extensions.contains(&"mp4".to_string()));
    }

    #[test]
    fn default_extensions_include_common_formats() {
        let cfg = Config::default();
        let exts = &cfg.import.allowed_extensions;

        // Image formats
        assert!(exts.contains(&"jpg".to_string()));
        assert!(exts.contains(&"jpeg".to_string()));
        assert!(exts.contains(&"heic".to_string()));

        // RAW formats
        assert!(exts.contains(&"dng".to_string()));
        assert!(exts.contains(&"cr2".to_string()));
        assert!(exts.contains(&"arw".to_string()));

        // Video formats
        assert!(exts.contains(&"mp4".to_string()));
        assert!(exts.contains(&"mov".to_string()));
    }

    // -------------------------------------------------------------------------
    // TOML serialization tests
    // -------------------------------------------------------------------------

    #[test]
    fn config_serializes_to_valid_toml() {
        let cfg = Config::default();
        let toml_str = toml::to_string_pretty(&cfg).expect("should serialize");

        // Should contain expected sections
        assert!(toml_str.contains("[global]"));
        assert!(toml_str.contains("[import]"));

        // Should contain default values (hash field removed from config model)
        assert!(toml_str.contains("sync_strategy"));
        assert!(toml_str.contains("allowed_extensions"));
    }

    #[test]
    fn config_roundtrips_through_toml() {
        let original = Config::default();
        let toml_str = toml::to_string_pretty(&original).unwrap();
        let loaded: Config = toml::from_str(&toml_str).expect("should deserialize");

        // Verify key values roundtrip
        assert_eq!(
            loaded.import.rename_template,
            original.import.rename_template
        );
        assert_eq!(
            loaded.import.allowed_extensions,
            original.import.allowed_extensions
        );
    }

    // -------------------------------------------------------------------------
    // TOML deserialization tests - valid configs
    // -------------------------------------------------------------------------

    #[test]
    fn parses_minimal_valid_config() {
        // Note: [global] section is now required (even if empty)
        let toml = r#"
[global]

[import]
path_template = "$year/$filename"
allowed_extensions = ["jpg", "png"]
"#;

        let cfg: Config = toml::from_str(toml).expect("should parse minimal config");
        assert_eq!(cfg.import.path_template, "$year/$filename");
        assert_eq!(cfg.import.allowed_extensions, vec!["jpg", "png"]);
    }

    #[test]
    fn parses_config_with_sync_strategy_list() {
        let toml = r#"
[global]
sync_strategy = ["reflink", "hardlink", "copy"]

[import]
path_template = "$year/$filename"
allowed_extensions = ["jpg"]
"#;

        let cfg: Config = toml::from_str(toml).expect("should parse strategy list");
        assert_eq!(cfg.global.sync_strategy.0.len(), 3);
        assert!(matches!(
            cfg.global.sync_strategy.0[0],
            TransferStrategyArg::Reflink
        ));
        assert!(matches!(
            cfg.global.sync_strategy.0[1],
            TransferStrategyArg::Hardlink
        ));
        assert!(matches!(
            cfg.global.sync_strategy.0[2],
            TransferStrategyArg::Copy
        ));
    }

    #[test]
    fn parses_config_with_sync_strategy_comma_string() {
        let toml = r#"
[global]
sync_strategy = "reflink,hardlink"

[import]
path_template = "$year/$filename"
allowed_extensions = ["jpg"]
"#;

        let cfg: Config = toml::from_str(toml).expect("should parse comma-separated strategy");
        assert_eq!(cfg.global.sync_strategy.0.len(), 2);
        assert!(matches!(
            cfg.global.sync_strategy.0[0],
            TransferStrategyArg::Reflink
        ));
        assert!(matches!(
            cfg.global.sync_strategy.0[1],
            TransferStrategyArg::Hardlink
        ));
    }

    #[test]
    fn parses_config_with_store_exif_true() {
        let toml = r#"
[global]

[import]
store_exif = true
path_template = "$year/$filename"
allowed_extensions = ["jpg"]
"#;

        let cfg: Config = toml::from_str(toml).expect("should parse store_exif");
        assert!(cfg.import.store_exif);
    }

    #[test]
    fn parses_config_with_custom_rename_template() {
        let toml = r#"
[global]

[import]
rename_template = "$filename-conflict-$n.$ext"
path_template = "$year/$filename"
allowed_extensions = ["jpg"]
"#;

        let cfg: Config = toml::from_str(toml).expect("should parse rename template");
        assert_eq!(cfg.import.rename_template, "$filename-conflict-$n.$ext");
    }

    // -------------------------------------------------------------------------
    // TOML deserialization tests - error handling
    // -------------------------------------------------------------------------

    #[test]
    fn rejects_unknown_strategy() {
        let toml = r#"
[global]
sync_strategy = ["reflink", "magic_copy"]

[import]
path_template = "$year/$filename"
allowed_extensions = ["jpg"]
"#;

        let result: Result<Config, _> = toml::from_str(toml);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("magic_copy") || err_msg.contains("unknown"));
    }

    #[test]
    fn rejects_unknown_strategy_in_string() {
        let toml = r#"
[global]
sync_strategy = "reflink,magic_copy"

[import]
path_template = "$year/$filename"
allowed_extensions = ["jpg"]
"#;

        let result: Result<Config, _> = toml::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_missing_required_import_section() {
        let toml = r#"
[global]
hash = "sha256"
"#;

        // Missing [import] section - should fail
        let result: Result<Config, _> = toml::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_invalid_toml_syntax() {
        let toml = r#"
[global
hash = "sha256"

[import]
path_template = "$year/$filename"
allowed_extensions = ["jpg"]
"#;
        // Missing closing bracket on [global]

        let result: Result<Config, _> = toml::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_malformed_strategy_type() {
        let toml = r#"
[global]
sync_strategy = 123

[import]
path_template = "$year/$filename"
allowed_extensions = ["jpg"]
"#;
        // Strategy should be string or list, not number

        let result: Result<Config, _> = toml::from_str(toml);
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // File I/O tests
    // -------------------------------------------------------------------------

    #[test]
    fn write_and_load_config_roundtrip() {
        let dir = tempfile::tempdir().unwrap();

        // Write default config
        Config::write_default(dir.path()).expect("should write config");

        // Verify file exists
        let config_path = dir.path().join(CONFIG_FILE);
        assert!(config_path.exists());

        // Load it back
        let loaded = Config::load(dir.path()).expect("should load config");

        // Verify values
        assert!(!loaded.import.store_exif);
    }

    #[test]
    fn load_returns_error_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();

        let result = Config::load(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn load_returns_error_for_invalid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(CONFIG_FILE);

        // Write invalid TOML
        std::fs::write(&config_path, "this is not valid toml {{").unwrap();

        let result = Config::load(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn preserves_custom_config_after_roundtrip() {
        let dir = tempfile::tempdir().unwrap();

        // Write a custom config manually
        let custom_toml = r#"
[global]
sync_strategy = ["hardlink", "copy"]

[import]
store_exif = true
rename_template = "custom-$n.$ext"
path_template = "$device/$filename"
allowed_extensions = ["cr2", "nef", "arw"]
"#;

        let config_path = dir.path().join(CONFIG_FILE);
        std::fs::write(&config_path, custom_toml).unwrap();

        // Load and verify
        let cfg = Config::load(dir.path()).expect("should load custom config");
        assert_eq!(cfg.global.sync_strategy.0.len(), 2);
        assert!(cfg.import.store_exif);
        assert_eq!(cfg.import.rename_template, "custom-$n.$ext");
        assert_eq!(cfg.import.path_template, "$device/$filename");
        assert_eq!(cfg.import.allowed_extensions, vec!["cr2", "nef", "arw"]);
    }

    // -------------------------------------------------------------------------
    // Helper function tests
    // -------------------------------------------------------------------------

    #[test]
    fn hash_algorithm_display_formats_correctly() {
        assert_eq!(format!("{}", HashAlgorithm::Xxh3_128), "fast");
        assert_eq!(format!("{}", HashAlgorithm::Sha256), "secure");
    }

    #[test]
    fn transfer_strategy_arg_converts_correctly() {
        use crate::fs::TransferStrategy;

        assert!(matches!(
            TransferStrategyArg::Reflink.to_transfer_strategy(),
            TransferStrategy::Reflink
        ));
        assert!(matches!(
            TransferStrategyArg::Hardlink.to_transfer_strategy(),
            TransferStrategy::Hardlink
        ));
        assert!(matches!(
            TransferStrategyArg::Copy.to_transfer_strategy(),
            TransferStrategy::StreamCopy
        ));
    }

    #[test]
    fn sync_strategy_converts_to_transfer_strategies() {
        let strategy = SyncStrategy(vec![
            TransferStrategyArg::Reflink,
            TransferStrategyArg::Copy,
        ]);

        let converted = strategy.to_transfer_strategies();
        assert_eq!(converted.len(), 2);
        assert!(matches!(converted[0], crate::fs::TransferStrategy::Reflink));
        assert!(matches!(
            converted[1],
            crate::fs::TransferStrategy::StreamCopy
        ));
    }

    #[test]
    fn transfer_strategy_arg_roundtrips_through_config_toml() {
        // Test serialization via full Config (TOML requires a table at root)
        let toml_str = r#"
[global]
sync_strategy = ["reflink", "hardlink", "copy"]

[import]
path_template = "$year/$filename"
allowed_extensions = ["jpg"]
"#;

        let cfg: Config = toml::from_str(toml_str).expect("should parse");
        assert_eq!(cfg.global.sync_strategy.0.len(), 3);
        assert!(matches!(
            cfg.global.sync_strategy.0[0],
            TransferStrategyArg::Reflink
        ));
        assert!(matches!(
            cfg.global.sync_strategy.0[1],
            TransferStrategyArg::Hardlink
        ));
        assert!(matches!(
            cfg.global.sync_strategy.0[2],
            TransferStrategyArg::Copy
        ));

        // Verify roundtrip
        let serialized = toml::to_string(&cfg).expect("should serialize");
        let deserialized: Config = toml::from_str(&serialized).expect("should deserialize");
        assert_eq!(deserialized.global.sync_strategy.0.len(), 3);
    }

    #[test]
    fn transfer_strategy_case_insensitive_in_config() {
        // Test case insensitivity via Config parsing
        let toml_lower = r#"
[global]
sync_strategy = "reflink"

[import]
path_template = "$year/$filename"
allowed_extensions = ["jpg"]
"#;

        let toml_upper = r#"
[global]
sync_strategy = "REFLINK,HARDLINK"

[import]
path_template = "$year/$filename"
allowed_extensions = ["jpg"]
"#;

        let toml_mixed = r#"
[global]
sync_strategy = ["Reflink", "Hardlink", "Copy"]

[import]
path_template = "$year/$filename"
allowed_extensions = ["jpg"]
"#;

        let cfg_lower: Config = toml::from_str(toml_lower).unwrap();
        let cfg_upper: Config = toml::from_str(toml_upper).unwrap();
        let cfg_mixed: Config = toml::from_str(toml_mixed).unwrap();

        assert!(matches!(
            cfg_lower.global.sync_strategy.0[0],
            TransferStrategyArg::Reflink
        ));
        assert!(matches!(
            cfg_upper.global.sync_strategy.0[0],
            TransferStrategyArg::Reflink
        ));
        assert!(matches!(
            cfg_upper.global.sync_strategy.0[1],
            TransferStrategyArg::Hardlink
        ));
        assert!(matches!(
            cfg_mixed.global.sync_strategy.0[0],
            TransferStrategyArg::Reflink
        ));
        assert!(matches!(
            cfg_mixed.global.sync_strategy.0[1],
            TransferStrategyArg::Hardlink
        ));
        assert!(matches!(
            cfg_mixed.global.sync_strategy.0[2],
            TransferStrategyArg::Copy
        ));
    }
}
