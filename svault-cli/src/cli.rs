use svault_core::config::{HashAlgorithm, TransferStrategyArg};
use clap::{Parser, Subcommand, ValueEnum};

/// Svault — content-addressed multimedia archive.
#[derive(Parser)]
#[command(name = "svault", version, about = "Content-addressed multimedia archive")]
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

    /// Emit JSON progress events to stderr
    #[arg(long, global = true)]
    pub progress: bool,

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
        #[arg(value_name = "SOURCE")]
        source: std::path::PathBuf,

        /// Vault sub-directory to import into. Discovers the vault root by
        /// walking up from this path. Defaults to the current working directory.
        #[arg(long, value_name = "PATH")]
        target: Option<std::path::PathBuf>,

        /// Hash algorithm for collision resolution.
        /// Priority: this flag > svault.toml config > default (xxh3_128).
        #[arg(short = 'H', long, value_enum)]
        hash: Option<HashAlgorithm>,

        /// File transfer strategy: reflink, hardlink, copy.
        /// Can be combined with commas (e.g. --strategy reflink,hardlink).
        /// Defaults to reflink; copy is always the final fallback.
        #[arg(long, value_delimiter = ',', value_enum, default_value = "reflink")]
        strategy: Vec<TransferStrategyArg>,

        /// Force import even when the file is confirmed as a duplicate.
        /// Use this to intentionally re-import an identical file.
        #[arg(long)]
        force: bool,

        /// Print duplicate files during the scan.
        /// By default duplicates are counted but not listed.
        #[arg(long)]
        show_dup: bool,
    },

    /// Re-check a previous import against its manifest.
    ///
    /// Reads an import manifest and verifies both the original source files
    /// and the vault copies against the hashes recorded at import time.
    /// A report is written to `.svault/staging/` so you can decide which
    /// side is correct. No files are imported or deleted.
    Recheck {
        /// Optional source directory to verify against the manifest.
        /// Must match the source_root recorded in the manifest.
        #[arg(value_name = "SOURCE")]
        source: Option<std::path::PathBuf>,

        /// Sub-directory inside the vault (same discovery rules as import).
        #[arg(long, value_name = "PATH")]
        target: Option<std::path::PathBuf>,

        /// Session ID to recheck (default: latest import).
        #[arg(long, value_name = "SESSION_ID")]
        session: Option<String>,

        /// Hash algorithm for the comparison.
        #[arg(short = 'H', long, value_enum)]
        hash: Option<HashAlgorithm>,
    },

    
    /// Register files already inside the vault
    Add {
        /// Directory inside the vault whose files should be registered.
        /// Must be located under the vault root.
        #[arg(value_name = "PATH")]
        path: std::path::PathBuf,

        /// Hash algorithm to use for add.
        /// Defaults to `global.hash` in svault.toml (fast if unset).
        #[arg(short = 'H', long, value_enum)]
        hash: Option<HashAlgorithm>,
    },


    /// Sync files from another vault
    Sync {
        /// Root directory of the source vault to sync from.
        /// Must contain `.svault/vault.db`.
        #[arg(value_name = "SOURCE_VAULT")]
        source: std::path::PathBuf,

        /// Transfer strategy: reflink, hardlink, copy.
        /// Can be combined with commas (e.g. --strategy reflink,hardlink).
        /// Defaults to reflink; copy is always the final fallback.
        #[arg(long, value_delimiter = ',', value_enum, default_value = "reflink")]
        strategy: Vec<TransferStrategyArg>,

        /// Scope of post-sync integrity verification.
        /// norm verifies only files touched in this sync;
        /// full verifies the entire local vault database.
        #[arg(long, default_value = "norm", value_enum)]
        verify: SyncVerifyScope,

        /// Hash algorithm used for verification and collision resolution.
        /// Overrides the default set in svault.toml (`global.hash`).
        #[arg(short = 'H', long, value_enum)]
        hash: Option<HashAlgorithm>,
    },

    /// Update database paths for moved files
    Reconcile {
        /// Sub-directory inside the vault to scan for relocated files.
        /// Defaults to the current working directory (same discovery rules as import).
        #[arg(long, value_name = "PATH")]
        target: Option<std::path::PathBuf>,

        /// Clean up files that cannot be reconciled (mark as missing in database).
        /// Files that are not found on disk and cannot be matched will be marked.
        #[arg(long, group = "clean_mode")]
        clean: bool,

        /// Actually delete files from vault (requires --clean).
        /// WARNING: This permanently removes files from the vault!
        #[arg(long, requires = "clean_mode")]
        delete: bool,
    },

    /// Verify archive integrity
    Verify {
        /// Hash algorithm to use for verification.
        /// Defaults to `global.hash` in svault.toml (fast if unset).
        #[arg(short = 'H', long, value_enum)]
        hash: Option<HashAlgorithm>,

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

        /// Run at low IO priority when --background-hash is used
        #[arg(long)]
        background_hash_nice: bool,
    },

    /// Show vault statistics
    Status,

    /// Query the event log
    ///
    /// Default shows import/add/reconcile sessions. Use --verbose to show files.
    /// Use --events for low-level event stream (import, add, reconcile, file.imported, etc.)
    History {
        /// Filter to events for this file
        #[arg(long, value_name = "PATH")]
        file: Option<std::path::PathBuf>,

        /// Show events from this time (RFC 3339 or YYYY-MM-DD)
        #[arg(long, value_name = "DATETIME")]
        from: Option<String>,

        /// Show events up to this time (RFC 3339 or YYYY-MM-DD)
        #[arg(long, value_name = "DATETIME")]
        to: Option<String>,

        /// Show low-level event stream instead of session view
        #[arg(long, group = "display_mode")]
        events: bool,

        /// Filter by event type (e.g. file.imported, file.path_updated)
        #[arg(long, value_name = "TYPE", requires = "events")]
        event_type: Option<String>,

        /// Maximum number of sessions/events to show
        #[arg(long, default_value = "50", value_name = "N")]
        limit: usize,

        /// Show detailed file list for each session
        #[arg(short, long)]
        verbose: bool,
    },

    /// Clone a subset to a working directory
    Clone {
        /// Destination directory for the cloned subset
        #[arg(long, value_name = "PATH")]
        target: std::path::PathBuf,

        /// Filter by date range (e.g. 2024-03-01..2024-03-31)
        #[arg(long, value_name = "RANGE")]
        filter_date: Option<String>,

        /// Filter by camera model
        #[arg(long, value_name = "MODEL")]
        filter_camera: Option<String>,

        /// Filter by media group type (live_photo, raw_jpeg, single)
        #[arg(long, value_name = "TYPE")]
        filter_group: Option<String>,
    },

    /// Browse MTP devices
    #[cfg(feature = "mtp")]
    Mtp {
        #[command(subcommand)]
        command: MtpCommand,
    },

    /// Database maintenance
    Db {
        #[command(subcommand)]
        command: DbCommand,
    },
}

#[derive(Subcommand)]
pub enum DbCommand {
    /// Verify the event-log hash chain
    VerifyChain,

    /// Rebuild views from the event log
    Replay {
        /// Replay only up to this event sequence number
        #[arg(long, value_name = "SEQ")]
        to_seq: Option<i64>,

        /// Replay only up to this point in time (RFC 3339 or YYYY-MM-DD)
        #[arg(long, value_name = "DATETIME")]
        to_time: Option<String>,
    },

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

#[derive(Subcommand)]
pub enum MtpCommand {
    /// List MTP devices or browse files
    Ls {
        /// MTP path (e.g., mtp://1/DCIM). If omitted, lists devices.
        #[arg(value_name = "PATH")]
        path: Option<String>,

        /// Show file sizes and modification time
        #[arg(short, long)]
        long: bool,
    },

    /// Show MTP device tree
    Tree {
        /// MTP path (e.g., mtp://1/)
        #[arg(value_name = "PATH")]
        path: String,

        /// Maximum depth to display
        #[arg(short, long, default_value = "3")]
        depth: usize,
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

#[derive(Clone, ValueEnum)]
pub enum SyncVerifyScope {
    /// No post-sync verification
    None,
    /// Verify only files added or updated in this sync (default)
    Norm,
    /// Verify every file in the local vault database
    Full,
}
