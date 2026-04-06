use std::path::PathBuf;

use crate::cli::OutputFormat;
use crate::context::VaultContext;
use svault_core::config::SyncStrategy;
use svault_core::import::{run as import_run, ImportOptions};

pub fn run(
    output: OutputFormat,
    dry_run: bool,
    yes: bool,
    source: PathBuf,
    target: Option<PathBuf>,
    hash: Option<svault_core::config::HashAlgorithm>,
    strategy: Vec<svault_core::config::TransferStrategyArg>,
    force: bool,
) -> anyhow::Result<()> {
    let source_str = source.to_string_lossy();
    
    if source_str.starts_with("mtp://") {
        // MTP import via VFS
        #[cfg(feature = "mtp")]
        {
            run_mtp_import(output, dry_run, yes, &source_str, target, hash, strategy, force)
        }
        #[cfg(not(feature = "mtp"))]
        {
            Err(anyhow::anyhow!("MTP support not enabled. Build with --features mtp"))
        }
    } else {
        // Local filesystem import
        run_local_import(output, dry_run, yes, source, target, hash, strategy, force)
    }
}

#[cfg(feature = "mtp")]
fn run_mtp_import(
    output: OutputFormat,
    dry_run: bool,
    yes: bool,
    source_str: &str,
    target: Option<PathBuf>,
    hash: Option<svault_core::config::HashAlgorithm>,
    strategy: Vec<svault_core::config::TransferStrategyArg>,
    force: bool,
) -> anyhow::Result<()> {
    use svault_core::import::vfs_import::{run_vfs_import, VfsImportOptions};
    use svault_core::vfs::manager::VfsManager;

    let ctx = VaultContext::open(target, &std::env::current_dir()?)?;
    let hash_algo = hash.unwrap_or_else(|| ctx.default_hash());

    let manager = VfsManager::new();
    let (backend, mtp_path) = manager
        .open_url(source_str)
        .map_err(|e| anyhow::anyhow!("failed to open MTP device: {e}"))?;

    let opts = VfsImportOptions {
        src_backend: &*backend,
        src_path: &mtp_path,
        vault_root: ctx.vault_root(),
        hash: hash_algo,
        dry_run,
        yes,
        import_config: ctx.config().import.clone(),
        source_name: source_str.to_string(),
        strategy: SyncStrategy(strategy),
        force,
        crc_buffer_size: 64 * 1024, // 64KB for MTP (good balance)
    };

    let summary = run_vfs_import(opts, ctx.db())?;

    if matches!(output, OutputFormat::Json) {
        println!(
            "{}",
            serde_json::json!({
                "total": summary.total,
                "imported": summary.imported,
                "duplicate": summary.duplicate,
                "failed": summary.failed,
                "all_cache_hit": summary.all_cache_hit,
                "manifest": summary.manifest_path.map(|p| p.display().to_string()),
            })
        );
    }
    Ok(())
}

fn run_local_import(
    output: OutputFormat,
    dry_run: bool,
    yes: bool,
    source: PathBuf,
    target: Option<PathBuf>,
    hash: Option<svault_core::config::HashAlgorithm>,
    strategy: Vec<svault_core::config::TransferStrategyArg>,
    force: bool,
) -> anyhow::Result<()> {
    let ctx = VaultContext::open(target, &source)?;
    let hash_algo = hash.unwrap_or_else(|| ctx.default_hash());
    let opts = ImportOptions {
        source,
        vault_root: ctx.vault_root().to_path_buf(),
        hash: hash_algo,
        strategy: SyncStrategy(strategy),
        dry_run,
        yes,
        import_config: ctx.config().import.clone(),
        force,
    };
    let summary = import_run(opts, ctx.db())?;
    if matches!(output, OutputFormat::Json) {
        println!(
            "{}",
            serde_json::json!({
                "total": summary.total,
                "imported": summary.imported,
                "duplicate": summary.duplicate,
                "failed": summary.failed,
                "all_cache_hit": summary.all_cache_hit,
                "manifest": summary.manifest_path.map(|p| p.display().to_string()),
            })
        );
    }
    Ok(())
}
