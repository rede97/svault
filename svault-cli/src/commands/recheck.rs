use std::path::PathBuf;

use crate::commands::find_vault_root;
use svault_core::config::HashAlgorithm;
use svault_core::db;
use svault_core::import::recheck::{run_recheck, RecheckOptions};
use svault_core::verify::manifest::ManifestManager;

pub fn run(
    source: Option<PathBuf>,
    target: Option<PathBuf>,
    session: Option<String>,
    hash: Option<HashAlgorithm>,
) -> anyhow::Result<()> {
    let vault_root = find_vault_root(target, &std::env::current_dir()?)?;
    let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
    let db = db::Db::open(&vault_root.join(".svault").join("vault.db"))
        .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;
    let config = svault_core::config::Config::load(&vault_root)?;
    let hash_algo = hash.unwrap_or(config.global.hash.clone());

    let manager = ManifestManager::new(&vault_root);
    let manifest = if let Some(session_id) = session {
        manager.load(&session_id)?
    } else {
        manager
            .latest()?
            .ok_or_else(|| anyhow::anyhow!("No import manifests found"))?
    };

    // Validate source path if explicitly provided
    if let Some(provided_source) = source {
        let provided = std::fs::canonicalize(&provided_source)
            .unwrap_or(provided_source)
            .to_string_lossy()
            .to_string();
        let recorded = std::fs::canonicalize(&manifest.source_root)
            .unwrap_or_else(|_| manifest.source_root.clone())
            .to_string_lossy()
            .to_string();
        if provided != recorded {
            anyhow::bail!(
                "Source path mismatch: provided '{}', but manifest records '{}'",
                provided,
                recorded
            );
        }
    }

    let opts = RecheckOptions {
        vault_root,
        manifest,
        hash: hash_algo,
    };
    run_recheck(opts, &db)?;
    Ok(())
}
