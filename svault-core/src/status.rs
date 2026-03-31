//! Vault status reporting with rich formatted output.

use std::path::Path;

use tabled::{Table, Tabled, settings::{Style, Alignment, Modify, object::Rows}};

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

#[derive(Tabled)]
struct StatRow {
    metric: String,
    value: String,
}

#[derive(Tabled)]
struct ExtRow {
    extension: String,
    files: String,
    size: String,
}

/// Renders a status report as human-readable text with rich formatting.
pub fn render_human(report: &StatusReport) -> String {
    let mut output = String::new();
    
    // Header box
    output.push_str("╔══════════════════════════════════════════════════════════════════════════════╗\n");
    output.push_str("║                         📦 Svault Vault Status                                 ║\n");
    output.push_str("╚══════════════════════════════════════════════════════════════════════════════╝\n\n");
    
    // Vault info
    output.push_str(&format!("📁 Vault: {}\n", report.vault_root.display()));
    output.push_str(&format!("🗄️  Database: {}\n\n", report.db_path.display()));
    
    // Files section
    let file_stats = vec![
        StatRow { metric: "Total Files".to_string(), value: format_count(report.stats.total_files) },
        StatRow { metric: "Total Size".to_string(), value: format_bytes(report.stats.total_size_bytes) },
        StatRow { metric: "Imported".to_string(), value: format_count(report.stats.imported_count) },
        StatRow { metric: "Duplicates".to_string(), value: format_count(report.stats.duplicate_count) },
    ];
    
    let mut files_table = Table::new(&file_stats);
    files_table.with(Style::modern_rounded());
    files_table.with(Modify::new(Rows::first()).with(Alignment::center()));
    
    output.push_str("📊 Files\n");
    output.push_str(&format!("{}\n", files_table));
    
    // Hash status section
    let hash_stats = vec![
        StatRow { metric: "SHA-256 Computed".to_string(), value: format_count(report.stats.has_sha256_count) },
        StatRow { metric: "Pending SHA-256".to_string(), value: format_count(report.stats.pending_sha256_count) },
    ];
    
    let mut hash_table = Table::new(&hash_stats);
    hash_table.with(Style::modern_rounded());
    hash_table.with(Modify::new(Rows::first()).with(Alignment::center()));
    
    output.push_str("🔐 Hash Status\n");
    output.push_str(&format!("{}\n", hash_table));
    
    if report.stats.pending_sha256_count > 0 {
        output.push_str("   💡 Run `svault background-hash` to compute pending hashes\n\n");
    } else {
        output.push('\n');
    }
    
    // Recent imports section
    let import_stats = vec![
        StatRow { metric: "Last 24 hours".to_string(), value: format_count(report.imports_last_24h) },
        StatRow { metric: "Last 7 days".to_string(), value: format_count(report.imports_last_7d) },
        StatRow { metric: "Last 30 days".to_string(), value: format_count(report.imports_last_30d) },
    ];
    
    let mut import_table = Table::new(&import_stats);
    import_table.with(Style::modern_rounded());
    import_table.with(Modify::new(Rows::first()).with(Alignment::center()));
    
    output.push_str("📈 Recent Imports\n");
    output.push_str(&format!("{}\n", import_table));
    
    // Event log section
    let event_stats = vec![
        StatRow { metric: "Total Events".to_string(), value: format_count(report.stats.total_events) },
        StatRow { metric: "Database Size".to_string(), value: format_bytes(report.stats.db_size_bytes) },
    ];
    
    let mut event_table = Table::new(&event_stats);
    event_table.with(Style::modern_rounded());
    event_table.with(Modify::new(Rows::first()).with(Alignment::center()));
    
    output.push_str("📝 Event Log\n");
    output.push_str(&format!("{}\n", event_table));
    
    // Top extensions section
    if !report.top_extensions.is_empty() {
        let ext_rows: Vec<ExtRow> = report.top_extensions.iter().map(|e| {
            ExtRow {
                extension: format!(".{}", e.extension),
                files: format_count(e.count),
                size: format_bytes(e.total_size_bytes),
            }
        }).collect();
        
        let mut ext_table = Table::new(&ext_rows);
        ext_table.with(Style::modern_rounded());
        ext_table.with(Modify::new(Rows::first()).with(Alignment::center()));
        
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
