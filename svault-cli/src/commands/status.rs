use crate::cli::OutputFormat;
use crate::commands::find_vault_root;
use svault_core::db;
use svault_core::status;

pub fn run(output: OutputFormat) -> anyhow::Result<()> {
    let vault_root = find_vault_root(None, &std::env::current_dir()?)?;
    let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
    let db = db::Db::open(&vault_root.join(".svault").join("vault.db"))
        .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;
    let report = status::generate_report(
        &vault_root,
        &db,
        status::StatusOptions::default(),
    )?;
    if matches!(output, OutputFormat::Json) {
        println!("{}", status::render_json(&report)?);
    } else {
        print!("{}", status::render_human(&report));
    }
    Ok(())
}
