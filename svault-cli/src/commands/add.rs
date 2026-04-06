use std::path::PathBuf;

use crate::context::VaultContext;
use svault_core::config::HashAlgorithm;
use svault_core::import::add::{run_add, AddOptions};

pub fn run(path: PathBuf, hash: Option<HashAlgorithm>) -> anyhow::Result<()> {
    let ctx = VaultContext::open(None, &path)?;
    let hash_algo = hash.unwrap_or_else(|| ctx.default_hash());
    let opts = AddOptions {
        path,
        vault_root: ctx.vault_root().to_path_buf(),
        hash: hash_algo,
    };
    run_add(opts, ctx.db())?;
    Ok(())
}
