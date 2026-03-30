pub mod system;

use std::path::{Path, PathBuf};

/// The transfer strategies available between two VFS backends, ordered from
/// most to least efficient. The transfer engine picks the best strategy that
/// both the source and destination support.
///
/// Note: Server-Side Copy (SMB `FSCTL_SRV_COPYCHUNK`) is intentionally absent.
/// On Windows, `CopyFileEx` negotiates it transparently with the OS; on Linux
/// and macOS the kernel CIFS driver does not expose it. There is no benefit in
/// modelling it as a distinct strategy at this layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TransferStrategy {
    /// Copy-on-write clone (btrfs/xfs FICLONE, APFS clonefile, ReFS).
    /// Zero data movement; almost instantaneous.
    Reflink,
    /// Hard link (same filesystem only). No data copied, shares inode.
    Hardlink,
    /// Streaming copy. On Windows uses `CopyFileEx` (which transparently
    /// negotiates SMB Server-Side Copy when applicable). On Linux/macOS
    /// uses `io::copy` with 4 MB chunks.
    StreamCopy,
}

/// Capabilities reported by a filesystem or storage backend for a specific
/// mount point / root path. Capabilities are probed lazily when a backend is
/// opened and cached for the lifetime of the connection.
#[derive(Debug, Clone, Default)]
pub struct FsCapabilities {
    /// Supports copy-on-write reflinks (btrfs, xfs, APFS, ReFS).
    pub reflink: bool,
    /// Supports hard links (most POSIX filesystems; not FAT/exFAT).
    pub hardlink: bool,
    /// Human-readable filesystem type string, e.g. "btrfs", "apfs", "ntfs".
    pub fs_type: String,
}

impl FsCapabilities {
    /// Returns the best transfer strategy this backend can offer as a source
    /// combined with what a destination backend can offer.
    pub fn best_strategy(&self, dst: &FsCapabilities) -> TransferStrategy {
        if self.reflink && dst.reflink {
            TransferStrategy::Reflink
        } else if self.hardlink && dst.hardlink {
            TransferStrategy::Hardlink
        } else {
            TransferStrategy::StreamCopy
        }
    }
}

/// A single file entry returned by [`VfsBackend::list`].
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub path: PathBuf,
    pub size: u64,
    /// Last-modified time as Unix milliseconds.
    pub mtime_ms: i64,
    pub is_dir: bool,
}

/// Errors that can occur during VFS operations.
#[derive(Debug)]
pub enum VfsError {
    /// The path does not exist on this backend.
    NotFound(PathBuf),
    /// The backend does not support this operation.
    Unsupported(&'static str),
    /// An IO error from the underlying OS or library.
    Io(std::io::Error),
    /// Any other backend-specific error.
    Other(String),
}

impl std::fmt::Display for VfsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VfsError::NotFound(p) => write!(f, "not found: {}", p.display()),
            VfsError::Unsupported(op) => write!(f, "operation not supported: {op}"),
            VfsError::Io(e) => write!(f, "io error: {e}"),
            VfsError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for VfsError {}

impl From<std::io::Error> for VfsError {
    fn from(e: std::io::Error) -> Self {
        VfsError::Io(e)
    }
}

pub type VfsResult<T> = Result<T, VfsError>;

/// Returns true if `path`'s extension (lowercased) is in `extensions`.
/// Returns true unconditionally when `extensions` is empty.
pub(crate) fn vfs_ext_matches(path: &Path, extensions: &[&str]) -> bool {
    if extensions.is_empty() {
        return true;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            extensions.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

/// Abstraction over a storage backend (local filesystem, MTP device, etc.).
///
/// Implementations probe their capabilities on construction and expose them
/// via [`VfsBackend::capabilities`]. The transfer engine uses those to select
/// the best [`TransferStrategy`] without hardcoding backend-specific logic.
pub trait VfsBackend: Send + Sync {
    /// Returns the capabilities of this backend for the given root path.
    /// The result is expected to be cached — do not re-probe on every call.
    fn capabilities(&self) -> &FsCapabilities;

    /// Returns true if the path exists on this backend.
    fn exists(&self, path: &Path) -> VfsResult<bool>;

    /// Lists directory entries one level deep (non-recursive).
    fn list(&self, dir: &Path) -> VfsResult<Vec<DirEntry>>;

    /// Recursively walks `dir`, returning all files whose extension
    /// (lowercase, no leading dot) is in `extensions`.
    /// If `extensions` is empty, all files are returned.
    /// Directories and symlinks are never included in the result.
    ///
    /// The default implementation recurses via [`VfsBackend::list`].
    /// Backends may override this with a more efficient native walk.
    fn walk(&self, dir: &Path, extensions: &[&str]) -> VfsResult<Vec<DirEntry>> {
        let mut result = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            for entry in self.list(&current)? {
                if entry.is_dir {
                    stack.push(entry.path.clone());
                } else if vfs_ext_matches(&entry.path, extensions) {
                    result.push(entry);
                }
            }
        }
        Ok(result)
    }

    /// Opens the file at `path` for reading and returns a boxed reader.
    fn open_read(&self, path: &Path) -> VfsResult<Box<dyn std::io::Read>>;

    /// Copies `src` on this backend to `dst` on `dest` backend using the
    /// best available strategy. Falls back to stream copy if nothing better
    /// is available.
    fn copy_to(
        &self,
        src: &Path,
        dest: &dyn VfsBackend,
        dst: &Path,
    ) -> VfsResult<TransferStrategy>;

    /// Creates all missing parent directories for `path`.
    fn create_dir_all(&self, path: &Path) -> VfsResult<()>;
}
