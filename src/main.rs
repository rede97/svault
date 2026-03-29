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
struct Cli {
    /// Output format
    #[arg(long, global = true, default_value = "human", value_enum)]
    output: OutputFormat,

    /// Preview changes without writing anything
    #[arg(long, global = true)]
    dry_run: bool,

    /// Skip interactive confirmation prompts
    #[arg(long, global = true)]
    yes: bool,

    /// Suppress non-error output
    #[arg(long, global = true)]
    quiet: bool,

    /// Emit JSON progress events to stderr
    #[arg(long, global = true)]
    progress: bool,

    /// Path to config file
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<std::path::PathBuf>,

    /// Override vault root directory
    #[arg(long, global = true, value_name = "PATH")]
    vault: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, ValueEnum)]
enum OutputFormat {
    Human,
    Json,
}

#[derive(Subcommand)]
enum Command {
    /// Import media files from a source directory or device
    ///
    /// Scans the source, computes fingerprints, deduplicates against the vault,
    /// copies new files, and writes a manifest. The manifest maps each imported
    /// file to its archive path. Review it and delete source files manually.
    Import {
        /// Source directory or mount point to import from
        #[arg(long, value_name = "PATH")]
        source: std::path::PathBuf,

        /// Target vault directory (defaults to vault in config)
        #[arg(long, value_name = "PATH")]
        target: Option<std::path::PathBuf>,

        /// Comparison strategy
        #[arg(long, default_value = "sha256", value_enum)]
        compare_level: CompareLevel,

        /// Write manifest to this path (default: auto-generated filename)
        #[arg(long, value_name = "PATH")]
        manifest: Option<std::path::PathBuf>,
    },

    /// Incrementally sync the vault to a backup target
    ///
    /// Only copies files not already present on the target (by SHA-256 or
    /// fingerprint). Uses the most efficient transfer strategy available
    /// (reflink, hardlink, or stream copy).
    Sync {
        /// Backup target path (local mount point)
        #[arg(long, value_name = "PATH")]
        target: std::path::PathBuf,

        /// Transfer strategy
        #[arg(long, default_value = "auto", value_enum)]
        strategy: SyncStrategy,

        /// Verify target file integrity after sync
        #[arg(long)]
        verify: bool,
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
    Verify {
        /// Fast mode: use XXH3-128 instead of SHA-256
        #[arg(long, conflicts_with = "full")]
        fast: bool,

        /// Full mode: verify with SHA-256 (default)
        #[arg(long, conflicts_with = "fast")]
        full: bool,

        /// Verify only this file
        #[arg(long, value_name = "PATH")]
        file: Option<std::path::PathBuf>,
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

    /// Database maintenance commands
    Db {
        #[command(subcommand)]
        command: DbCommand,
    },
}

#[derive(Subcommand)]
enum DbCommand {
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
}

#[derive(Clone, ValueEnum)]
enum CompareLevel {
    /// Fingerprint only (CRC32C + EXIF, no full-file read)
    Fast,
    /// Full SHA-256 (default)
    Sha256,
}

#[derive(Clone, ValueEnum)]
enum SyncStrategy {
    /// Pick the best available strategy automatically
    Auto,
    /// Use reflink (copy-on-write, btrfs/xfs only)
    Reflink,
    /// Use hardlink (same filesystem only)
    Hardlink,
    /// Stream copy (always works)
    Copy,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Import { .. } => todo!("import"),
        Command::Sync { .. } => todo!("sync"),
        Command::Reconcile { .. } => todo!("reconcile"),
        Command::Verify { .. } => todo!("verify"),
        Command::Status => todo!("status"),
        Command::History { .. } => todo!("history"),
        Command::BackgroundHash { .. } => todo!("background-hash"),
        Command::Clone { .. } => todo!("clone"),
        Command::Db { command } => match command {
            DbCommand::VerifyChain => todo!("db verify-chain"),
            DbCommand::Replay { .. } => todo!("db replay"),
        },
    }
}
