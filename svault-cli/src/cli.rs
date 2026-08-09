use clap::{Parser, Subcommand, ValueEnum};
use svault_core::config::TransferStrategyArg;

/// Svault — content-addressed multimedia archive.
#[derive(Parser)]
#[command(
    name = "svault",
    version,
    about = "Content-addressed multimedia archive"
)]
pub struct Cli {
    /// Output format
    #[arg(long, global = true, default_value = "human", value_enum)]
    pub output: OutputFormat,

    /// Preview changes without writing anything
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Skip interactive confirmation prompts
    #[arg(long, global = true)]
    pub yes: bool,

    /// Suppress non-error output
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Number of Rayon worker threads (0 = use Rayon default)
    #[arg(long, global = true, default_value = "0")]
    pub threads: usize,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Clone, ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
}

#[derive(Subcommand)]
pub enum Command {
    /// Initialize a new vault
    Init,

    /// Import media files from a source directory
    Import {
        /// Source directory or mount point to import from.
        /// Must not be located inside the vault root — use `svault add` for that.
        /// Use "-" to read file list from stdin (requires --files-from).
        #[arg(value_name = "SOURCE")]
        source: std::path::PathBuf,

        /// Read file list from a text file (one path per line) instead of scanning.
        /// When source is "-", reads from stdin.
        #[arg(long, value_name = "PATH")]
        files_from: Option<std::path::PathBuf>,

        /// Vault sub-directory to import into. Discovers the vault root by
        /// walking up from this path. Defaults to the current working directory.
        #[arg(long, value_name = "PATH")]
        target: Option<std::path::PathBuf>,

        /// File transfer strategy: reflink, hardlink, copy.
        /// Can be combined with commas (e.g. --strategy reflink,hardlink).
        /// Defaults to reflink; copy is always the final fallback.
        #[arg(long, value_delimiter = ',', default_value = "reflink")]
        strategy: Vec<TransferStrategyArg>,

        /// Force import even when the file is confirmed as a duplicate.
        /// Use this to intentionally re-import an identical file.
        /// Also computes SHA-256 for definitive identity verification.
        #[arg(long)]
        force: bool,

        /// Compute SHA-256 hash for definitive identity verification.
        /// Files with SHA-256 are identified by their cryptographic hash,
        /// providing stronger deduplication guarantees at the cost of speed.
        #[arg(long)]
        full_id: bool,

        /// Show duplicate files that were skipped during import.
        #[arg(long)]
        show_dup: bool,

        /// Maximum scan depth below the source directory:
        /// 0 = unlimited (default), 1 = only files directly inside it.
        /// Ignored when --files-from is used.
        #[arg(short = 'r', long, value_name = "N", default_value = "0")]
        max_depth: usize,

        /// Only import files matching this glob (source-relative,
        /// case-insensitive, repeatable, e.g. --include 'DCIM/**/*.JPG').
        #[arg(long, value_name = "GLOB")]
        include: Vec<String>,

        /// Skip files matching this glob (wins over --include, repeatable).
        #[arg(long, value_name = "GLOB")]
        exclude: Vec<String>,

        /// How thoroughly fingerprint-suspected duplicates are re-verified
        /// against the vault database: fast = trust the fingerprint;
        /// mid = full XXH3-128 comparison; high = SHA-256 when available.
        /// Re-run with mid/high to audit files you suspect were mis-detected.
        #[arg(short = 'c', long, value_enum, default_value = "fast")]
        compare_level: CompareLevelArg,
    },

    /// Register files already inside the vault
    Add {
        /// Directory inside the vault whose files should be registered.
        /// Must be located under the vault root.
        #[arg(value_name = "PATH")]
        path: std::path::PathBuf,
    },

    /// Update database paths for moved or renamed files
    ///
    /// Scans the vault and updates the database to reflect files that were
    /// moved or renamed outside of Svault. Missing files are automatically
    /// marked as missing in the database. Svault never deletes user files.
    Update {
        /// Sub-directory inside the vault to scan for relocated files.
        /// Defaults to the current working directory (same discovery rules as import).
        #[arg(long, value_name = "PATH")]
        target: Option<std::path::PathBuf>,
    },

    /// Verify archive integrity
    Verify {
        /// Verify only this file
        #[arg(long, value_name = "PATH")]
        file: Option<std::path::PathBuf>,

        /// Verify only files imported in the last N seconds
        #[arg(long, value_name = "SECONDS")]
        recent: Option<u64>,

        /// Upgrade hardlinked files to independent binary copies during verification
        #[arg(long)]
        upgrade_links: bool,

        /// Compute missing SHA-256 hashes before verifying
        #[arg(long)]
        background_hash: bool,

        /// Maximum number of files to process when --background-hash is used
        #[arg(long, value_name = "N")]
        background_hash_limit: Option<usize>,
    },

    /// Show vault statistics
    Status,

    /// Export a subset of the vault to a working directory
    ///
    /// Copies files (optionally filtered) out of the vault into a plain
    /// directory, preserving the vault's relative paths. The target does
    /// not become a vault; a manifest JSON is written alongside the files.
    Clone {
        /// Destination directory for the exported files
        #[arg(long, value_name = "PATH")]
        target: std::path::PathBuf,

        /// Only export files modified in this date range (e.g. 2024-03-01..2024-03-31)
        #[arg(long, value_name = "RANGE")]
        filter_date: Option<String>,

        /// File transfer strategy: reflink, hardlink, copy.
        /// Can be combined with commas (e.g. --strategy reflink,hardlink).
        #[arg(long, value_delimiter = ',', default_value = "reflink")]
        strategy: Vec<TransferStrategyArg>,
    },

    /// Copy files from another vault that this vault is missing
    ///
    /// Compares both vaults' database records (hash-accelerated — no full
    /// re-hashing), shows a diff plan, and copies missing files in.
    /// The source vault is opened read-only and never modified.
    /// Files only in this vault are kept (Svault never deletes files).
    Sync {
        /// Root directory of the source vault to sync from.
        /// Must contain `.svault/vault.db`.
        #[arg(value_name = "SOURCE_VAULT")]
        source: std::path::PathBuf,

        /// Transfer strategy: reflink, hardlink, copy.
        /// Can be combined with commas (e.g. --strategy reflink,hardlink).
        #[arg(long, value_delimiter = ',', default_value = "reflink")]
        strategy: Vec<TransferStrategyArg>,

        /// Scope of post-sync integrity verification.
        /// norm verifies only files added in this sync;
        /// full verifies the entire local vault database.
        #[arg(long, default_value = "norm")]
        verify: svault_core::ops::sync::SyncVerifyScope,
    },

    /// Manage albums — named, optionally nested collections of vault files
    /// (e.g. "trips/norway/tromso") with per-membership ratings
    Album {
        #[command(subcommand)]
        command: AlbumCommand,
    },

    /// Database maintenance
    Db {
        #[command(subcommand)]
        command: DbCommand,
    },

    /// Scan directory and output file status for import pipeline (debug builds only)
    ///
    /// Output format: SCAN:<source_path> [status:filename ...]
    /// Status: new=will import, dup=duplicate, fail=error
    #[cfg(debug_assertions)]
    Scan {
        /// Source directory to scan
        #[arg(value_name = "SOURCE")]
        source: std::path::PathBuf,

        /// Show duplicate files during scanning
        #[arg(long)]
        show_dup: bool,
    },

    /// Debug utilities (debug builds only)
    #[cfg(debug_assertions)]
    Debug {
        #[command(subcommand)]
        command: DebugCommand,
    },
}

#[cfg(debug_assertions)]
#[derive(Subcommand)]
pub enum DebugCommand {
    /// Test reporter output with simulated import
    Reporter {
        /// Number of files to simulate
        #[arg(short, long, default_value = "10")]
        count: usize,

        /// Delay between events (milliseconds)
        #[arg(short, long, default_value = "100")]
        delay_ms: u64,

        /// Show duplicate file simulation
        #[arg(long)]
        show_dup: bool,
    },
}

/// Duplicate-verification level for `import --compare-level`
/// (aliases: 0 = fast, 1 = mid, 2 = high).
#[derive(Clone, Copy, ValueEnum)]
pub enum CompareLevelArg {
    /// Trust the size+CRC fingerprint (default).
    #[value(alias = "0")]
    Fast,
    /// Re-verify fingerprint hits with a full XXH3-128 source hash.
    #[value(alias = "1")]
    Mid,
    /// Re-verify with SHA-256 when the DB record has one, else XXH3-128.
    #[value(alias = "2")]
    High,
}

impl From<CompareLevelArg> for svault_core::ops::types::CompareLevel {
    fn from(arg: CompareLevelArg) -> Self {
        match arg {
            CompareLevelArg::Fast => Self::Fast,
            CompareLevelArg::Mid => Self::Mid,
            CompareLevelArg::High => Self::High,
        }
    }
}

#[derive(Subcommand)]
pub enum AlbumCommand {
    /// Create an album; parent levels are auto-created (like mkdir -p)
    Create {
        /// Album path, e.g. "trips/norway/tromso"
        #[arg(value_name = "PATH")]
        path: String,
    },
    /// List all albums as a tree with member counts.
    /// An optional glob filters by full album path (e.g. "trips/*"),
    /// keeping the ancestor chain of matches.
    List {
        /// Glob pattern over album paths
        #[arg(value_name = "GLOB")]
        pattern: Option<String>,
    },
    /// Show album members with per-membership ratings.
    /// Accepts an exact album path or a glob matching several albums.
    Show {
        /// Album path
        #[arg(value_name = "PATH")]
        path: String,
    },
    /// Add vault files to an album (membership only; files are not copied)
    Add {
        /// Album path
        #[arg(value_name = "ALBUM")]
        album: String,
        /// Vault-relative file paths (or absolute paths inside the vault)
        #[arg(value_name = "PATH", required = true)]
        paths: Vec<String>,
    },
    /// Remove files from an album (never deletes the files themselves)
    Remove {
        /// Album path
        #[arg(value_name = "ALBUM")]
        album: String,
        /// Vault-relative file paths
        #[arg(value_name = "PATH", required = true)]
        paths: Vec<String>,
    },
    /// Rate album members (1-5 stars, 0 clears). Ratings are per membership:
    /// the same photo may be rated differently in different albums.
    Rate {
        /// Album path
        #[arg(value_name = "ALBUM")]
        album: String,
        /// Rating: 1-5, or 0 to clear
        #[arg(value_name = "RATING")]
        rating: u8,
        /// Vault-relative file paths (must already be members)
        #[arg(value_name = "PATH", required = true)]
        paths: Vec<String>,
    },
    /// Delete an album (only when it has no members and no child albums)
    Delete {
        /// Album path
        #[arg(value_name = "PATH")]
        path: String,
    },
}

#[derive(Subcommand)]
pub enum DbCommand {
    /// Dump database contents
    Dump {
        /// Tables to dump (default: all)
        #[arg(value_name = "TABLE")]
        tables: Vec<String>,

        /// Output format
        #[arg(short, long, default_value = "csv", value_enum)]
        format: DumpFormat,

        /// Limit number of rows per table
        #[arg(short, long, value_name = "N")]
        limit: Option<usize>,
    },
}

#[derive(Clone, ValueEnum)]
pub enum DumpFormat {
    /// CSV format (default)
    Csv,
    /// JSON format
    Json,
    /// SQL INSERT statements
    Sql,
}
