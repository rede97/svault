//! Vault context management for CLI commands.
//!
//! Provides unified vault discovery, locking, database connection,
//! and configuration loading for all commands.

use std::path::{Path, PathBuf};
use svault_core::config::Config;
use svault_core::db::Db;
use svault_core::lock::VaultLock;

/// Unified vault context for all commands.
///
/// Manages vault discovery, locking, database connection, and configuration.
/// Ensures consistent initialization and proper resource cleanup.
///
/// # Resource Lifecycle
///
/// - `VaultLock`: Released when context is dropped
/// - `Db`: Closed when context is dropped
///
/// # Example
///
/// ```rust,ignore
/// let ctx = VaultContext::open_cwd()?;
/// let report = status::generate_report(ctx.vault_root(), ctx.db(), opts)?;
/// ```
pub struct VaultContext {
    vault_root: PathBuf,
    db: Db,
    config: Config,
    _lock: VaultLock,
}

impl VaultContext {
    /// Open a vault context from a target path.
    ///
    /// # Arguments
    /// * `target` - Optional target directory (defaults to current dir)
    /// * `reference` - Reference path for vault discovery
    ///
    /// # Returns
    /// Initialized context with database connection and lock acquired.
    ///
    /// # Errors
    /// Returns error if vault not found, lock cannot be acquired,
    /// database cannot be opened, or config cannot be loaded.
    pub fn open(target: Option<PathBuf>, reference: &Path) -> anyhow::Result<Self> {
        let vault_root = crate::commands::find_vault_root(target, reference)?;
        let lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
        let db_path = vault_root.join(".svault").join("vault.db");
        let db = Db::open(&db_path)
            .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;
        let config = Config::load(&vault_root)?;

        Ok(Self {
            vault_root,
            db,
            config,
            _lock: lock,
        })
    }

    /// Open context from current directory.
    ///
    /// Convenience method for commands that don't specify a target.
    pub fn open_cwd() -> anyhow::Result<Self> {
        Self::open(None, &std::env::current_dir()?)
    }

    /// Open context with explicit vault root.
    ///
    /// Used when vault root is already known (e.g., after init).
    pub fn open_at(vault_root: PathBuf) -> anyhow::Result<Self> {
        let lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
        let db_path = vault_root.join(".svault").join("vault.db");
        let db = Db::open(&db_path)
            .map_err(|e| anyhow::anyhow!("cannot open vault db: {e}"))?;
        let config = Config::load(&vault_root)?;

        Ok(Self {
            vault_root,
            db,
            config,
            _lock: lock,
        })
    }

    /// Get database reference.
    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Get config reference.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Get vault root path.
    pub fn vault_root(&self) -> &Path {
        &self.vault_root
    }

    /// Get default hash algorithm from config.
    pub fn default_hash(&self) -> svault_core::config::HashAlgorithm {
        self.config.global.hash.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_vault_context_open_at() {
        let tmp = TempDir::new().unwrap();
        let vault_root = tmp.path().to_path_buf();

        // Create minimal vault structure
        std::fs::create_dir_all(vault_root.join(".svault")).unwrap();

        // Create a minimal config
        let config_content = r#"
[global]
hash = "xxh3_128"
"#;
        std::fs::write(vault_root.join("svault.toml"), config_content).unwrap();

        // Should fail because db doesn't exist yet
        let result = VaultContext::open_at(vault_root);
        assert!(result.is_err());
    }
}
