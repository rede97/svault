use svault_core::config::{HashAlgorithm, RecheckMode, SyncStrategy};
use clap::{Parser, Subcommand, ValueEnum};

/// Svault — distributed multimedia archival tool.
/// Your memories, replicated forever.
#[derive(Parser)]
#[command(
    name = "svault",
    version,
    about = "Distributed multimedia archival tool",
    long_about = "Svault is an open-source, content-addressed multimedia archival tool.\n\
                  It backs up photos and videos across multiple drives, deduplicates\n\
                  files by content, and manages composite media like Live Photos and\n\
                  RAW+JPEG pairs — all from the command line.\n\n\
                  Svault never deletes your files. After import, review the manifest\n\
                  and delete source files yourself."
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

    /// Emit JSON progress events to stderr
    #[arg(long, global = true)]
    pub progress: bool,

    /// Path to config file
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<std::path::PathBuf>,

    /// Override vault root directory
    #[arg(long, global = true, value_name = "PATH")]
    pub vault: Option<std::path::PathBuf>,

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
    /// Initialize a new vault in the current directory
    ///
    /// Creates a `.svault/` directory and sets up the database.
    Init,

    /// Import media files from a source directory or device
    ///
    /// Scans SOURCE for supported media files, deduplicates them against the
    /// vault, copies new files into the vault, and writes a manifest.
    ///
    /// VAULT DISCOVERY
    /// svault locates the vault by walking up the directory tree from --target
    /// (or the current working directory if --target is omitted) until it finds
    /// a directory containing `.svault/vault.db`. This mirrors how `git` finds
    /// its `.git` directory. Run `svault init` in the desired root first.
    ///
    /// TARGET SUB-DIRECTORY
    /// --target is an optional sub-directory *inside* the vault that acts as
    /// the import destination prefix. It must be located under the vault root;
    /// pointing it outside the vault is an error. If omitted, the vault root
    /// itself is used as the destination prefix.
    ///
    /// PATH TEMPLATE RESOLUTION
    /// The final path of each imported file is:
    ///   <vault_root>/<target_rel>/<resolved_template>
    ///
    /// where <resolved_template> is expanded from the `import.path_template`
    /// in `svault.toml` (default: `$year/$mon-$day/$device`).
    ///
    /// Template placeholders:
    ///   $year   - 4-digit year  (EXIF DateTimeOriginal, else file mtime)
    ///   $mon    - 2-digit month
    ///   $day    - 2-digit day
    ///   $device - EXIF camera model (Make + Model), or "Unknown Device"
    ///
    /// CONFIG OVERRIDE IN SUB-DIRECTORIES
    /// If --target (or any ancestor directory between --target and the vault
    /// root) contains its own `svault.toml`, the `import.path_template` in
    /// that file overrides the vault-level config. The closest config file
    /// to --target wins, allowing different sub-vaults to use different
    /// organisational schemes without touching the root config.
    ///
    /// TRANSFER STRATEGY
    /// --strategy controls how files are copied into the vault:
    ///   auto     - pick the best available strategy automatically (default)
    ///   reflink  - copy-on-write clone (btrfs/xfs/APFS); zero data movement
    ///   hardlink - hard link (source and vault must be on the same filesystem)
    ///   copy     - streaming copy; always works
    /// When the source directory is on the same filesystem as the vault,
    /// hardlink is significantly faster and uses no extra disk space.
    ///
    /// SOURCE INSIDE VAULT
    /// If SOURCE is located inside the vault root, svault will refuse and
    /// suggest using `svault add` instead. Use `add` to register files that
    /// are already physically present inside the vault into the database
    /// without moving them.
    ///
    /// COMPARISON PIPELINE
    /// Stage 1 (always): EXIF metadata + CRC32C partial fingerprint (head/tail
    ///   64 KB). Files that do not collide are imported immediately.
    /// Stage 2 (collision fallback only): full-file hash computed lazily.
    ///   Both XXH3-128 and SHA-256 are stored as binary BLOBs in the database.
    ///   SHA-256 takes precedence when both are present; XXH3-128 serves as
    ///   a temporary identity until SHA-256 is computed.
    ///   -H / --hash selects which hash is computed in this run:
    ///     xxh3_128 - XXH3-128 (high throughput; good for slow networks
    ///                or bandwidth-constrained devices)
    ///     sha256   - SHA-256  (cryptographic strength)
    ///   Priority: CLI flag > svault.toml [import].dedup_hash > built-in default (xxh3_128)
    ///
    /// DUPLICATE HANDLING
    /// If a file's hash matches an existing vault entry it is skipped and
    /// recorded as "duplicate" in the manifest.
    ///
    /// XXH3-128 COLLISION (extremely rare)
    /// If two files share the same XXH3-128 but differ in content, svault
    /// reports a collision warning and refuses to import without
    /// -H sha256 / --hash sha256. If SHA-256 also matches the file is a true
    /// duplicate. If SHA-256 differs the file is imported normally.
    ///
    /// --ignore-duplicate forces import even when the file is confirmed as
    /// a duplicate (e.g. intentional re-import of an identical file).
    ///
    /// MANIFEST
    /// Every operation that modifies the database automatically writes a
    /// manifest file to `<vault_root>/manifest/`. The directory is created
    /// if it does not exist. Each manifest file is named with a UTC timestamp
    /// so that successive runs never overwrite each other:
    ///   manifest/import-20260330T143000Z.json
    /// The manifest records every file processed: its source path, resolved
    /// archive path, hashes, file size, and outcome (imported / duplicate /
    /// skipped / error / xxh3-collision).
    ///
    /// After import, review the manifest and delete source files yourself —
    /// svault never removes your originals.
    Import {
        /// Source directory or mount point to import from.
        /// Must not be located inside the vault root — use `svault add` for that.
        #[arg(value_name = "SOURCE")]
        source: std::path::PathBuf,

        /// Sub-directory inside the vault to use as the import destination
        /// prefix. Must be located under an initialized vault root. svault
        /// walks up from this path to discover the vault root. If this
        /// directory (or an ancestor up to the vault root) contains its own
        /// `svault.toml`, its `import.path_template` overrides the root
        /// config. Defaults to the current working directory.
        #[arg(long, value_name = "PATH")]
        target: Option<std::path::PathBuf>,

        /// Hash algorithm for full-file collision resolution (Stage 2).
        /// Priority: this flag > svault.toml [global].hash > default (xxh3_128).
        #[arg(short = 'H', long, value_enum)]
        hash: Option<HashAlgorithm>,

        /// File transfer strategy (see TRANSFER STRATEGY above).
        #[arg(long, default_value = "auto", value_enum)]
        strategy: SyncStrategy,

        /// Re-check duplicate files when all source files hit the CRC32C cache.
        /// fast - trust CRC32C cache, do not re-verify (default)
        /// exif - binary-compare EXIF header from archive vs source (64KB)
        /// hash - compute full-file hash and compare against database
        #[arg(short = 'R', long, default_value = "fast", value_enum, value_name = "MODE")]
        recheck: RecheckMode,

        /// Force import even when the file is confirmed as a duplicate.
        /// Use this to intentionally re-import an identical file.
        #[arg(long)]
        ignore_duplicate: bool,

        /// Print duplicate files during the scan.
        /// By default duplicates are counted but not listed.
        #[arg(long)]
        show_dup: bool,
    },

    
    /// Register files already inside the vault into the database
    ///
    /// Use this when files have been copied into the vault directory manually
    /// (outside of `svault import`). `add` computes their fingerprints and
    /// registers them in the database without moving them.
    ///
    /// PATH must be a directory located inside an initialized vault root.
    /// svault walks up from PATH to discover the vault root. Passing a path
    /// outside the vault is an error — use `svault import` instead.
    ///
    /// A manifest is written to `<vault_root>/manifest/` as with all
    /// database-modifying operations.
    Add {
        /// Directory inside the vault whose files should be registered.
        /// Must be located under the vault root.
        #[arg(value_name = "PATH")]
        path: std::path::PathBuf,

        /// Hash algorithm to use for add.
        #[arg(short = 'H', long, default_value = "sha256", value_enum)]
        hash: HashAlgorithm,
    },


    /// Sync files and metadata from another vault
    ///
    /// Pulls files and database records from SOURCE_VAULT into the local vault.
    /// SOURCE_VAULT must be the root directory of an initialized svault
    /// repository (i.e. it must contain `.svault/vault.db`).
    ///
    /// SYNC STRATEGY
    /// The two vaults' event logs are compared directly, so files that are
    /// already present in both databases are identified without re-hashing.
    /// Only files that have a database record in SOURCE_VAULT are considered;
    /// files that were copied into SOURCE_VAULT manually without `svault add`
    /// are invisible to sync.
    ///
    /// WHAT IS SYNCED
    /// - Files and their metadata (hash, path, mtime, media group, EXIF)
    ///   present in SOURCE_VAULT but missing from the local vault are copied
    ///   and registered.
    /// - Files already in both vaults are not touched.
    /// - No files are deleted from either vault.
    ///
    /// DIFFERENCE REPORT
    /// A diff report is written to `<vault_root>/manifest/` (UTC-timestamped)
    /// listing every file that was added, already present, or skipped.
    ///
    /// VERIFICATION
    /// --verify controls which files are checked after sync:
    ///   none - no post-sync verification
    ///   norm - verify only files that were added or updated in this sync
    ///          (default; catches transfer errors without a full scan)
    ///   full - verify every file in the local vault database; use this
    ///          for periodic integrity audits and forced data correction
    ///
    /// -H / --hash controls the hash algorithm used during verification:
    ///   xxh3_128 - XXH3-128 (high throughput, non-cryptographic)
    ///   sha256   - SHA-256  (cryptographic strength, default)
    Sync {
        /// Root directory of the source vault to sync from.
        /// Must contain `.svault/vault.db`.
        #[arg(value_name = "SOURCE_VAULT")]
        source: std::path::PathBuf,

        /// Transfer strategy
        #[arg(long, default_value = "auto", value_enum)]
        strategy: SyncStrategy,

        /// Scope of post-sync integrity verification.
        /// norm verifies only files touched in this sync;
        /// full verifies the entire local vault database.
        #[arg(long, default_value = "norm", value_enum)]
        verify: SyncVerifyScope,

        /// Hash algorithm used for verification and collision resolution.
        /// Overrides the default set in svault.toml (`global.hash`).
        #[arg(short = 'H', long, default_value = "sha256", value_enum)]
        hash: HashAlgorithm,
    },

    /// Locate files moved outside Svault and update database paths
    ///
    /// Scans the given root for files whose paths are no longer valid in the
    /// database. Matches by content fingerprint and records path updates as
    /// file.path_updated events. Runs in dry-run mode by default.
    Reconcile {
        /// Root directory to scan for relocated files
        #[arg(long, value_name = "PATH")]
        root: std::path::PathBuf,
    },

    /// Verify archive file integrity
    ///
    /// Checks every file in the vault against its stored hash. Reports
    /// corrupt or missing files. Suggest running `reconcile` for missing files.
    ///
    /// VERIFY LEVELS
    ///   fast   - verify with XXH3-128 (high throughput)
    ///   sha256 - verify with SHA-256 (cryptographic strength, default)
    ///
    /// USAGE EXAMPLES
    ///   svault verify                    # Verify all files
    ///   svault verify --recent 300       # Verify files imported in last 5 minutes
    ///   svault verify --file path/to.jpg # Verify single file
    Verify {
        /// Hash algorithm to use for verification.
        #[arg(short = 'H', long, default_value = "sha256", value_enum)]
        hash: HashAlgorithm,

        /// Verify only this file
        #[arg(long, value_name = "PATH")]
        file: Option<std::path::PathBuf>,

        /// Verify only files imported in the last N seconds
        #[arg(long, value_name = "SECONDS")]
        recent: Option<u64>,
    },

    /// Verify source files against import manifest
    ///
    /// Compares source files with the recorded state in the import manifest.
    /// This detects if source files were modified or deleted after import.
    ///
    /// USAGE EXAMPLES
    ///   svault verify-source                  # Verify latest import
    ///   svault verify-source --session <id>   # Verify specific session
    ///   svault verify-source --dir /path      # Verify specific source directory
    VerifySource {
        /// Session ID to verify (default: latest)
        #[arg(long, value_name = "SESSION_ID")]
        session: Option<String>,

        /// Verify only this source directory
        #[arg(long, value_name = "PATH")]
        dir: Option<std::path::PathBuf>,
    },

    /// Show vault status overview
    ///
    /// Displays counts of imported files, duplicates, pending SHA-256
    /// computation, media groups, events, and database size.
    Status,

    /// Query the event log
    ///
    /// Shows the operation history for a specific file or the entire vault.
    /// All changes are recorded as immutable events with a tamper-evident
    /// hash chain.
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

        /// Filter by event type (e.g. file.imported, file.path_updated)
        #[arg(long, value_name = "TYPE")]
        event_type: Option<String>,

        /// Maximum number of events to show
        #[arg(long, default_value = "50", value_name = "N")]
        limit: usize,
    },

    /// Compute SHA-256 for files imported without it (background task)
    ///
    /// SHA-256 is computed lazily — only when a fingerprint collision is
    /// detected during import. This command fills in sha256 = NULL records.
    /// Run it when the system is idle, or let it run automatically.
    BackgroundHash {
        /// Maximum number of files to process in this run
        #[arg(long, value_name = "N")]
        limit: Option<usize>,

        /// Run at low IO priority to minimise system impact
        #[arg(long)]
        nice: bool,
    },

    /// Clone a subset of the vault to a local working directory
    ///
    /// Useful for working offline (e.g. on a laptop). Filters let you select
    /// a date range, camera model, or media group type.
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

    /// MTP device browser
    ///
    /// Browse and explore MTP devices (Android phones, cameras) before importing.
    /// Use 'svault import' to actually import files from MTP URLs.
    ///
    /// MTP URLs use the format: mtp://device_name/path or mtp://SN:serial/path
    #[cfg(feature = "mtp")]
    Mtp {
        #[command(subcommand)]
        command: MtpCommand,
    },

    /// Database maintenance commands
    Db {
        #[command(subcommand)]
        command: DbCommand,
    },
}

#[derive(Subcommand)]
pub enum DbCommand {
    /// Verify the tamper-evident hash chain of the event log
    ///
    /// Walks every event in sequence and checks that each self_hash matches
    /// the computed value. Any break in the chain is reported with its seq.
    VerifyChain,

    /// Rebuild materialised view tables from the event log
    ///
    /// Use this to recover from a corrupted database. The event log is the
    /// source of truth; all other tables are derived from it.
    Replay {
        /// Replay only up to this event sequence number
        #[arg(long, value_name = "SEQ")]
        to_seq: Option<i64>,

        /// Replay only up to this point in time (RFC 3339 or YYYY-MM-DD)
        #[arg(long, value_name = "DATETIME")]
        to_time: Option<String>,
    },

    /// Dump database contents for debugging
    ///
    /// Exports table contents in a readable format. Useful for debugging
    /// without writing SQL queries manually.
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

/// MTP subcommands - browse devices and files only
#[derive(Subcommand)]
pub enum MtpCommand {
    /// List MTP devices or browse files
    ///
    /// Without PATH: lists connected MTP devices
    /// With PATH: lists files at the specified MTP path
    ///
    /// Examples:
    ///   svault mtp ls                          # List devices
    ///   svault mtp ls mtp://1/                 # List root
    ///   svault mtp ls mtp://1/DCIM -l          # List with details
    Ls {
        /// MTP path (e.g., mtp://1/DCIM). If omitted, lists devices.
        #[arg(value_name = "PATH")]
        path: Option<String>,

        /// Show file sizes and modification time
        #[arg(short, long)]
        long: bool,
    },

    /// Show directory tree of an MTP device
    ///
    /// Displays a tree view of the MTP device filesystem.
    /// Useful for exploring the device structure.
    ///
    /// Examples:
    ///   svault mtp tree mtp://1/
    ///   svault mtp tree mtp://1/DCIM --depth 3
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
