//! # svault-cli
//!
//! Command-line interface for **Svault** — a content-addressed multimedia archive.
//!
//! This crate is the thin application layer (L2): it parses arguments with
//! clap, builds sinks/interactors from `svault-ui`, and calls into
//! `svault-core`. See `docs/ARCHITECTURE.md`.

pub mod cli;
pub mod commands;

use clap::Parser;
#[cfg(debug_assertions)]
use cli::DebugCommand;
use cli::{Cli, Command, DbCommand};

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
    let quiet = cli.quiet;
    let dry_run = cli.dry_run;
    let yes = cli.yes;

    match cli.command {
        Command::Init => commands::init::run(),
        Command::Import {
            source,
            files_from,
            target,
            strategy,
            force,
            full_id,
            show_dup,
        } => commands::import::run(
            output, quiet, dry_run, yes, source, files_from, target, strategy, force, full_id,
            show_dup,
        ),
        Command::Recheck {
            source,
            target,
            session,
        } => commands::recheck::run(output, quiet, source, target, session),
        Command::Add { path } => commands::add::run(output, quiet, path),
        Command::Update { target } => commands::update::run(output, quiet, dry_run, yes, target),
        Command::Verify {
            file,
            recent,
            upgrade_links,
            background_hash,
            background_hash_limit,
        } => commands::verify::run(
            output,
            quiet,
            file,
            recent,
            upgrade_links,
            background_hash,
            background_hash_limit,
        ),
        Command::Status => commands::status::run(output),
        Command::Clone {
            target,
            filter_date,
            strategy,
        } => commands::clone::run(output, quiet, target, filter_date, strategy),
        Command::Sync {
            source,
            strategy,
            verify,
        } => commands::sync::run(output, quiet, yes, source, strategy, verify),
        Command::Db { command } => match command {
            DbCommand::VerifyChain => commands::db::run_verify_chain(),
            DbCommand::Dump {
                tables,
                format,
                limit,
            } => commands::db::run_dump(tables, format, limit),
        },
        #[cfg(debug_assertions)]
        Command::Scan { source, show_dup } => commands::scan::run(source, show_dup),
        #[cfg(debug_assertions)]
        Command::Debug { command } => match command {
            DebugCommand::Reporter {
                count,
                delay_ms,
                show_dup,
            } => commands::debug_reporter::run(count, delay_ms, show_dup),
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
