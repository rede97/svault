mod cli;

use std::path::{Path, PathBuf};

use clap::Parser;
use cli::{Cli, Command, DbCommand, DumpFormat};
use svault_core::db;
use svault_core::import::{ImportOptions, run as import_run};

/// Walk up from `start` looking for `.svault/vault.db`.
fn find_vault_root(target: Option<PathBuf>, source: &Path) -> anyhow::Result<PathBuf> {
    let start = target
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| source.to_path_buf());
    let mut cur: &Path = &start;
    loop {
        if cur.join(".svault").join("vault.db").exists() {
            return Ok(cur.to_path_buf());
        }
        match cur.parent() {
            Some(p) => cur = p,
            None => anyhow::bail!(
                "no vault found (no .svault/vault.db in {} or any parent). \
                 Run `svault init` first.",
                start.display()
            ),
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Init => {
            let root = std::env::current_dir().expect("cannot read cwd");
            db::init(&root)
        }
        Command::Import { source, target, hash, recheck, show_dup, .. } => {
            let vault_root = find_vault_root(target, &source)?;
            let db = db::Db::open(&vault_root.join(".svault").join("vault.db"))
                .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;
            let config = svault_core::config::Config::load(&vault_root)?;
            let hash_algo = hash.unwrap_or(config.global.hash.clone());
            let opts = ImportOptions {
                source,
                vault_root,
                hash: hash_algo,
                recheck,
                dry_run: cli.dry_run,
                yes: cli.yes,
                show_dup,
                import_config: config.import,
            };
            let summary = import_run(opts, &db)?;
            if matches!(cli.output, cli::OutputFormat::Json) {
                println!("{}", serde_json::json!({
                    "total":         summary.total,
                    "imported":      summary.imported,
                    "duplicate":     summary.duplicate,
                    "failed":        summary.failed,
                    "all_cache_hit": summary.all_cache_hit,
                    "manifest":      summary.manifest_path.map(|p| p.display().to_string()),
                }));
            }
            Ok(())
        }
        Command::Add { .. } => todo!("add"),
        Command::Sync { .. } => todo!("sync"),
        Command::Reconcile { .. } => todo!("reconcile"),
        Command::Verify { .. } => todo!("verify"),
        Command::Status => {
            let vault_root = find_vault_root(cli.vault, &std::env::current_dir()?)?;
            let db = db::Db::open(&vault_root.join(".svault").join("vault.db"))
                .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;
            let report = svault_core::status::generate_report(
                &vault_root,
                &db,
                svault_core::status::StatusOptions::default(),
            )?;
            if matches!(cli.output, cli::OutputFormat::Json) {
                println!("{}", svault_core::status::render_json(&report)?);
            } else {
                print!("{}", svault_core::status::render_human(&report));
            }
            Ok(())
        }
        Command::History { .. } => todo!("history"),
        Command::BackgroundHash { .. } => todo!("background-hash"),
        Command::Clone { .. } => todo!("clone"),
        #[cfg(feature = "mtp")]
        Command::Mtp { command } => {
            use cli::MtpCommand;
            use svault_core::vfs::manager::VfsManager;
            
            let manager = VfsManager::new();
            
            match command {
                MtpCommand::List => {
                    let sources = manager.probe_all()
                        .map_err(|e| anyhow::anyhow!("failed to probe devices: {e}"))?;
                    
                    if sources.is_empty() {
                        println!("No MTP devices found.");
                        println!("Make sure your Android phone or camera is connected via USB");
                        println!("and set to 'File transfer' / 'MTP' mode.");
                    } else {
                        println!("Connected MTP devices:");
                        println!();
                        for source in &sources {
                            if source.id.starts_with("mtp://SN:") {
                                continue; // Skip SN entries for cleaner output
                            }
                            println!("  {}:", source.id);
                            println!("    Name:       {}", source.name);
                            println!("    Type:       {}", source.device_type);
                            println!("    Serial:     {}", source.unique_id);
                            if !source.roots.is_empty() {
                                println!("    Storages:   {}", source.roots.join(", "));
                            }
                            println!();
                        }
                        println!("Import examples:");
                        println!("  svault mtp import mtp://1/DCIM/Camera");
                        println!("  svault mtp import \"mtp://{}/DCIM/Camera\"", sources.first().map(|s| &s.name).unwrap_or(&"Device".to_string()).replace(' ', "%20"));
                    }
                    Ok(())
                }
                MtpCommand::Import { source, target, recheck, show_dup } => {
                    // Open vault
                    let vault_root = find_vault_root(target, &std::env::current_dir()?)?;
                    let db = db::Db::open(&vault_root.join(".svault").join("vault.db"))
                        .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;
                    let config = svault_core::config::Config::load(&vault_root)?;
                    
                    // Import from MTP URL
                    println!("Importing from {}...", source);
                    
                    let (backend, path) = manager.open_url(&source)
                        .map_err(|e| anyhow::anyhow!("failed to open MTP device: {e}"))?;
                    
                    // TODO: Implement MTP import using the backend
                    // For now, show what we found
                    let entries = backend.list(&path)
                        .map_err(|e| anyhow::anyhow!("failed to list directory: {e}"))?;
                    
                    println!("Found {} items:", entries.len());
                    for entry in entries.iter().take(10) {
                        let type_str = if entry.is_dir { "DIR" } else { "FILE" };
                        println!("  [{}] {} ({} bytes)", type_str, entry.path.display(), entry.size);
                    }
                    if entries.len() > 10 {
                        println!("  ... and {} more", entries.len() - 10);
                    }
                    
                    println!();
                    println!("Full MTP import is coming soon!");
                    println!("Use 'svault mtp list' to see available devices.");
                    
                    Ok(())
                }
            }
        }
        
        Command::Db { command } => {
            let vault_root = cli.vault
                .or_else(|| std::env::current_dir().ok())
                .ok_or_else(|| anyhow::anyhow!("cannot determine vault root"))?;
            let db_path = vault_root.join(".svault").join("vault.db");
            
            match command {
                DbCommand::VerifyChain => todo!("db verify-chain"),
                DbCommand::Replay { .. } => todo!("db replay"),
                DbCommand::Dump { tables, format, limit } => {
                    let db = svault_core::db::Db::open(&db_path)
                        .map_err(|e| anyhow::anyhow!("cannot open db: {e}"))?;
                    
                    let dumps = db.dump(tables, limit)
                        .map_err(|e| anyhow::anyhow!("dump failed: {e}"))?;
                    
                    match format {
                        DumpFormat::Csv => {
                            print!("{}", svault_core::db::render_csv(&dumps)?);
                        }
                        DumpFormat::Json => {
                            println!("{}", svault_core::db::render_json(&dumps)?);
                        }
                        DumpFormat::Sql => {
                            print!("{}", svault_core::db::render_sql(&dumps));
                        }
                    }
                    Ok(())
                }
            }
        }
    }
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
