//! Detect and upgrade hardlinked files to independent binary copies.

use std::fs;
use std::io;
use std::path::Path;

/// Returns true if the file at `path` has more than one hard link.
#[cfg(unix)]
pub fn is_hardlinked(path: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(path)?;
    Ok(meta.nlink() > 1)
}

#[cfg(windows)]
pub fn is_hardlinked(path: &Path) -> io::Result<bool> {
    use std::os::windows::fs::MetadataExt;
    let meta = fs::metadata(path)?;
    Ok(meta.number_of_links().unwrap_or(1) > 1)
}

#[cfg(not(any(unix, windows)))]
pub fn is_hardlinked(_path: &Path) -> io::Result<bool> {
    Ok(false)
}

/// Create an independent binary copy of `path` by breaking its hard link.
///
/// Strategy:
/// 1. Copy the file to a temp file in the same directory.
/// 2. Atomically rename the temp file over the original path.
///
/// This preserves the original path, permissions, and content while ensuring
/// the inode is no longer shared with the source file.
pub fn upgrade_to_binary_copy(path: &Path) -> io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "invalid file path")
    })?;

    // Use a hidden temp name in the same directory to guarantee atomic rename.
    let tmp = parent.join(format!(".svault_upgrade_tmp_{}", file_name.to_string_lossy()));

    // Stream copy to avoid any OS-level copy optimisation that might preserve a hard link.
    {
        let mut reader = fs::File::open(path)?;
        let mut writer = fs::File::create(&tmp)?;
        io::copy(&mut reader, &mut writer)?;
        // Sync to disk so the rename sees complete data.
        writer.sync_all()?;
    }

    // Preserve permissions from the original file.
    let perms = fs::metadata(path)?.permissions();
    fs::set_permissions(&tmp, perms)?;

    // Atomic replace.
    fs::rename(&tmp, path)?;

    Ok(())
}
