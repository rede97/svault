use std::path::PathBuf;

use svault_core::context::VaultContext;
use svault_core::import::update::{run_update, UpdateOptions};

pub fn run(
    dry_run: bool,
    yes: bool,
    target: Option<PathBuf>,
    clean: bool,
    delete: bool,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let scan_root = target.unwrap_or_else(|| cwd.clone());
    let ctx = VaultContext::open(None, &scan_root)?;
    let opts = UpdateOptions {
        root: scan_root,
        vault_root: ctx.vault_root().to_path_buf(),
        dry_run,
        yes,
        clean,
        delete,
    };
    run_update(opts, ctx.db())?;
    Ok(())
}
