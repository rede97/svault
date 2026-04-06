use crate::cli::OutputFormat;
use crate::context::VaultContext;
use svault_core::status;

pub fn run(output: OutputFormat) -> anyhow::Result<()> {
    let ctx = VaultContext::open_cwd()?;
    let report = status::generate_report(
        ctx.vault_root(),
        ctx.db(),
        status::StatusOptions::default(),
    )?;
    if matches!(output, OutputFormat::Json) {
        println!("{}", status::render_json(&report)?);
    } else {
        print!("{}", status::render_human(&report));
    }
    Ok(())
}
