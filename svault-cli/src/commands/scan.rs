//! Scan command (debug builds only) — output file status in svault's
//! pipeable text format.
//!
//! Delegates entirely to [`ImportOptions::run_scan`] with a [`PipeSink`]
//! so the scan logic is never duplicated relative to the import pipeline.
//!
//! # Example usage
//! ```bash
//! # Scan and pipe directly into import
//! svault scan /mnt/sdcard | svault import /mnt/sdcard --files-from -
//!
//! # Show duplicates in output
//! svault scan /mnt/sdcard --show-dup
//! ```

use std::path::PathBuf;

use svault_core::config::SyncStrategy;
use svault_core::context::VaultContext;
use svault_core::ops::ImportOptions;
use svault_ui::PipeSink;

pub fn run(source: PathBuf, show_dup: bool) -> anyhow::Result<()> {
    // Vault context is optional: without a vault we can still scan, but
    // duplicate detection is disabled and the default extension list is used.
    let vault_ctx = VaultContext::open(None, &source).ok();
    let db = vault_ctx.as_ref().map(|ctx| ctx.db());

    let opts = ImportOptions {
        source,
        vault_root: vault_ctx
            .as_ref()
            .map(|ctx| ctx.vault_root().to_path_buf())
            // Empty PathBuf → canonicalize fails → no vault path filtered out
            .unwrap_or_default(),
        strategy: SyncStrategy::default(),
        dry_run: false,
        yes: false,
        import_config: vault_ctx
            .as_ref()
            .map(|ctx| ctx.config().import.clone())
            .unwrap_or_default(),
        force: false,
        full_id: false,
        show_dup,
        files_from: None,
        max_depth: 0,
        include: Vec::new(),
        exclude: Vec::new(),
    };

    let sink = PipeSink::new(show_dup);
    opts.run_scan(db, &sink)?;

    Ok(())
}
