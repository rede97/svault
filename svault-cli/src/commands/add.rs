use std::path::PathBuf;

use svault_core::context::VaultContext;
use svault_core::ops::add::{AddOptions, run_add};

use crate::cli::OutputFormat;
use crate::commands::SinkSet;

pub fn run(output: OutputFormat, quiet: bool, path: PathBuf) -> anyhow::Result<()> {
    let ctx = VaultContext::open(None, &path)?;
    let opts = AddOptions {
        path,
        vault_root: ctx.vault_root().to_path_buf(),
        full_id: false, // Default to fast mode for add
    };
    let sink = SinkSet::new(&output, quiet, false);
    run_add(opts, ctx.db(), sink.as_sink())?;
    Ok(())
}
