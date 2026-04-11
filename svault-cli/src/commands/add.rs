use std::path::PathBuf;
use std::sync::Arc;

use crate::reporting::TerminalReporterBuilder;
use svault_core::context::VaultContext;
use svault_core::import::add::{AddOptions, run_add};

pub fn run(path: PathBuf) -> anyhow::Result<()> {
    let ctx = VaultContext::open(None, &path)?;
    let opts = AddOptions {
        path,
        vault_root: ctx.vault_root().to_path_buf(),
        full_id: false, // Default to fast mode for add
    };
    let reporter_builder = Arc::new(TerminalReporterBuilder::new());
    run_add(opts, ctx.db(), reporter_builder.as_ref())?;
    Ok(())
}
