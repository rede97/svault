//! Vault context management for all vault operations.
//!
//! Provides unified vault discovery, locking, database connection,
//! and configuration loading for both CLI and programmatic use.

use std::path::{Path, PathBuf};
use crate::config::Config;
use crate::db::Db;
use crate::lock::VaultLock;

/// Walk up from `start` looking for `.svault/vault.db`.
pub fn find_vault_root(target: Option<PathBuf>, source: &Path) -> anyhow::Result<PathBuf> {
    let start = target
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| source.to_path_buf());
    let mut cur: &Path = &start;
    loop {
        if cur.join(".svault").join("vault.db").exists() {
            return Ok(cur.to_path_buf());
        }
        match cur.parent() {
            Some(p) => cur = p,
            None => anyhow::bail!(
                "no vault found (no .svault/vault.db in {} or any parent). \
                 Run `svault init` first.",
                start.display()
            ),
        }
    }
}

/// Unified vault context for all operations.
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
        let vault_root = find_vault_root(target, reference)?;
        Self::open_at(vault_root)
    }

    /// Open context from current directory.
    ///
    /// Convenience method for operations that don't specify a target.
    pub fn open_cwd() -> anyhow::Result<Self> {
        let cwd = std::env::current_dir()?;
        Self::open_at(find_vault_root(None, &cwd)?)
    }

    /// Open context with explicit vault root.
    ///
    /// Used when vault root is already known (e.g., after init).
    pub fn open_at(vault_root: PathBuf) -> anyhow::Result<Self> {
        let lock = crate::lock::acquire_vault_lock(&vault_root)?;
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
    pub fn default_hash(&self) -> crate::config::HashAlgorithm {
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

    #[test]
    fn test_find_vault_root_not_found() {
        let tmp = TempDir::new().unwrap();
        let result = find_vault_root(Some(tmp.path().to_path_buf()), tmp.path());
        assert!(result.is_err());
    }
}
