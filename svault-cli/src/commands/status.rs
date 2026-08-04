use svault_core::context::VaultContext;
use svault_core::status;

use crate::cli::OutputFormat;

pub fn run(output: OutputFormat) -> anyhow::Result<()> {
    let ctx = VaultContext::open_cwd()?;
    let report =
        status::generate_report(ctx.vault_root(), ctx.db(), status::StatusOptions::default())?;
    if matches!(output, OutputFormat::Json) {
        println!("{}", svault_ui::status::render_json(&report)?);
    } else {
        print!("{}", svault_ui::status::render_human(&report));
    }
    Ok(())
}
