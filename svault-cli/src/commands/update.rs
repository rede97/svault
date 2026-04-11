use std::path::PathBuf;
use std::sync::Arc;

use crate::reporting::{SuspendingInteractor, TerminalReporterBuilder};
use svault_core::context::VaultContext;
use svault_core::import::update::{UpdateOptions, run_update};
use svault_core::reporting::YesInteractor;

pub fn run(dry_run: bool, yes: bool, target: Option<PathBuf>, delete: bool) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let scan_root = target.unwrap_or_else(|| cwd.clone());
    let ctx = VaultContext::open(None, &scan_root)?;
    let opts = UpdateOptions {
        root: scan_root,
        vault_root: ctx.vault_root().to_path_buf(),
        dry_run,
        yes,
        delete,
    };
    let reporter_builder = Arc::new(TerminalReporterBuilder::new());
    if yes {
        run_update(opts, ctx.db(), reporter_builder.as_ref(), &YesInteractor)?;
    } else {
        let interactor = SuspendingInteractor::new(reporter_builder.multi_progress.clone());
        run_update(opts, ctx.db(), reporter_builder.as_ref(), &interactor)?;
    }
    Ok(())
}
