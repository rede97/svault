//! Scan command — output file status in svault's pipeable text format.
//!
//! Delegates entirely to [`ImportOptions::run_scan`] with a [`PipeReporterBuilder`]
//! so the scan logic is never duplicated relative to the import pipeline.
//!
//! # Output format
//! ```text
//! SCAN:/absolute/source/path
//! new:DCIM/IMG_0001.jpg
//! dup:DCIM/IMG_0003.jpg
//! fail:DCIM/broken.dng
//! ```
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

use crate::cli::OutputFormat;
use crate::reporting::PipeReporterBuilder;
use svault_core::config::SyncStrategy;
use svault_core::context::VaultContext;
use svault_core::import::ImportOptions;

pub fn run(_output: OutputFormat, source: PathBuf, show_dup: bool) -> anyhow::Result<()> {
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
    };

    let reporter_builder = PipeReporterBuilder::new(show_dup);
    opts.run_scan(db, &reporter_builder)?;

    Ok(())
}
