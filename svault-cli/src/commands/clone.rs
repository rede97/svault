use std::path::PathBuf;

use svault_core::config::{SyncStrategy, TransferStrategyArg};
use svault_core::context::VaultContext;
use svault_core::ops::clone::{CloneOptions, parse_date_range, run_clone};

use crate::cli::OutputFormat;
use crate::commands::SinkSet;

pub fn run(
    output: OutputFormat,
    quiet: bool,
    target: PathBuf,
    filter_date: Option<String>,
    strategy: Vec<TransferStrategyArg>,
) -> anyhow::Result<()> {
    let ctx = VaultContext::open_cwd()?;

    let filter = match filter_date {
        Some(range) => Some(parse_date_range(&range)?),
        None => None,
    };

    let opts = CloneOptions {
        vault_root: ctx.vault_root().to_path_buf(),
        target,
        filter_date: filter,
        strategy: SyncStrategy(strategy),
    };

    let sink = SinkSet::new(&output, quiet, false);
    run_clone(opts, ctx.db(), sink.as_sink())?;
    Ok(())
}
