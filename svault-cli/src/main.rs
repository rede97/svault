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

use clap::Parser;
use cli::{Cli, Command, DbCommand, MtpCommand};
use commands::setup_signal_handler;

fn run(cli: Cli) -> anyhow::Result<()> {
    // Extract global flags before matching on command
    let output = cli.output;
    let dry_run = cli.dry_run;
    let yes = cli.yes;
    
    match cli.command {
        Command::Init => commands::init::run(),
        Command::Scan { source, show_dup } => {
            commands::scan::run(output, source, show_dup)
        }
        Command::Import {
            source,
            files_from,
            target,
            strategy,
            force,
            show_dup,
        } => commands::import::run(output, dry_run, yes, source, files_from, target, strategy, force, show_dup),
        Command::Recheck {
            source,
            target,
            session,
        } => commands::recheck::run(source, target, session),
        Command::Add { path } => commands::add::run(path),
        Command::Sync { .. } => commands::sync::run(),
        Command::Update { target, clean, delete } => {
            commands::update::run(dry_run, yes, target, clean, delete)
        }
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
        Command::History {
            file,
            from,
            to,
            events,
            limit,
            verbose,
        } => commands::history::run(output, file, from, to, events, limit, verbose),
        Command::Clone { .. } => commands::clone::run(),
        #[cfg(feature = "mtp")]
        Command::Mtp { command } => match command {
            MtpCommand::Ls { path, long } => commands::mtp::run_ls(path, long),
            MtpCommand::Tree { path, depth } => commands::mtp::run_tree(path, depth),
        },
        #[cfg(not(feature = "mtp"))]
        Command::Mtp { .. } => {
            Err(anyhow::anyhow!("MTP support not enabled. Build with --features mtp"))
        }
        Command::Db { command } => match command {
            DbCommand::VerifyChain => commands::db::run_verify_chain(),
            DbCommand::Replay { .. } => commands::db::run_replay(),
            DbCommand::Dump { tables, format, limit } => {
                commands::db::run_dump(tables, format, limit)
            }
        },
    }
}

fn main() {
    // Initialize logger (RUST_LOG env var controls level)
    env_logger::init();

    // Setup signal handler for graceful shutdown on Ctrl-C
    setup_signal_handler();

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
