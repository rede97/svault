use std::path::PathBuf;
use std::sync::Arc;

use crate::reporting::TerminalReporterBuilder;
use svault_core::context::VaultContext;
use svault_core::import::recheck::{RecheckOptions, run_recheck};
use svault_core::verify::manifest::ManifestManager;

pub fn run(
    source: Option<PathBuf>,
    target: Option<PathBuf>,
    session: Option<String>,
) -> anyhow::Result<()> {
    let ctx = VaultContext::open(target, &std::env::current_dir()?)?;

    let manager = ManifestManager::new(ctx.vault_root());
    let manifest = if let Some(session_id) = session {
        manager.load(&session_id)?
    } else {
        manager
            .latest()?
            .ok_or_else(|| anyhow::anyhow!("No import manifests found"))?
    };

    // Validate source path if explicitly provided
    if let Some(provided_source) = source {
        let provided = dunce::canonicalize(&provided_source)
            .unwrap_or(provided_source)
            .to_string_lossy()
            .to_string();
        let recorded = dunce::canonicalize(&manifest.source_root)
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
        vault_root: ctx.vault_root().to_path_buf(),
        manifest,
    };
    let reporter_builder = Arc::new(TerminalReporterBuilder::new());
    run_recheck(opts, ctx.db(), reporter_builder.as_ref())?;
    Ok(())
}
