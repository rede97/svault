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
            // Check if source is a URL (mtp://) or local path
            let source_str = source.to_string_lossy();
            if source_str.starts_with("mtp://") {
                // MTP import via VFS
                #[cfg(feature = "mtp")]
                {
                    use svault_core::vfs::manager::VfsManager;
                    
                    let vault_root = find_vault_root(target.clone(), &std::env::current_dir()?)?;
                    let _db = db::Db::open(&vault_root.join(".svault").join("vault.db"))
                        .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;
                    
                    let manager = VfsManager::new();
                    let (backend, mtp_path) = manager.open_url(&source_str)
                        .map_err(|e| anyhow::anyhow!("failed to open MTP device: {e}"))?;
                    
                    // For now, list what would be imported
                    println!("MTP import from: {}", source_str);
                    println!("Vault: {}", vault_root.display());
                    if let Some(t) = target {
                        println!("Target: {}", t.display());
                    }
                    println!();
                    
                    // Walk and count files
                    fn count_files(
                        backend: &dyn svault_core::vfs::VfsBackend,
                        path: &Path,
                        total: &mut usize,
                    ) -> anyhow::Result<()> {
                        let entries = backend.list(path)
                            .map_err(|e| anyhow::anyhow!("failed to list: {e}"))?;
                        for entry in entries {
                            if entry.is_dir {
                                count_files(backend, &entry.path, total)?;
                            } else {
                                *total += 1;
                            }
                        }
                        Ok(())
                    }
                    
                    let mut total_files = 0;
                    count_files(&*backend, &mtp_path, &mut total_files)?;
                    println!("Found {} files to import", total_files);
                    println!();
                    println!("Full MTP import integration is coming soon!");
                    println!("Use 'svault mtp ls {}' to browse files.", source_str);
                    Ok(())
                }
                #[cfg(not(feature = "mtp"))]
                {
                    Err(anyhow::anyhow!("MTP support not enabled. Build with --features mtp"))
                }
            } else {
                // Local filesystem import
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
                    let all_sources = manager.probe_all()
                        .map_err(|e| anyhow::anyhow!("failed to probe devices: {e}"))?;
                    
                    // Filter to only MTP devices
                    let mtp_sources: Vec<_> = all_sources.into_iter()
                        .filter(|s| s.scheme == "mtp" && !s.id.starts_with("mtp://SN:"))
                        .collect();
                    
                    if mtp_sources.is_empty() {
                        println!("No MTP devices found.");
                        println!("Make sure your Android phone or camera is connected via USB");
                        println!("and set to 'File transfer' / 'MTP' mode.");
                    } else {
                        println!("Connected MTP devices:");
                        println!();
                        for source in &mtp_sources {
                            println!("  {}:", source.id);
                            println!("    Name:       {}", source.name);
                            println!("    Type:       {}", source.device_type);
                            println!("    Serial:     {}", source.unique_id);
                            if !source.roots.is_empty() {
                                println!("    Storages:   {}", source.roots.join(", "));
                            }
                            println!();
                        }
                        println!("Browse examples:");
                        println!("  svault mtp ls mtp://1/");
                        println!("  svault mtp ls mtp://1/DCIM");
                        println!("  svault mtp tree mtp://1/DCIM --depth 2");
                        println!();
                        println!("Then import with:");
                        println!("  svault import mtp://1/DCIM/Camera --target phone_backup");
                    }
                    Ok(())
                }
                MtpCommand::Ls { path, long } => {
                    let (backend, mtp_path) = manager.open_url(&path)
                        .map_err(|e| anyhow::anyhow!("failed to open MTP device: {e}"))?;
                    
                    let entries = backend.list(&mtp_path)
                        .map_err(|e| anyhow::anyhow!("failed to list directory: {e}"))?;
                    
                    if entries.is_empty() {
                        println!("Directory is empty.");
                    } else {
                        for entry in entries {
                            let type_str = if entry.is_dir { "d" } else { "-" };
                            if long {
                                let size_str = format_bytes(entry.size);
                                println!("{} {:>10}  {}", type_str, size_str, entry.path.file_name().unwrap_or_default().to_string_lossy());
                            } else {
                                let suffix = if entry.is_dir { "/" } else { "" };
                                println!("{}{}", entry.path.file_name().unwrap_or_default().to_string_lossy(), suffix);
                            }
                        }
                    }
                    Ok(())
                }
                MtpCommand::Tree { path, depth } => {
                    let (backend, mtp_path) = manager.open_url(&path)
                        .map_err(|e| anyhow::anyhow!("failed to open MTP device: {e}"))?;
                    
                    print_tree(&*backend, &mtp_path, "", depth, 0)?;
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

/// Format bytes to human readable string
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let exp = (bytes as f64).log(1024.0).min(4.0) as usize;
    let value = bytes as f64 / 1024f64.powi(exp as i32);
    if exp == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", value, UNITS[exp])
    }
}

/// Print directory tree for MTP browser
fn print_tree(
    backend: &dyn svault_core::vfs::VfsBackend,
    path: &Path,
    prefix: &str,
    max_depth: usize,
    current_depth: usize,
) -> anyhow::Result<()> {
    if current_depth >= max_depth {
        return Ok(());
    }

    let entries = backend.list(path)
        .map_err(|e| anyhow::anyhow!("failed to list directory: {e}"))?;

    let mut dirs: Vec<_> = entries.iter().filter(|e| e.is_dir).collect();
    let files: Vec<_> = entries.iter().filter(|e| !e.is_dir).collect();

    // Sort alphabetically
    dirs.sort_by(|a, b| a.path.file_name().cmp(&b.path.file_name()));

    let total = dirs.len() + files.len();
    
    // Print directories first
    for (i, entry) in dirs.iter().enumerate() {
        let is_last = i == total - 1 && files.is_empty();
        let connector = if is_last { "└── " } else { "├── " };
        let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
        println!("{}{}{}/", prefix, connector, name);
        
        let new_prefix = if is_last {
            format!("{}    ", prefix)
        } else {
            format!("{}│   ", prefix)
        };
        print_tree(backend, &entry.path, &new_prefix, max_depth, current_depth + 1)?;
    }

    // Then print files
    for (i, entry) in files.iter().enumerate() {
        let is_last = i == files.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
        let size = format_bytes(entry.size);
        println!("{}{}{} ({})", prefix, connector, name, size);
    }

    Ok(())
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
