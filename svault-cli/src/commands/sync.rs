use std::path::PathBuf;

use svault_core::config::{SyncStrategy, TransferStrategyArg};
use svault_core::context::VaultContext;
use svault_core::event::{Interactor, YesInteractor};
use svault_core::ops::sync::{SyncOptions, SyncVerifyScope, run_sync};

use crate::cli::OutputFormat;
use crate::commands::SinkSet;

pub fn run(
    output: OutputFormat,
    quiet: bool,
    yes: bool,
    source: PathBuf,
    strategy: Vec<TransferStrategyArg>,
    verify: SyncVerifyScope,
) -> anyhow::Result<()> {
    // Destination = the vault we are standing in (takes the write lock).
    let ctx = VaultContext::open_cwd()?;

    let opts = SyncOptions {
        source_vault: source,
        dest_vault_root: ctx.vault_root().to_path_buf(),
        strategy: SyncStrategy(strategy),
        verify,
        yes,
    };

    let sink = SinkSet::new(&output, quiet, false);
    let yes_i = YesInteractor;
    let term_i;
    let interactor: &dyn Interactor = if yes {
        &yes_i
    } else {
        match &sink {
            SinkSet::Terminal(s) => {
                term_i = s.interactor();
                &term_i
            }
            _ => &yes_i,
        }
    };

    run_sync(opts, ctx.db(), sink.as_sink(), interactor)?;
    Ok(())
}
