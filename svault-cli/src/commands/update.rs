use std::path::PathBuf;

use svault_core::context::VaultContext;
use svault_core::event::{Interactor, YesInteractor};
use svault_core::ops::update::{UpdateOptions, run_update};

use crate::cli::OutputFormat;
use crate::commands::SinkSet;

pub fn run(
    output: OutputFormat,
    quiet: bool,
    dry_run: bool,
    yes: bool,
    target: Option<PathBuf>,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let scan_root = target.unwrap_or_else(|| cwd.clone());
    let ctx = VaultContext::open(None, &scan_root)?;
    let opts = UpdateOptions {
        root: scan_root,
        vault_root: ctx.vault_root().to_path_buf(),
        dry_run,
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

    run_update(opts, ctx.db(), sink.as_sink(), interactor)?;
    Ok(())
}
