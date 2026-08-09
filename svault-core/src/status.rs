//! Vault status report — pure data, no rendering.
//!
//! This is a **pull-model** query (see `docs/ARCHITECTURE.md` §2.2): core
//! returns a serializable [`StatusReport`]; formatting (terminal tables or
//! JSON) is the responsibility of the presentation layer (`svault-ui`).

use std::path::Path;

use serde::Serialize;

use crate::db::{Db, ExtensionStats, VaultStats};

/// Status report for a vault.
#[derive(Debug, Clone, Serialize)]
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
    /// Interrupted operation sessions (import/sync dirs without a manifest),
    /// with their staging residue. Empty when every session completed.
    pub incomplete_sessions: Vec<crate::session::IncompleteSession>,
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
    let incomplete_sessions = crate::session::find_incomplete_sessions(vault_root);

    Ok(StatusReport {
        vault_root: vault_root.to_path_buf(),
        db_path: vault_root.join(".svault").join("vault.db"),
        stats,
        top_extensions,
        imports_last_24h,
        imports_last_7d,
        imports_last_30d,
        incomplete_sessions,
    })
}
