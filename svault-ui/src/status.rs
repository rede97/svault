//! Status report rendering (human tables and JSON).
//!
//! The data comes from [`svault_core::status::generate_report`] (pull model);
//! this module only formats it.

use rich_rust::r#box::BoxChars;
use rich_rust::prelude::*;
use rich_rust::renderables::Renderable;
use svault_core::db::{format_bytes, format_count};
use svault_core::status::StatusReport;

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

/// Helper to convert renderable to string.
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

fn table(title: &str, columns: &[&str]) -> Table {
    let mut t = Table::new()
        .title(title)
        .title_justify(JustifyMethod::Left)
        .box_style(&CLEAN_STYLE)
        .min_width(40);
    for (i, col) in columns.iter().enumerate() {
        let c = if i == 0 {
            Column::new(*col)
        } else {
            Column::new(*col).justify(JustifyMethod::Right)
        };
        t = t.with_column(c);
    }
    t
}

/// Render a status report as human-readable tables.
pub fn render_human(report: &StatusReport) -> String {
    let mut output = String::new();

    // Header
    output.push_str("📦 Svault Vault Status\n");
    output.push_str(&format!("   {}\n", report.vault_root.display()));
    output.push_str(&format!("   {}\n\n", report.db_path.display()));

    // Files section
    let mut files_table = table("📊 Files", &["Metric", "Value"]);
    files_table.add_row_cells(["Total Files", &format_count(report.stats.total_files)]);
    files_table.add_row_cells(["Total Size", &format_bytes(report.stats.total_size_bytes)]);
    files_table.add_row_cells(["Imported", &format_count(report.stats.imported_count)]);
    files_table.add_row_cells(["Duplicates", &format_count(report.stats.duplicate_count)]);
    output.push_str(&render_to_string(&files_table));
    output.push('\n');

    // Hash status section
    let mut hash_table = table("🔐 Hash Status", &["Metric", "Value"]);
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
        output.push_str("\n\x1b[3m\x1b[90m💡 Run `svault verify --background-hash` to compute pending hashes\x1b[0m\n\n");
    } else {
        output.push('\n');
    }

    // Recent imports section
    let mut import_table = table("📈 Recent Imports", &["Period", "Count"]);
    import_table.add_row_cells(["Last 24 hours", &format_count(report.imports_last_24h)]);
    import_table.add_row_cells(["Last 7 days", &format_count(report.imports_last_7d)]);
    import_table.add_row_cells(["Last 30 days", &format_count(report.imports_last_30d)]);
    output.push_str(&render_to_string(&import_table));
    output.push('\n');

    // Database section
    let mut db_table = table("📝 Database", &["Metric", "Value"]);
    db_table.add_row_cells(["Database Size", &format_bytes(report.stats.db_size_bytes)]);
    output.push_str(&render_to_string(&db_table));
    output.push('\n');

    // Interrupted sessions section (only when present)
    if !report.incomplete_sessions.is_empty() {
        let mut interrupted = table(
            "⚠️  Interrupted Sessions",
            &["Operation", "Session", "Leftover Files", "Size"],
        );
        for s in &report.incomplete_sessions {
            interrupted.add_row_cells([
                s.kind.to_string(),
                s.session_id.clone(),
                format_count(s.residue_files as i64),
                format_bytes(s.residue_bytes as i64),
            ]);
        }
        output.push_str(&render_to_string(&interrupted));
        for s in &report.incomplete_sessions {
            output.push_str(&format!("\n   ↳ {}", s.dir.display()));
        }
        output.push_str("\n\x1b[3m\x1b[90m💡 Review plan.json inside; the next import finishes pending renames. Delete the directories manually when done.\x1b[0m\n\n");
    }

    // Top extensions section
    if !report.top_extensions.is_empty() {
        let mut ext_table = table("📁 Top File Types", &["Type", "Files", "Size"]);
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

/// Render a status report as pretty JSON.
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
            "db_size_bytes": report.stats.db_size_bytes,
            "db_size_human": format_bytes(report.stats.db_size_bytes),
        },
        "recent_imports": {
            "last_24h": report.imports_last_24h,
            "last_7d": report.imports_last_7d,
            "last_30d": report.imports_last_30d,
        },
        "incomplete_sessions": report.incomplete_sessions.iter().map(|s| {
            serde_json::json!({
                "kind": s.kind.to_string(),
                "session_id": s.session_id,
                "dir": s.dir,
                "residue_files": s.residue_files,
                "residue_bytes": s.residue_bytes,
            })
        }).collect::<Vec<_>>(),
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
