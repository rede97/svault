//! Vault status reporting.

use std::path::Path;

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

/// Renders a status report as human-readable text.
pub fn render_human(report: &StatusReport) -> String {
    let mut output = String::new();

    // Header
    output.push_str(&format!("📦 Svault Status\n"));
    output.push_str(&format!("   Path: {}\n", report.vault_root.display()));
    output.push_str(&format!("   DB:   {}\n", report.db_path.display()));
    output.push('\n');

    // Files overview
    output.push_str(&format!("📁 Files\n"));
    output.push_str(&format!("   Total:    {} files\n", format_count(report.stats.total_files)));
    output.push_str(&format!("   Size:     {}\n", format_bytes(report.stats.total_size_bytes)));
    output.push_str(&format!("   Imported: {}\n", format_count(report.stats.imported_count)));
    if report.stats.duplicate_count > 0 {
        output.push_str(&format!("   Dups:     {} (excluded from storage)\n", 
            format_count(report.stats.duplicate_count)));
    }
    output.push('\n');

    // Hash status
    output.push_str(&format!("🔐 Hash Status\n"));
    output.push_str(&format!("   SHA-256:    {} files\n", format_count(report.stats.has_sha256_count)));
    if report.stats.pending_sha256_count > 0 {
        output.push_str(&format!("   Pending:    {} files (run `svault background-hash`)\n", 
            format_count(report.stats.pending_sha256_count)));
    }
    output.push('\n');

    // Recent activity
    output.push_str(&format!("📈 Recent Imports\n"));
    output.push_str(&format!("   Last 24h:  {}\n", format_count(report.imports_last_24h)));
    output.push_str(&format!("   Last 7d:   {}\n", format_count(report.imports_last_7d)));
    output.push_str(&format!("   Last 30d:  {}\n", format_count(report.imports_last_30d)));
    output.push('\n');

    // Event log
    output.push_str(&format!("📝 Event Log\n"));
    output.push_str(&format!("   Events:    {}\n", format_count(report.stats.total_events)));
    output.push_str(&format!("   DB Size:   {}\n", format_bytes(report.stats.db_size_bytes)));
    output.push('\n');

    // Top extensions
    if !report.top_extensions.is_empty() {
        output.push_str(&format!("📊 Top File Types\n"));
        for ext in &report.top_extensions {
            output.push_str(&format!("   {:<8} {:>8} files  {}\n", 
                ext.extension,
                format_count(ext.count),
                format_bytes(ext.total_size_bytes)
            ));
        }
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
