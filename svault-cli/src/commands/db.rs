use crate::cli::{DumpFormat, MtpCommand};
use svault_core::context::find_vault_root;
use svault_core::db;

pub fn run_verify_chain() -> anyhow::Result<()> {
    let vault_root = find_vault_root(None, &std::env::current_dir()?)?;
    let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
    
    let db_path = vault_root.join(".svault").join("vault.db");
    let db = db::Db::open(&db_path).map_err(|e| anyhow::anyhow!("cannot open db: {e}"))?;
    
    // Get event count for user feedback
    let count: i64 = db.conn_ref().query_row(
        "SELECT COUNT(*) FROM events",
        [],
        |row| row.get(0),
    )?;
    
    match db.verify_chain() {
        Ok(()) => {
            println!("✓ Hash chain verified: {} events intact", count);
            Ok(())
        }
        Err(e) => {
            anyhow::bail!("✗ Hash chain verification failed: {}", e);
        }
    }
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

    let result = db
        .dump(tables, limit)
        .map_err(|e| anyhow::anyhow!("dump failed: {e}"))?;

    // Print any warnings
    for warning in &result.warnings {
        eprintln!("Warning: {}", warning);
    }

    match format {
        DumpFormat::Csv => {
            print!("{}", db::render_csv(&result.dumps)?);
        }
        DumpFormat::Json => {
            println!("{}", db::render_json(&result.dumps)?);
        }
        DumpFormat::Sql => {
            print!("{}", db::render_sql(&result.dumps));
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
