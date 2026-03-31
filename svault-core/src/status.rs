//! Vault status reporting with rich terminal output.

use std::path::Path;

use comfy_table::{Table, ContentArrangement};

use crate::db::{Db, VaultStats, ExtensionStats, format_bytes, format_count};

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
pub fn generate_report(vault_root: &Path, db: &Db, opts: StatusOptions) -> anyhow::Result<StatusReport> {
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

/// Configure table with clean style.
fn create_clean_table(headers: Vec<&str>) -> Table {
    let mut table = Table::new();
    table
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers)
        .set_width(80); // Max width, comfy-table will adapt to terminal
    table
}

/// Renders a status report with rich terminal output.
pub fn render_human(report: &StatusReport) -> String {
    let mut output = String::new();
    
    // Header
    output.push_str("📦 Svault Vault Status\n");
    output.push_str(&format!("   {}\n", report.vault_root.display()));
    output.push_str(&format!("   {}\n\n", report.db_path.display()));
    
    // Files section
    let mut files_table = create_clean_table(vec!["Metric", "Value"]);

    files_table.add_row(vec![
        "Total Files",
        &format_count(report.stats.total_files),
    ]);
    files_table.add_row(vec![
        "Total Size",
        &format_bytes(report.stats.total_size_bytes),
    ]);
    files_table.add_row(vec![
        "Imported",
        &format_count(report.stats.imported_count),
    ]);
    files_table.add_row(vec![
        "Duplicates",
        &format_count(report.stats.duplicate_count),
    ]);
    
    output.push_str("📊 Files\n");
    output.push_str(&format!("{}\n", files_table));
    
    // Hash status section
    let mut hash_table = create_clean_table(vec!["Metric", "Value"]);

    hash_table.add_row(vec![
        "SHA-256 Computed",
        &format_count(report.stats.has_sha256_count),
    ]);
    hash_table.add_row(vec![
        "Pending SHA-256",
        &format_count(report.stats.pending_sha256_count),
    ]);
    
    output.push_str("🔐 Hash Status\n");
    output.push_str(&format!("{}\n", hash_table));
    
    if report.stats.pending_sha256_count > 0 {
        output.push_str("   💡 Run `svault background-hash` to compute pending hashes\n\n");
    } else {
        output.push('\n');
    }
    
    // Recent imports section
    let mut import_table = create_clean_table(vec!["Period", "Count"]);

    import_table.add_row(vec!["Last 24 hours", &format_count(report.imports_last_24h)]);
    import_table.add_row(vec!["Last 7 days", &format_count(report.imports_last_7d)]);
    import_table.add_row(vec!["Last 30 days", &format_count(report.imports_last_30d)]);
    
    output.push_str("📈 Recent Imports\n");
    output.push_str(&format!("{}\n", import_table));
    
    // Event log section
    let mut event_table = create_clean_table(vec!["Metric", "Value"]);

    event_table.add_row(vec!["Total Events", &format_count(report.stats.total_events)]);
    event_table.add_row(vec!["Database Size", &format_bytes(report.stats.db_size_bytes)]);
    
    output.push_str("📝 Event Log\n");
    output.push_str(&format!("{}\n", event_table));
    
    // Top extensions section
    if !report.top_extensions.is_empty() {
        let mut ext_table = create_clean_table(vec!["Type", "Files", "Size"]);

        
        for e in &report.top_extensions {
            ext_table.add_row(vec![
                &format!(".{}", e.extension),
                &format_count(e.count),
                &format_bytes(e.total_size_bytes),
            ]);
        }
        
        output.push_str("📁 Top File Types\n");
        output.push_str(&format!("{}\n", ext_table));
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
