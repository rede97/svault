use std::path::PathBuf;

use svault_core::context::VaultContext;
use svault_core::import::add::{run_add, AddOptions};

pub fn run(path: PathBuf) -> anyhow::Result<()> {
    let ctx = VaultContext::open(None, &path)?;
    let opts = AddOptions {
        path,
        vault_root: ctx.vault_root().to_path_buf(),
        full_id: false, // Default to fast mode for add
    };
    run_add(opts, ctx.db())?;
    Ok(())
}
