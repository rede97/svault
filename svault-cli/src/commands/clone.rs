//! `svault clone` — clone a subset of files to a working directory.
//!
//! Reads from DB (status='imported'), applies filters, copies files to target.
//! Performs post-copy verification (size + xxh3).

use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::OutputFormat;
use anyhow::bail;
use chrono::NaiveDate;
use svault_core::context::VaultContext;
use svault_core::db::files::FileRow;
use svault_core::hash::xxh3_128_file;

/// Summary of clone operation
#[derive(Debug, Default)]
pub struct CloneSummary {
    pub selected: usize,
    pub copied: usize,
    pub skipped: usize,
    pub failed: usize,
    pub verify_failed: usize,
}

/// Run clone command
pub fn run(
    output: OutputFormat,
    target: PathBuf,
    filter_date: Option<String>,
    filter_camera: Option<String>,
) -> anyhow::Result<()> {
    // Open vault context
    let ctx = VaultContext::open_cwd()?;
    let vault_root = ctx.vault_root().canonicalize()?;
    let db = ctx.db();

    // Security check: Ensure target is not inside vault
    // For existing paths: use canonicalize to resolve symlinks
    // For non-existing paths: use lexical normalization (.. and .)
    let target_for_check = if target.exists() {
        target.canonicalize()?
    } else {
        normalize_path(&target)
    };

    if is_subdir(&target_for_check, &vault_root) {
        bail!(
            "Target directory cannot be inside the vault: {} is within {}",
            target_for_check.display(),
            vault_root.display()
        );
    }

    // Normalize target for use (use the checked path if it exists)
    let target = if target.exists() { target_for_check.clone() } else { target_for_check };

    // Ensure target exists
    fs::create_dir_all(&target)?;

    // Step 1: Query candidate files from DB
    let candidates = query_imported_files(db)?;

    // Step 2: Apply filters
    let filtered = apply_filters(
        candidates,
        &filter_date,
        &filter_camera,
    )?;

    let summary = if filtered.is_empty() {
        CloneSummary {
            selected: 0,
            ..Default::default()
        }
    } else {
        // Step 3: Copy files
        let mut summary = copy_files(&filtered, &vault_root, &target, &output)?;

        // Step 4: Verify copied files
        summary.verify_failed = verify_copied_files(&filtered, &vault_root, &target)?;

        summary.selected = filtered.len();
        summary
    };

    // Output summary
    print_summary(&output, &summary);

    Ok(())
}

/// Check if child is a subdirectory of parent
fn is_subdir(child: &Path, parent: &Path) -> bool {
    if let Ok(relative) = child.strip_prefix(parent) {
        // If relative is empty, they're the same
        // If relative has components, child is inside parent
        relative.as_os_str().is_empty() || relative.components().next().is_some()
    } else {
        false
    }
}

/// Normalize a path to absolute form, resolving .. and . components
/// Works even if the path doesn't exist (unlike canonicalize)
fn normalize_path(path: &Path) -> PathBuf {
    // First, make it absolute
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(path)
    };
    
    // Manually resolve . and .. components
    let mut components = Vec::new();
    for component in absolute.components() {
        match component {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                components.push(component);
            }
            std::path::Component::CurDir => {
                // Skip . components
            }
            std::path::Component::ParentDir => {
                // Pop the last normal component if possible
                if let Some(last) = components.last() {
                    match last {
                        std::path::Component::Normal(_) => {
                            components.pop();
                        }
                        _ => {
                            components.push(component);
                        }
                    }
                } else {
                    components.push(component);
                }
            }
            std::path::Component::Normal(_) => {
                components.push(component);
            }
        }
    }
    
    components.iter().collect()
}

/// Query all imported files from DB
fn query_imported_files(db: &svault_core::db::Db) -> anyhow::Result<Vec<FileRow>> {
    let all_files = db.get_all_files()?;
    Ok(all_files.into_iter()
        .filter(|f| f.status == "imported")
        .collect())
}

/// Apply filters to candidate files
fn apply_filters(
    candidates: Vec<FileRow>,
    filter_date: &Option<String>,
    filter_camera: &Option<String>,
) -> anyhow::Result<Vec<FileRow>> {
    let mut result = candidates;

    // Filter by date range (e.g., "2024-03-01..2024-03-31")
    if let Some(date_range) = filter_date {
        let (start, end) = parse_date_range(date_range)?;
        result.retain(|f| {
            // Extract date from path: vault uses YYYY/MM-DD/ structure
            if let Some(file_date) = extract_date_from_path(&f.path) {
                file_date >= start && file_date <= end
            } else {
                false
            }
        });
    }

    // Filter by camera model (path contains camera name)
    if let Some(camera) = filter_camera {
        let camera_lower = camera.to_lowercase();
        result.retain(|f| f.path.to_lowercase().contains(&camera_lower));
    }

    Ok(result)
}

/// Parse date range string "2024-03-01..2024-03-31"
fn parse_date_range(range: &str) -> anyhow::Result<(NaiveDate, NaiveDate)> {
    let parts: Vec<&str> = range.split("..").collect();
    if parts.len() != 2 {
        bail!("Invalid date range format. Expected: YYYY-MM-DD..YYYY-MM-DD");
    }
    
    let start = NaiveDate::parse_from_str(parts[0], "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("Invalid start date format: {}", parts[0]))?;
    let end = NaiveDate::parse_from_str(parts[1], "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("Invalid end date format: {}", parts[1]))?;
    
    Ok((start, end))
}

/// Extract date from vault path (assumes YYYY/MM-DD/ structure)
/// Returns NaiveDate for the first day of that month-day
fn extract_date_from_path(path: &str) -> Option<NaiveDate> {
    // Path format: 2024/03-15/Camera/file.jpg
    let components: Vec<&str> = path.split('/').collect();
    if components.len() >= 2 {
        let year = components[0];
        let month_day = components[1];
        // Parse as YYYY-MM-DD format
        if year.len() == 4 && month_day.len() == 5 && month_day.contains('-') {
            let date_str = format!("{}-{} 00:00:00", year, month_day);
            return NaiveDate::parse_from_str(&date_str, "%Y-%m-%d %H:%M:%S").ok();
        }
    }
    None
}

/// Copy files from vault to target
fn copy_files(
    files: &[FileRow],
    vault_root: &Path,
    target: &Path,
    output: &OutputFormat,
) -> anyhow::Result<CloneSummary> {
    let mut summary = CloneSummary::default();

    for file in files {
        let src = vault_root.join(&file.path);
        let dst = target.join(&file.path);

        // Create parent directory
        if let Some(parent) = dst.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                summary.failed += 1;
                if matches!(output, OutputFormat::Human) {
                    eprintln!("Failed to create directory {}: {}", parent.display(), e);
                }
                continue;
            }
        }

        // Check if destination exists
        if dst.exists() {
            let dst_meta = fs::metadata(&dst)?;
            if dst_meta.len() as i64 == file.size {
                // Same size, skip
                summary.skipped += 1;
                continue;
            } else {
                // Different size, fail (don't overwrite)
                summary.failed += 1;
                if matches!(output, OutputFormat::Human) {
                    eprintln!(
                        "Failed: {} exists with different size (not overwriting)",
                        dst.display()
                    );
                }
                continue;
            }
        }

        // Copy file
        match copy_file_with_fallback(&src, &dst) {
            Ok(_) => {
                summary.copied += 1;
            }
            Err(e) => {
                summary.failed += 1;
                if matches!(output, OutputFormat::Human) {
                    eprintln!("Failed to copy {}: {}", src.display(), e);
                }
            }
        }
    }

    Ok(summary)
}

/// Copy file from src to dst
fn copy_file_with_fallback(src: &Path, dst: &Path) -> anyhow::Result<()> {
    // Use standard fs::copy for simplicity in clone scenario
    // (target is outside vault, so reflink/hardlink benefits are limited)
    fs::copy(src, dst)?;
    Ok(())
}

/// Verify copied files (size + xxh3)
fn verify_copied_files(
    files: &[FileRow],
    _vault_root: &Path,
    target: &Path,
) -> anyhow::Result<usize> {
    let mut verify_failed = 0;

    for file in files {
        let dst = target.join(&file.path);

        // Skip if file doesn't exist (copy failed)
        if !dst.exists() {
            continue;
        }

        // Check size
        let dst_meta = match fs::metadata(&dst) {
            Ok(m) => m,
            Err(_) => {
                verify_failed += 1;
                continue;
            }
        };

        if dst_meta.len() as i64 != file.size {
            verify_failed += 1;
            continue;
        }

        // Check xxh3 hash if available
        if let Some(expected_hash) = &file.xxh3_128 {
            match xxh3_128_file(&dst) {
                Ok(actual_hash) => {
                    let actual_bytes = actual_hash.to_bytes();
                    if actual_bytes.as_slice() != expected_hash.as_slice() {
                        verify_failed += 1;
                    }
                }
                Err(_) => {
                    // Can't compute hash, skip verification
                }
            }
        }
    }

    Ok(verify_failed)
}

/// Print summary
fn print_summary(output: &OutputFormat, summary: &CloneSummary) {
    match output {
        OutputFormat::Human => {
            println!("\nClone Summary:");
            println!("  Selected:      {:>6}", summary.selected);
            println!("  Copied:        {:>6}", summary.copied);
            println!("  Skipped:       {:>6}", summary.skipped);
            println!("  Failed:        {:>6}", summary.failed);
            println!("  Verify Failed: {:>6}", summary.verify_failed);
        }
        OutputFormat::Json => {
            let json = serde_json::json!({
                "event": "clone_summary",
                "selected": summary.selected,
                "copied": summary.copied,
                "skipped": summary.skipped,
                "failed": summary.failed,
                "verify_failed": summary.verify_failed,
            });
            println!("{}", json);
        }
    }
}
