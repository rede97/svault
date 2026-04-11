//! Vault status reporting with rich terminal output.

use std::path::Path;

use rich_rust::r#box::BoxChars;
use rich_rust::prelude::*;
use rich_rust::renderables::Renderable;

/// Custom box style: only heavy header separator (continuous), no vertical dividers.
const CLEAN_STYLE: BoxChars = BoxChars::new(
    [' ', ' ', ' ', ' '], // No top border
    [' ', ' ', ' ', ' '], // No vertical dividers for body
    [' ', '━', '━', ' '], // Heavy continuous line for header separator
    [' ', ' ', ' ', ' '], // No mid separator
    [' ', ' ', ' ', ' '], // No row separators
    [' ', ' ', ' ', ' '], // No foot row separator
    [' ', ' ', ' ', ' '], // No footer vertical dividers
    [' ', ' ', ' ', ' '], // No bottom border
    false,
);

use crate::db::{Db, ExtensionStats, VaultStats, format_bytes, format_count};

/// Status report for a vault.
#[derive(Debug, Clone)]
pub struct StatusReport {
    /// Path to the vault root.
    pub vault_root: std::path::PathBuf,
    /// Database file path.
    pub db_path: std::path::PathBuf,
    /// Overall statistics.
    pub stats: VaultStats,
    /// Top file extensions by size.
    pub top_extensions: Vec<ExtensionStats>,
    /// Files imported in the last 24 hours.
    pub imports_last_24h: i64,
    /// Files imported in the last 7 days.
    pub imports_last_7d: i64,
    /// Files imported in the last 30 days.
    pub imports_last_30d: i64,
}

/// Options for generating a status report.
#[derive(Debug, Clone)]
pub struct StatusOptions {
    /// Number of top extensions to show.
    pub top_extensions_limit: i64,
}

impl Default for StatusOptions {
    fn default() -> Self {
        Self {
            top_extensions_limit: 10,
        }
    }
}

/// Generates a status report for the vault at `vault_root`.
pub fn generate_report(
    vault_root: &Path,
    db: &Db,
    opts: StatusOptions,
) -> anyhow::Result<StatusReport> {
    let stats = db.vault_stats()?;
    let top_extensions = db.extension_stats(opts.top_extensions_limit)?;

    let imports_last_24h = db.recent_imports(1)?;
    let imports_last_7d = db.recent_imports(7)?;
    let imports_last_30d = db.recent_imports(30)?;

    Ok(StatusReport {
        vault_root: vault_root.to_path_buf(),
        db_path: vault_root.join(".svault").join("vault.db"),
        stats,
        top_extensions,
        imports_last_24h,
        imports_last_7d,
        imports_last_30d,
    })
}

/// Helper to convert renderable to string
fn render_to_string<R: Renderable>(renderable: &R) -> String {
    let console = Console::new();
    let options = console.options();
    let segments = renderable.render(&console, &options);

    segments
        .into_iter()
        .map(|seg| seg.text.into_owned())
        .collect::<Vec<_>>()
        .join("")
}

/// Renders a status report with rich terminal output.
pub fn render_human(report: &StatusReport) -> String {
    let mut output = String::new();

    // Header
    output.push_str("📦 Svault Vault Status\n");
    output.push_str(&format!("   {}\n", report.vault_root.display()));
    output.push_str(&format!("   {}\n\n", report.db_path.display()));

    // Files section
    let mut files_table = Table::new()
        .title("📊 Files")
        .title_justify(JustifyMethod::Left)
        .box_style(&CLEAN_STYLE)
        .min_width(40)
        .with_column(Column::new("Metric"))
        .with_column(Column::new("Value").justify(JustifyMethod::Right));

    files_table.add_row_cells(["Total Files", &format_count(report.stats.total_files)]);
    files_table.add_row_cells(["Total Size", &format_bytes(report.stats.total_size_bytes)]);
    files_table.add_row_cells(["Imported", &format_count(report.stats.imported_count)]);
    files_table.add_row_cells(["Duplicates", &format_count(report.stats.duplicate_count)]);

    output.push_str(&render_to_string(&files_table));
    output.push('\n');

    // Hash status section
    let mut hash_table = Table::new()
        .title("🔐 Hash Status")
        .title_justify(JustifyMethod::Left)
        .box_style(&CLEAN_STYLE)
        .min_width(40)
        .with_column(Column::new("Metric"))
        .with_column(Column::new("Value").justify(JustifyMethod::Right));

    hash_table.add_row_cells([
        "SHA-256 Computed",
        &format_count(report.stats.has_sha256_count),
    ]);
    hash_table.add_row_cells([
        "Pending SHA-256",
        &format_count(report.stats.pending_sha256_count),
    ]);

    output.push_str(&render_to_string(&hash_table));

    if report.stats.pending_sha256_count > 0 {
        // Dim italic gray hint (ANSI: \x1b[3m = italic, \x1b[90m = bright black/gray, \x1b[0m = reset)
        output.push_str("\n\x1b[3m\x1b[90m💡 Run `svault verify --background-hash` to compute pending hashes\x1b[0m\n\n");
    } else {
        output.push('\n');
    }

    // Recent imports section
    let mut import_table = Table::new()
        .title("📈 Recent Imports")
        .title_justify(JustifyMethod::Left)
        .box_style(&CLEAN_STYLE)
        .min_width(40)
        .with_column(Column::new("Period"))
        .with_column(Column::new("Count").justify(JustifyMethod::Right));

    import_table.add_row_cells(["Last 24 hours", &format_count(report.imports_last_24h)]);
    import_table.add_row_cells(["Last 7 days", &format_count(report.imports_last_7d)]);
    import_table.add_row_cells(["Last 30 days", &format_count(report.imports_last_30d)]);

    output.push_str(&render_to_string(&import_table));
    output.push('\n');

    // Event log section
    let mut event_table = Table::new()
        .title("📝 Event Log")
        .title_justify(JustifyMethod::Left)
        .box_style(&CLEAN_STYLE)
        .min_width(40)
        .with_column(Column::new("Metric"))
        .with_column(Column::new("Value").justify(JustifyMethod::Right));

    event_table.add_row_cells(["Total Events", &format_count(report.stats.total_events)]);
    event_table.add_row_cells(["Database Size", &format_bytes(report.stats.db_size_bytes)]);

    output.push_str(&render_to_string(&event_table));
    output.push('\n');

    // Top extensions section
    if !report.top_extensions.is_empty() {
        let mut ext_table = Table::new()
            .title("📁 Top File Types")
            .title_justify(JustifyMethod::Left)
            .box_style(&CLEAN_STYLE)
            .min_width(40)
            .with_column(Column::new("Type"))
            .with_column(Column::new("Files").justify(JustifyMethod::Right))
            .with_column(Column::new("Size").justify(JustifyMethod::Right));

        for e in &report.top_extensions {
            ext_table.add_row_cells([
                format!(".{}", e.extension).as_str(),
                &format_count(e.count),
                &format_bytes(e.total_size_bytes),
            ]);
        }

        output.push_str(&render_to_string(&ext_table));
        output.push('\n');
    }

    output
}

/// Renders a status report as JSON.
pub fn render_json(report: &StatusReport) -> anyhow::Result<String> {
    let json = serde_json::json!({
        "vault_root": report.vault_root,
        "db_path": report.db_path,
        "stats": {
            "total_files": report.stats.total_files,
            "total_size_bytes": report.stats.total_size_bytes,
            "total_size_human": format_bytes(report.stats.total_size_bytes),
            "imported_count": report.stats.imported_count,
            "duplicate_count": report.stats.duplicate_count,
            "has_sha256_count": report.stats.has_sha256_count,
            "pending_sha256_count": report.stats.pending_sha256_count,
            "total_events": report.stats.total_events,
            "db_size_bytes": report.stats.db_size_bytes,
            "db_size_human": format_bytes(report.stats.db_size_bytes),
        },
        "recent_imports": {
            "last_24h": report.imports_last_24h,
            "last_7d": report.imports_last_7d,
            "last_30d": report.imports_last_30d,
        },
        "top_extensions": report.top_extensions.iter().map(|e| {
            serde_json::json!({
                "extension": e.extension,
                "count": e.count,
                "size_bytes": e.total_size_bytes,
                "size_human": format_bytes(e.total_size_bytes),
            })
        }).collect::<Vec<_>>(),
    });

    Ok(serde_json::to_string_pretty(&json)?)
}
