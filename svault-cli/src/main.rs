mod cli;
mod config;
mod db;
mod hash;
mod vfs;

use clap::Parser;
use cli::{Cli, Command, DbCommand};

fn main() {
    let cli = Cli::parse();

    let result: anyhow::Result<()> = match cli.command {
        Command::Init => {
            let root = std::env::current_dir().expect("cannot read cwd");
            db::init(&root)
        }
        Command::Import { .. } => todo!("import"),
        Command::Add { .. } => todo!("add"),
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
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
