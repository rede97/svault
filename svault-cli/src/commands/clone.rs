//! `svault clone` — clone a subset of files to a working directory.

use std::path::PathBuf;

use crate::cli::OutputFormat;
use crate::reporting::{JsonReporterBuilder, TerminalReporterBuilder};
use chrono::NaiveDate;
use svault_core::context::VaultContext;
use svault_core::db::FileRow;
use svault_core::sync;

/// Run clone command: export vault files to a plain directory.
pub fn run(
    output: OutputFormat,
    target: PathBuf,
    filter_date: Option<String>,
    filter_camera: Option<String>,
) -> anyhow::Result<()> {
    let ctx = VaultContext::open_cwd()?;
    let vault_root = ctx.vault_root().canonicalize()?;

    // Security check: ensure target is not inside vault
    let target_for_check = if target.exists() {
        target.canonicalize()?
    } else {
        normalize_path(&target)
    };
    if is_subdir(&target_for_check, &vault_root) {
        anyhow::bail!(
            "Target directory cannot be inside the vault: {} is within {}",
            target_for_check.display(),
            vault_root.display()
        );
    }
    let target = if target.exists() { target_for_check } else { target_for_check };

    // Ensure target exists
    std::fs::create_dir_all(&target)?;

    // Query and filter files (if no filters, let run_clone handle the query)
    let has_filters = filter_date.is_some() || filter_camera.is_some();
    let filtered: Option<Vec<FileRow>> = if has_filters {
        let candidates: Vec<FileRow> = ctx
            .db()
            .get_all_files()?
            .into_iter()
            .filter(|f| f.status == "imported")
            .collect();
        Some(apply_filters(candidates, &filter_date, &filter_camera)?)
    } else {
        None
    };

    let strategies: Vec<svault_core::fs::TransferStrategy> =
        vec![svault_core::fs::TransferStrategy::StreamCopy];

    let summary = match output {
        OutputFormat::Human => {
            let builder = TerminalReporterBuilder::new();
            sync::run_clone(&ctx, &target, &strategies, &builder, filtered.as_deref())?
        }
        OutputFormat::Json => {
            let builder = JsonReporterBuilder::new();
            sync::run_clone(&ctx, &target, &strategies, &builder, filtered.as_deref())?
        }
    };

    // Print summary
    match output {
        OutputFormat::Human => {
            if summary.transferred == 0 && summary.skipped == 0 && summary.failed == 0 {
                println!("Nothing to clone — all files already present.");
            } else {
                println!();
                println!("Clone complete:");
                println!("  Transferred: {:>6}", summary.transferred);
                println!("  Skipped:     {:>6}", summary.skipped);
                println!("  Failed:      {:>6}", summary.failed);
                if summary.total_bytes > 0 {
                    println!("  Total size:  {:>6}", crate::commands::format_bytes(summary.total_bytes));
                }
            }
        }
        OutputFormat::Json => {
            let json = serde_json::json!({
                "event": "clone_summary",
                "transferred": summary.transferred,
                "skipped": summary.skipped,
                "failed": summary.failed,
                "total_bytes": summary.total_bytes,
            });
            println!("{}", json);
        }
    }

    Ok(())
}

// ── Path helpers ────────────────────────────────────────────────────────────

fn is_subdir(child: &std::path::Path, parent: &std::path::Path) -> bool {
    if let Ok(relative) = child.strip_prefix(parent) {
        relative.as_os_str().is_empty() || relative.components().next().is_some()
    } else {
        false
    }
}

fn normalize_path(path: &std::path::Path) -> std::path::PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(path)
    };
    let mut components = Vec::new();
    for component in absolute.components() {
        match component {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                components.push(component);
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if let Some(std::path::Component::Normal(_)) = components.last() {
                    components.pop();
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

// ── Filtering ───────────────────────────────────────────────────────────────

fn apply_filters(
    candidates: Vec<FileRow>,
    filter_date: &Option<String>,
    filter_camera: &Option<String>,
) -> anyhow::Result<Vec<FileRow>> {
    let mut result = candidates;

    if let Some(date_range) = filter_date {
        let (start, end) = parse_date_range(date_range)?;
        result.retain(|f| {
            if let Some(file_date) = extract_date_from_path(&f.path) {
                file_date >= start && file_date <= end
            } else {
                false
            }
        });
    }

    if let Some(camera) = filter_camera {
        let camera_lower = camera.to_lowercase();
        result.retain(|f| f.path.to_lowercase().contains(&camera_lower));
    }

    Ok(result)
}

fn parse_date_range(range: &str) -> anyhow::Result<(NaiveDate, NaiveDate)> {
    let parts: Vec<&str> = range.split("..").collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid date range format. Expected: YYYY-MM-DD..YYYY-MM-DD");
    }
    let start = NaiveDate::parse_from_str(parts[0], "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("Invalid start date format: {}", parts[0]))?;
    let end = NaiveDate::parse_from_str(parts[1], "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("Invalid end date format: {}", parts[1]))?;
    Ok((start, end))
}

fn extract_date_from_path(path: &str) -> Option<NaiveDate> {
    let components: Vec<&str> = path.split('/').collect();
    if components.len() >= 2 {
        let year = components[0];
        let month_day = components[1];
        if year.len() == 4 && month_day.len() == 5 && month_day.contains('-') {
            let date_str = format!("{}-{} 00:00:00", year, month_day);
            return NaiveDate::parse_from_str(&date_str, "%Y-%m-%d %H:%M:%S").ok();
        }
    }
    None
}
