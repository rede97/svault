use crate::commands::format_bytes;
use std::path::Path;
use svault_core::vfs::VfsBackend;

pub fn run_ls(path: Option<String>, long: bool) -> anyhow::Result<()> {
    use svault_core::vfs::manager::VfsManager;

    let manager = VfsManager::new();

    // If no path provided, list devices
    let path = match path {
        Some(p) => p,
        None => {
            list_devices(&manager)?;
            return Ok(());
        }
    };

    let (backend, mtp_path) = manager
        .open_url(&path)
        .map_err(|e| anyhow::anyhow!("failed to open MTP device: {e}"))?;

    let entries = backend
        .list(&mtp_path)
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
            let name = entry
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
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
                println!(
                    "{} {:>10}  {}",
                    type_str,
                    size_str,
                    entry.path.file_name().unwrap_or_default().to_string_lossy()
                );
            } else {
                let suffix = if entry.is_dir { "/" } else { "" };
                println!(
                    "{}{}",
                    entry.path.file_name().unwrap_or_default().to_string_lossy(),
                    suffix
                );
            }
        }
    }
    Ok(())
}

fn list_devices(manager: &svault_core::vfs::manager::VfsManager) -> anyhow::Result<()> {
    let all_sources = manager
        .probe_all()
        .map_err(|e| anyhow::anyhow!("failed to probe devices: {e}"))?;

    // Filter to only MTP devices
    let mtp_sources: Vec<_> = all_sources
        .into_iter()
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
        println!(
            "  svault mtp ls mtp://1/\"Internal Storage\"/# List internal storage"
        );
        println!("  svault mtp ls \"mtp://1/SD Card/\"          # List SD card");
        println!("  svault mtp tree mtp://1/DCIM --depth 2    # Tree view");
        println!();
        println!("Then import with:");
        println!("  svault import mtp://1/DCIM/Camera --target phone_backup");
    }
    Ok(())
}

pub fn run_tree(path: String, depth: usize) -> anyhow::Result<()> {
    use svault_core::vfs::manager::VfsManager;

    let manager = VfsManager::new();
    let (backend, mtp_path) = manager
        .open_url(&path)
        .map_err(|e| anyhow::anyhow!("failed to open MTP device: {e}"))?;

    print_tree(&*backend, &mtp_path, "", depth, 0)?;
    Ok(())
}

fn print_tree(
    backend: &dyn VfsBackend,
    path: &Path,
    prefix: &str,
    max_depth: usize,
    current_depth: usize,
) -> anyhow::Result<()> {
    if current_depth >= max_depth {
        return Ok(());
    }

    let entries = backend
        .list(path)
        .map_err(|e| anyhow::anyhow!("failed to list directory: {e}"))?;

    let mut dirs: Vec<_> = entries.iter().filter(|e| e.is_dir).collect();
    let files: Vec<&svault_core::vfs::DirEntry> = entries.iter().filter(|e| !e.is_dir).collect();

    // Sort alphabetically
    dirs.sort_by(|a, b| a.path.file_name().cmp(&b.path.file_name()));

    let total = dirs.len() + files.len();

    // Print directories first
    for (i, entry) in dirs.iter().enumerate() {
        let is_last = i == total - 1 && files.is_empty();
        let connector = if is_last { "└── " } else { "├── " };
        let name = entry
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
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
        let name = entry
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let size = format_bytes(entry.size);
        println!("{}{}{} ({})", prefix, connector, name, size);
    }

    Ok(())
}
