//! Vault-level process locking.
//!
//! Prevents multiple svault processes from modifying the same vault
//! concurrently, which could lead to database corruption or duplicate imports.

use std::fs::OpenOptions;
use std::path::Path;

use fs2::FileExt;

/// An advisory lock on a vault.
pub struct VaultLock {
    _file: std::fs::File,
}

/// Attempts to acquire an exclusive lock on the given vault.
///
/// Returns `Err` if another svault process already holds the lock.
pub fn acquire_vault_lock(vault_root: &Path) -> anyhow::Result<VaultLock> {
    let lock_path = vault_root.join(".svault").join("lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| anyhow::anyhow!("cannot open lock file {}: {e}", lock_path.display()))?;

    file.try_lock_exclusive()
        .map_err(|_| anyhow::anyhow!("another svault process is already running on this vault"))?;

    Ok(VaultLock { _file: file })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lock_acquire_and_release() {
        let temp = tempfile::tempdir().unwrap();
        let vault = temp.path();
        std::fs::create_dir_all(vault.join(".svault")).unwrap();

        let lock = acquire_vault_lock(vault);
        assert!(lock.is_ok());

        // Should fail while first lock is held
        let second = acquire_vault_lock(vault);
        assert!(second.is_err());

        drop(lock);

        // Should succeed after first lock is dropped
        let third = acquire_vault_lock(vault);
        assert!(third.is_ok());
    }
}
