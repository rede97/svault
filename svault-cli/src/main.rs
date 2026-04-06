//! # svault-cli
//!
//! Command-line interface for **Svault** — a content-addressed multimedia archive.
//!
//! ## Quick start
//!
//! ```bash
//! # Initialize a vault
//! svault init
//!
//! # Import photos from a directory or device
//! svault import /path/to/photos
//!
//! # Check vault health
//! svault status
//! svault verify
//! ```
//!
//! ## Global flags
//!
//! | Flag | Description |
//! |------|-------------|
//! | `--output human\|json` | Output format |
//! | `--dry-run` | Preview changes without writing |
//! | `--yes` | Skip confirmation prompts |
//! | `--quiet` | Suppress non-error output |
//! | `--progress` | Emit JSON progress events to stderr |
//! | `--config <PATH>` | Path to config file |
//! | `--vault <PATH>` | Override vault root directory |
//!
//! ## Commands
//!
//! ### `init`
//! Initialize a new vault in the current directory by creating `.svault/` and the database.
//!
//! ### `import <SOURCE>`
//! Import media files from a source directory or device.
//!
//! - **Vault discovery**: svault walks up from `--target` (or CWD) to find `.svault/vault.db`.
//! - **Path template**: `svault.toml` controls the destination path. Default is `$year/$mon-$day/$device`.
//!   - `$year` / `$mon` / `$day` — from EXIF DateTimeOriginal, or file mtime if missing.
//!   - `$device` — EXIF camera model, or "Unknown Device".
//! - **Transfer strategy**: `--strategy reflink,hardlink` (default `reflink`; `copy` always falls back).
//! - **Hash algorithm**: `-H xxh3_128|sha256` for full-file collision resolution.
//! - **Manifest**: every import writes a timestamped manifest to `<vault_root>/manifest/`.
//!
//! > Svault never deletes your originals. Review the manifest and delete source files yourself.
//!
//! ### `recheck <SOURCE>`
//! Compare source files against the vault when everything hits the CRC32C cache.
//! Computes full-file hashes and writes a report to `.svault/staging/`.
//! Use this when you suspect previously-imported vault files may be corrupt,
//! or when you want to verify that your source files are still intact.
//! No files are imported or deleted — review the report and act manually.
//!
//! ### `add <PATH>`
//! Register files already physically inside the vault without moving them.
//! Use this when you have manually copied files into the vault directory.
//!
//! ### `sync <SOURCE_VAULT>`
//! Pull files and database records from another vault.
//! Event logs are compared directly, so duplicates are detected without re-hashing.
//! No files are deleted on either side.
//!
//! ### `reconcile --root <PATH>`
//! Locate files that were moved outside svault and update their paths in the database.
//!
//! ### `verify`
//! Check every file in the vault against its stored hash.
//!
//! | Option | Description |
//! |--------|-------------|
//! | `-H sha256\|xxh3_128` | Hash algorithm |
//! | `--file <PATH>` | Verify a single file |
//! | `--recent <SECONDS>` | Verify files imported in the last N seconds |
//!
//! ### `status`
//! Show a summary of the vault: imported files, duplicates, pending hashes, events, and DB size.
//!
//! ### `history`
//! Query the immutable event log. All changes are stored as events with a tamper-evident hash chain.
//!
//! ### `background-hash`
//! Compute SHA-256 for files that were imported without it. Safe to run when the system is idle.
//!
//! ### `clone --target <DIR>`
//! Copy a filtered subset of the vault to a local working directory (e.g. for offline editing).
//!
//! ### `mtp ls [PATH]` / `mtp tree <PATH>`
//! Browse MTP devices (Android phones, cameras) before importing. Use `svault import mtp://...`
//! to actually import files.
//!
//! ### `db`
//! Database maintenance subcommands:
//! - `db verify-chain` — verify the event-log hash chain.
//! - `db replay` — rebuild materialised views from the event log.
//! - `db dump [TABLES]` — export table contents for debugging.

pub mod cli;

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

use svault_core::verify::VerifyResult;

fn print_verify_results(
    results: &[(String, VerifyResult)],
    summary: &svault_core::verify::VerifySummary,
) -> anyhow::Result<()> {
    use console::style;

    let mut has_failures = false;
    for (path, result) in results {
        match result {
            VerifyResult::Ok => {}
            VerifyResult::Missing => {
                has_failures = true;
                println!("{} {} - Missing", style("✗").red().bold(), path);
            }
            VerifyResult::SizeMismatch { expected, actual } => {
                has_failures = true;
                println!("{} {} - Size mismatch (expected {}, got {})",
                    style("✗").red().bold(), path, expected, actual);
            }
            VerifyResult::HashMismatch { algo } => {
                has_failures = true;
                println!("{} {} - {} hash mismatch",
                    style("✗").red().bold(), path, algo);
            }
            VerifyResult::IoError(e) => {
                has_failures = true;
                println!("{} {} - IO error: {}",
                    style("✗").red().bold(), path, e);
            }
            VerifyResult::HashNotAvailable => {
                println!("{} {} - Hash not available",
                    style("!").yellow().bold(), path);
            }
        }
    }

    println!();
    println!("{}", style("Summary:").bold());
    println!(
        "  {} {}",
        style(format!("OK:               {:>6}", summary.ok)).green(),
        style("verified successfully")
    );
    if summary.missing > 0 {
        println!(
            "  {} {}",
            style(format!("Missing:          {:>6}", summary.missing)).red(),
            style("file not found on disk")
        );
    }
    if summary.size_mismatch > 0 {
        println!(
            "  {} {}",
            style(format!("Size mismatch:    {:>6}", summary.size_mismatch)).red(),
            style("file size differs from database")
        );
    }
    if summary.hash_mismatch > 0 {
        println!(
            "  {} {}",
            style(format!("Hash mismatch:    {:>6}", summary.hash_mismatch)).red(),
            style("hash does not match database")
        );
    }
    if summary.io_error > 0 {
        println!(
            "  {} {}",
            style(format!("IO error:         {:>6}", summary.io_error)).red(),
            style("unable to read file")
        );
    }
    if summary.hash_not_available > 0 {
        println!(
            "  {} {}",
            style(format!("Hash pending:     {:>6}", summary.hash_not_available)).yellow(),
            style("hash not yet computed")
        );
    }

    if has_failures {
        std::process::exit(1);
    }
    Ok(())
}

/// Parse a datetime string (RFC 3339 or YYYY-MM-DD) into Unix milliseconds.
fn parse_datetime_to_ms(s: &str) -> Option<i64> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        let dt_utc: chrono::DateTime<chrono::Utc> = dt.into();
        return Some(dt_utc.timestamp_millis());
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt: chrono::NaiveDateTime = date.and_hms_opt(0, 0, 0)?;
        return Some(dt.and_utc().timestamp_millis());
    }
    None
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Init => {
            let root = std::env::current_dir().expect("cannot read cwd");
            db::init(&root)
        }
        Command::Import { source, target, hash, strategy, show_dup, force, .. } => {
            // Check if source is a URL (mtp://) or local path
            let source_str = source.to_string_lossy();
            if source_str.starts_with("mtp://") {
                // MTP import via VFS
                #[cfg(feature = "mtp")]
                {
                    use svault_core::vfs::manager::VfsManager;
                    use svault_core::import::vfs_import::{run_vfs_import, VfsImportOptions};
                    
                    let vault_root = find_vault_root(target.clone(), &std::env::current_dir()?)?;
                    let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
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
                        dry_run: cli.dry_run,
                        yes: cli.yes,
                        show_dup,
                        import_config: config.import,
                        source_name: source_str.to_string(),
                        strategy: svault_core::config::SyncStrategy(strategy),
                        force,
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
                let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
                let db = db::Db::open(&vault_root.join(".svault").join("vault.db"))
                    .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;
                let config = svault_core::config::Config::load(&vault_root)?;
                let hash_algo = hash.unwrap_or(config.global.hash.clone());
                let opts = ImportOptions {
                    source,
                    vault_root,
                    hash: hash_algo,
                    strategy: svault_core::config::SyncStrategy(strategy),
                    dry_run: cli.dry_run,
                    yes: cli.yes,
                    show_dup,
                    import_config: config.import,
                    force,
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
        Command::Recheck { source, target, session, hash } => {
            let vault_root = find_vault_root(target, &std::env::current_dir()?)?;
            let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
            let db = db::Db::open(&vault_root.join(".svault").join("vault.db"))
                .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;
            let config = svault_core::config::Config::load(&vault_root)?;
            let hash_algo = hash.unwrap_or(config.global.hash.clone());

            use svault_core::verify::manifest::ManifestManager;
            let manager = ManifestManager::new(&vault_root);
            let manifest = if let Some(session_id) = session {
                manager.load(&session_id)?
            } else {
                manager.latest()?.ok_or_else(|| anyhow::anyhow!("No import manifests found"))?
            };

            // Validate source path if explicitly provided
            if let Some(provided_source) = source {
                let provided = std::fs::canonicalize(&provided_source)
                    .unwrap_or(provided_source)
                    .to_string_lossy()
                    .to_string();
                let recorded = std::fs::canonicalize(&manifest.source_root)
                    .unwrap_or_else(|_| manifest.source_root.clone())
                    .to_string_lossy()
                    .to_string();
                if provided != recorded {
                    anyhow::bail!(
                        "Source path mismatch: provided '{}', but manifest records '{}'",
                        provided,
                        recorded
                    );
                }
            }

            use svault_core::import::recheck::{run_recheck, RecheckOptions};
            let opts = RecheckOptions {
                vault_root,
                manifest,
                hash: hash_algo,
            };
            run_recheck(opts, &db)?;
            Ok(())
        }
        Command::Add { path, hash } => {
            let vault_root = find_vault_root(cli.vault, &path)?;
            let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
            let db = db::Db::open(&vault_root.join(".svault").join("vault.db"))
                .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;
            let config = svault_core::config::Config::load(&vault_root)?;
            let hash_algo = hash.unwrap_or(config.global.hash.clone());
            let opts = svault_core::import::add::AddOptions {
                path,
                vault_root,
                hash: hash_algo,
            };
            svault_core::import::add::run_add(opts, &db)?;
            Ok(())
        }
        Command::Sync { .. } => todo!("sync"),
        Command::Reconcile { target, clean, delete } => {
            let cwd = std::env::current_dir()?;
            let scan_root = target.unwrap_or_else(|| cwd.clone());
            let vault_root = find_vault_root(cli.vault, &scan_root)?;
            let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
            let db = db::Db::open(&vault_root.join(".svault").join("vault.db"))
                .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;
            let opts = svault_core::import::reconcile::ReconcileOptions {
                root: scan_root,
                vault_root,
                dry_run: cli.dry_run,
                yes: cli.yes,
                clean,
                delete,
            };
            svault_core::import::reconcile::run_reconcile(opts, &db)?;
            Ok(())
        }
        Command::Verify { hash, file, recent, upgrade_links, background_hash, background_hash_limit, background_hash_nice } => {
            use svault_core::verify::{verify_all, verify_single, verify_recent, VerifyResult};
            use console::style;

            let vault_root = find_vault_root(cli.vault, &std::env::current_dir()?)?;
            let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
            let db = db::Db::open(&vault_root.join(".svault").join("vault.db"))
                .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;
            let config = svault_core::config::Config::load(&vault_root)?;
            let algo = hash.unwrap_or(config.global.hash.clone());

            if background_hash {
                let opts = svault_core::background_hash::BackgroundHashOptions {
                    vault_root: vault_root.clone(),
                    limit: background_hash_limit,
                    nice: background_hash_nice,
                };
                let summary = svault_core::background_hash::run_background_hash(opts, &db)?;
                println!(
                    "Processed {} file(s), {} failed.",
                    summary.processed, summary.failed
                );
            }

            if upgrade_links {
                // Upgrade hardlinked files before verification.
                let files_to_check: Vec<svault_core::db::FileRow> = if let Some(seconds) = recent {
                    db.get_recent_files(seconds)?
                } else if let Some(ref file_path) = file {
                    if let Some(f) = db.get_file_by_path(&file_path.to_string_lossy())? {
                        vec![f]
                    } else {
                        Vec::new()
                    }
                } else {
                    db.get_all_files()?
                };

                for file_row in files_to_check {
                    let full_path = vault_root.join(&file_row.path);
                    match svault_core::hardlink_upgrade::is_hardlinked(&full_path) {
                        Ok(true) => {
                            if let Err(e) = svault_core::hardlink_upgrade::upgrade_to_binary_copy(&full_path) {
                                eprintln!("  {} Failed to upgrade hardlink {}: {}",
                                    style("⚠").yellow().bold(), full_path.display(), e);
                            } else {
                                eprintln!("  {} Upgraded hardlink {}",
                                    style("→").cyan(), full_path.display());
                            }
                        }
                        Ok(false) => {}
                        Err(e) => {
                            eprintln!("  {} Failed to check {}: {}",
                                style("⚠").yellow().bold(), full_path.display(), e);
                        }
                    }
                }
            }

            if let Some(seconds) = recent {
                println!(
                    "{} Verifying files imported in the last {} seconds",
                    style("Verify:").bold().cyan(),
                    style(seconds).cyan()
                );
                let (results, summary) = verify_recent(&vault_root, &db, &algo, seconds)?;
                print_verify_results(&results, &summary)?;
                return Ok(());
            }

            if let Some(file_path) = file {
                match verify_single(&vault_root, &db, &file_path.to_string_lossy(), &algo)? {
                    Some(result) => {
                        match result {
                            VerifyResult::Ok => {
                                println!("{} {}", style("✓").green().bold(), file_path.display());
                            }
                            VerifyResult::Missing => {
                                println!("{} {} - File not found", style("✗").red().bold(), file_path.display());
                                std::process::exit(1);
                            }
                            VerifyResult::SizeMismatch { expected, actual } => {
                                println!("{} {} - Size mismatch (expected {}, got {})",
                                    style("✗").red().bold(), file_path.display(), expected, actual);
                                std::process::exit(1);
                            }
                            VerifyResult::HashMismatch { algo } => {
                                println!("{} {} - Hash mismatch ({})",
                                    style("✗").red().bold(), file_path.display(), algo);
                                std::process::exit(1);
                            }
                            VerifyResult::IoError(e) => {
                                println!("{} {} - IO error: {}",
                                    style("✗").red().bold(), file_path.display(), e);
                                std::process::exit(1);
                            }
                            VerifyResult::HashNotAvailable => {
                                println!("{} {} - Hash not computed yet",
                                    style("!").yellow().bold(), file_path.display());
                            }
                        }
                    }
                    None => {
                        anyhow::bail!("File not found in database: {}", file_path.display());
                    }
                }
            } else {
                println!(
                    "{} Verifying all files in vault",
                    style("Verify:").bold().cyan()
                );
                let (results, summary) = verify_all(&vault_root, &db, &algo)?;
                print_verify_results(&results, &summary)?;
            }

            Ok(())
        }
        Command::Status => {
            let vault_root = find_vault_root(cli.vault, &std::env::current_dir()?)?;
            let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
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
        Command::History { file, from, to, event_type, limit, by_session, files } => {
            let vault_root = find_vault_root(cli.vault, &std::env::current_dir()?)?;
            let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;

            if by_session {
                // Show history grouped by import session
                use console::style;
                use svault_core::verify::manifest::ManifestManager;
                let manager = ManifestManager::new(&vault_root);
                
                let manifests = manager.list_all()
                    .map_err(|e| anyhow::anyhow!("cannot list manifests: {e}"))?;

                if manifests.is_empty() {
                    println!("No import sessions found.");
                    return Ok(());
                }

                if matches!(cli.output, cli::OutputFormat::Json) {
                    let sessions: Vec<_> = manifests.iter().map(|(_, m)| {
                        serde_json::json!({
                            "session_id": m.session_id,
                            "imported_at": m.imported_at,
                            "source_root": m.source_root,
                            "file_count": m.files.len(),
                            "total_size": m.files.iter().map(|f| f.size).sum::<u64>(),
                            "hash_algorithm": m.hash_algorithm,
                            "files": if files { 
                                m.files.iter().map(|f| serde_json::json!({
                                    "src": f.src_path,
                                    "dest": f.dest_path,
                                    "size": f.size,
                                })).collect::<Vec<_>>()
                            } else {
                                vec![]
                            },
                        })
                    }).collect();
                    println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "sessions": sessions }))?);
                } else {
                    println!("{}", style("Import History").bold().underlined());
                    println!();
                    
                    for (i, (_, manifest)) in manifests.iter().enumerate() {
                        let datetime = chrono::DateTime::from_timestamp_millis(manifest.imported_at)
                            .unwrap_or_default();
                        let total_size: u64 = manifest.files.iter().map(|f| f.size).sum();
                        let size_str = format_size(total_size);
                        
                        // Session header
                        println!("{} {} {}", 
                            style(format!("[{}]", i + 1)).cyan().bold(),
                            style(datetime.format("%Y-%m-%d %H:%M:%S")).bright(),
                            style(&manifest.session_id).dim()
                        );
                        
                        // Source and stats
                        println!("  Source: {}", style(manifest.source_root.display()).blue());
                        println!("  Files:  {} ({})", 
                            style(manifest.files.len()).yellow(),
                            style(size_str).green()
                        );
                        println!("  Hash:   {}", style(&manifest.hash_algorithm).dim());
                        
                        // File list (if requested)
                        if files && !manifest.files.is_empty() {
                            println!("  Files:");
                            for (_j, file) in manifest.files.iter().take(10).enumerate() {
                                println!("    {} {} → {}",
                                    style("•").dim(),
                                    style(file.src_path.file_name().unwrap_or_default().to_string_lossy()).dim(),
                                    style(&file.dest_path.display()).dim()
                                );
                            }
                            if manifest.files.len() > 10 {
                                println!("    ... and {} more files", manifest.files.len() - 10);
                            }
                        }
                        
                        println!();
                    }
                }
                Ok(())
            } else {
                // Original event-based history
                let db = db::Db::open(&vault_root.join(".svault").join("vault.db"))
                    .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;

                let from_ms = from.as_ref().and_then(|s| parse_datetime_to_ms(s));
                let to_ms = to.as_ref().and_then(|s| parse_datetime_to_ms(s));
                let file_path = file.as_ref().map(|p| p.to_string_lossy().to_string());

                let events = db.get_events(
                    limit,
                    event_type.as_deref(),
                    from_ms,
                    to_ms,
                    file_path.as_deref(),
                )?;

                if matches!(cli.output, cli::OutputFormat::Json) {
                    let json = serde_json::json!({
                        "events": events.iter().map(|e| serde_json::json!({
                            "seq": e.seq,
                            "occurred_at": e.occurred_at,
                            "event_type": e.event_type,
                            "entity_type": e.entity_type,
                            "entity_id": e.entity_id,
                            "payload": e.payload,
                            "prev_hash": e.prev_hash,
                            "self_hash": e.self_hash,
                        })).collect::<Vec<_>>(),
                    });
                    println!("{}", serde_json::to_string_pretty(&json)?);
                } else {
                    if events.is_empty() {
                        println!("No events found.");
                        return Ok(());
                    }
                    println!("{:>6}  {:<22}  {:<20}  payload", "seq", "time", "event");
                    for e in events {
                        let datetime = chrono::DateTime::from_timestamp_millis(e.occurred_at).unwrap_or_default();
                        println!("{:>6}  {:<22}  {:<20}  {}", e.seq, datetime.format("%Y-%m-%d %H:%M:%S"), e.event_type, e.payload);
                    }
                }
                Ok(())
            }
        }

        Command::Clone { .. } => {
            let vault_root = find_vault_root(cli.vault, &std::env::current_dir()?)?;
            let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
            todo!("clone")
        }
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
            let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
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

/// Format byte size to human-readable string.
fn format_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if size == 0 {
        return "0 B".to_string();
    }
    let exp = (size as f64).log(1024.0).min(UNITS.len() as f64 - 1.0) as usize;
    let value = size as f64 / 1024_f64.powi(exp as i32);
    if exp == 0 {
        format!("{} {}", size, UNITS[0])
    } else {
        format!("{:.2} {}", value, UNITS[exp])
    }
}
