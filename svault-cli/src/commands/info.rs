//! `svault info` command wiring.

use svault_core::context::VaultContext;
use svault_core::ops::info;

use crate::cli::OutputFormat;

pub fn run(
    output: OutputFormat,
    path: Option<std::path::PathBuf>,
    hash: Option<String>,
) -> anyhow::Result<()> {
    let ctx = VaultContext::open_cwd()?;
    let report = match (path, hash) {
        (Some(p), None) => info::info_by_path(ctx.db(), ctx.vault_root(), &p.to_string_lossy())?,
        (None, Some(h)) => info::info_by_hash(ctx.db(), ctx.vault_root(), &h)?,
        _ => anyhow::bail!("give either a path or --hash <hex>"),
    };

    if matches!(output, OutputFormat::Json) {
        println!("{}", svault_ui::info::render_json(&report)?);
    } else {
        print!("{}", svault_ui::info::render_human(&report));
    }
    Ok(())
}
