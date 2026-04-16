//! # svault-cli
//!
//! Command-line interface for **Svault** — a content-addressed multimedia archive.
//!
//! ## Quick start
//!
//! ```bash
//! # Initialize a vault
//! svault init
//!
//! # Import photos from a directory or device
//! svault import /path/to/photos
//!
//! # Check vault health
//! svault status
//! svault verify
//! ```

pub mod cli;
pub mod commands;
pub mod reporting;

use clap::Parser;
use cli::{Cli, Command, DbCommand};
#[cfg(debug_assertions)]
use cli::DebugCommand;

fn run(cli: Cli) -> anyhow::Result<()> {
    // Configure Rayon thread pool if specified
    if cli.threads > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(cli.threads)
            .build_global()
            .map_err(|e| anyhow::anyhow!("Failed to initialize Rayon thread pool: {}", e))?;
    }

    // Extract global flags before matching on command
    let output = cli.output;
    let dry_run = cli.dry_run;
    let yes = cli.yes;

    // Note: JSON output support is limited; individual commands handle their own output formatting

    match cli.command {
        Command::Init => commands::init::run(),
        Command::Scan { source, show_dup } => commands::scan::run(output, source, show_dup),
        Command::Import {
            source,
            files_from,
            target,
            strategy,
            force,
            full_id,
            show_dup,
        } => commands::import::run(
            output, dry_run, yes, source, files_from, target, strategy, force, full_id, show_dup,
        ),
        Command::Recheck {
            source,
            target,
            session,
        } => commands::recheck::run(source, target, session),
        Command::Add { path } => commands::add::run(path),
        Command::Sync { .. } => commands::sync::run(),
        Command::Update { target, delete } => commands::update::run(dry_run, yes, target, delete),
        Command::Verify {
            file,
            recent,
            upgrade_links,
            background_hash,
            background_hash_limit,
        } => commands::verify::run(
            output,
            file,
            recent,
            upgrade_links,
            background_hash,
            background_hash_limit,
        ),
        Command::Status => commands::status::run(output),
        Command::History { subcommand } => commands::history::run(output, subcommand),
        Command::Clone {
            target,
            filter_date,
            filter_camera,
        } => commands::clone::run(output, target, filter_date, filter_camera),
        Command::Db { command } => match command {
            DbCommand::VerifyChain => commands::db::run_verify_chain(),
            DbCommand::Dump {
                tables,
                format,
                limit,
            } => commands::db::run_dump(tables, format, limit),
        },
        #[cfg(debug_assertions)]
        Command::Debug { command } => match command {
            DebugCommand::Reporter {
                count,
                delay_ms,
                show_dup,
            } => commands::debug_reporter::run(output, count, delay_ms, show_dup),
        },
    }
}

fn main() {
    // Initialize logger (RUST_LOG env var controls level)
    env_logger::init();

    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        let msg = e.to_string();
        // Improve common error messages for better UX
        let friendly_msg = if msg.contains("database or disk is full") {
            "No space left on device (vault disk full)".to_string()
        } else if msg.contains("disk I/O error") {
            "Disk I/O error (possible hardware issue or disk full)".to_string()
        } else {
            msg
        };
        eprintln!("error: {}", friendly_msg);
        std::process::exit(1);
    }
}
