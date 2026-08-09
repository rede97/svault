use crate::cli::DumpFormat;
use svault_core::context::find_vault_root;
use svault_core::db;

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
