use std::path::PathBuf;


use crate::commands::find_vault_root;
use svault_core::db;
use svault_core::import::reconcile::{run_reconcile, ReconcileOptions};

pub fn run(
    dry_run: bool,
    yes: bool,
    target: Option<PathBuf>,
    clean: bool,
    delete: bool,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let scan_root = target.unwrap_or_else(|| cwd.clone());
    let vault_root = find_vault_root(None, &scan_root)?;
    let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
    let db = db::Db::open(&vault_root.join(".svault").join("vault.db"))
        .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;
    let opts = ReconcileOptions {
        root: scan_root,
        vault_root,
        dry_run,
        yes,
        clean,
        delete,
    };
    run_reconcile(opts, &db)?;
    Ok(())
}
