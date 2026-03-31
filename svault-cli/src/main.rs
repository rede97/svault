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
                    use svault_core::import::vfs_import::{run_vfs_import, VfsImportOptions};
                    
                    let vault_root = find_vault_root(target.clone(), &std::env::current_dir()?)?;
                    let db = db::Db::open(&vault_root.join(".svault").join("vault.db"))
                        .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;
                    
                    let config = svault_core::config::Config::load(&vault_root)?;
                    let hash_algo = hash.unwrap_or(config.global.hash.clone());
                    
                    let manager = VfsManager::new();
                    let (backend, mtp_path) = manager.open_url(&source_str)
                        .map_err(|e| anyhow::anyhow!("failed to open MTP device: {e}"))?;
                    
                    let opts = VfsImportOptions {
                        src_backend: &*backend,
                        src_path: &mtp_path,
                        vault_root: &vault_root,
                        hash: hash_algo,
                        recheck,
                        dry_run: cli.dry_run,
                        yes: cli.yes,
                        show_dup,
                        import_config: config.import,
                        source_name: source_str.to_string(),
                        crc_buffer_size: 64 * 1024, // 64KB for MTP (good balance)
                    };
                    
                    let summary = run_vfs_import(opts, &db)?;
                    
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
                MtpCommand::Ls { path, long } => {
                    // If no path provided, list devices
                    let path = match path {
                        Some(p) => p,
                        None => {
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
                                        println!("    Storages:");
                                        for storage_name in source.roots.iter() {
                                            println!("      {}/", storage_name);
                                        }
                                    }
                                    println!();
                                }
                                println!("Browse examples:");
                                println!("  svault mtp ls mtp://1/                    # List storages");
                                println!("  svault mtp ls mtp://1/\"Internal Storage\"/# List internal storage");
                                println!("  svault mtp ls \"mtp://1/SD Card/\"          # List SD card");
                                println!("  svault mtp tree mtp://1/DCIM --depth 2    # Tree view");
                                println!();
                                println!("Then import with:");
                                println!("  svault import mtp://1/DCIM/Camera --target phone_backup");
                            }
                            return Ok(());
                        }
                    };
                    let (backend, mtp_path) = manager.open_url(&path)
                        .map_err(|e| anyhow::anyhow!("failed to open MTP device: {e}"))?;
                    
                    let entries = backend.list(&mtp_path)
                        .map_err(|e| anyhow::anyhow!("failed to list directory: {e}"))?;
                    
                    // Check if we're listing device root (storages)
                    let is_root = mtp_path.as_os_str().is_empty() || mtp_path == std::path::Path::new("/");
                    
                    if entries.is_empty() {
                        if is_root {
                            eprintln!("Device root appears empty.");
                            eprintln!();
                            eprintln!("This can happen if:");
                            eprintln!("  1. The device was 'ejected' in the file manager");
                            eprintln!("     → Reconnect the USB cable");
                            eprintln!("  2. The device is locked or screen is off");
                            eprintln!("     → Unlock the device and keep screen on");
                            eprintln!("  3. MTP permission was denied");
                            eprintln!("     → Check the device screen for permission prompt");
                        } else {
                            println!("Directory is empty.");
                        }
                    } else if is_root && entries.iter().all(|e| e.is_dir) {
                        // Listing storages
                        println!("Available storages:");
                        println!();
                        for entry in entries.iter() {
                            let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
                            println!("  {}/", name);
                        }
                        println!();
                        println!("Access a storage with: svault mtp ls mtp://1/\"Storage Name\"/");
                    } else {
                        // Normal directory listing
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

/// Global flag for shutdown signal
static SHUTDOWN_REQUESTED: std::sync::atomic::AtomicBool = 
    std::sync::atomic::AtomicBool::new(false);

/// Check if shutdown has been requested (for periodic checks in long operations)
pub fn is_shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(std::sync::atomic::Ordering::Relaxed)
}

fn setup_signal_handler() {
    ctrlc::set_handler(move || {
        eprintln!("\n⚠️  Received interrupt signal (Ctrl-C)");
        eprintln!("   Closing MTP device connection, please wait...");
        SHUTDOWN_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
        // Give the program a moment to clean up
        // The actual MTP session close happens in MtpFs::Drop
        std::thread::sleep(std::time::Duration::from_millis(800));
        std::process::exit(130); // 128 + SIGINT(2)
    }).expect("Error setting Ctrl-C handler");
}

fn main() {
    // Initialize logger (RUST_LOG env var controls level)
    // e.g., RUST_LOG=info, RUST_LOG=debug, RUST_LOG=svault_core::vfs::mtp=debug
    env_logger::init();
    
    // Setup signal handler for graceful shutdown on Ctrl-C
    setup_signal_handler();
    
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
