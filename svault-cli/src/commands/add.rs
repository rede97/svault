use std::path::PathBuf;

use crate::commands::find_vault_root;
use svault_core::config::HashAlgorithm;
use svault_core::db;
use svault_core::import::add::{run_add, AddOptions};

pub fn run(path: PathBuf, hash: Option<HashAlgorithm>) -> anyhow::Result<()> {
    let vault_root = find_vault_root(None, &path)?;
    let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
    let db = db::Db::open(&vault_root.join(".svault").join("vault.db"))
        .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;
    let config = svault_core::config::Config::load(&vault_root)?;
    let hash_algo = hash.unwrap_or(config.global.hash.clone());
    let opts = AddOptions {
        path,
        vault_root,
        hash: hash_algo,
    };
    run_add(opts, &db)?;
    Ok(())
}
