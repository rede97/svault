use svault_core::context::VaultContext;
use svault_core::status::{self, WorkingTreeFilter};

use crate::cli::OutputFormat;

pub fn run(
    output: OutputFormat,
    untracked: bool,
    moved: bool,
    missing: bool,
    modified: bool,
) -> anyhow::Result<()> {
    let ctx = VaultContext::open_cwd()?;
    let report =
        status::generate_report(ctx.vault_root(), ctx.db(), status::StatusOptions::default())?;
    let filter = WorkingTreeFilter {
        untracked,
        moved,
        missing,
        modified,
    };
    if matches!(output, OutputFormat::Json) {
        println!("{}", svault_ui::status::render_json(&report)?);
    } else {
        print!("{}", svault_ui::status::render_human(&report, &filter));
    }
    Ok(())
}
