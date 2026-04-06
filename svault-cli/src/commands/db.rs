use crate::cli::{DumpFormat, MtpCommand};
use crate::commands::find_vault_root;
use svault_core::db;

pub fn run_verify_chain() -> anyhow::Result<()> {
    let _vault_root = find_vault_root(None, &std::env::current_dir()?)?;
    let _lock = svault_core::lock::acquire_vault_lock(&_vault_root)?;
    todo!("db verify-chain")
}

pub fn run_replay() -> anyhow::Result<()> {
    let _vault_root = find_vault_root(None, &std::env::current_dir()?)?;
    let _lock = svault_core::lock::acquire_vault_lock(&_vault_root)?;
    todo!("db replay")
}

pub fn run_dump(
    tables: Vec<String>,
    format: DumpFormat,
    limit: Option<usize>,
) -> anyhow::Result<()> {
    let vault_root = find_vault_root(None, &std::env::current_dir()?)?;
    let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
    let db_path = vault_root.join(".svault").join("vault.db");

    let db = db::Db::open(&db_path).map_err(|e| anyhow::anyhow!("cannot open db: {e}"))?;

    let dumps = db
        .dump(tables, limit)
        .map_err(|e| anyhow::anyhow!("dump failed: {e}"))?;

    match format {
        DumpFormat::Csv => {
            print!("{}", db::render_csv(&dumps)?);
        }
        DumpFormat::Json => {
            println!("{}", db::render_json(&dumps)?);
        }
        DumpFormat::Sql => {
            print!("{}", db::render_sql(&dumps));
        }
    }
    Ok(())
}

pub fn run_mtp(command: MtpCommand) -> anyhow::Result<()> {
    match command {
        MtpCommand::Ls { path, long } => super::mtp::run_ls(path, long),
        MtpCommand::Tree { path, depth } => super::mtp::run_tree(path, depth),
    }
}
