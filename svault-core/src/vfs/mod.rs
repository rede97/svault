//! Virtual File System (VFS) abstraction layer.
//!
//! This module provides a unified interface for accessing different storage backends:
//! - Local filesystem (`system`)
//! - MTP devices (`mtp`) - USB-connected cameras, phones, etc.
//!
//! # Thread Safety and Concurrency
//!
//! All VFS backends implement `Send + Sync` and can be shared across threads.
//! However, the optimal concurrency strategy depends heavily on the backend type:
//!
//! ## Local Filesystem
//!
//! Local filesystems (ext4, xfs, APFS, NTFS) benefit significantly from parallel
//! operations. The `SystemFs` backend is designed for concurrent use:
//!
//! - **Parallel reads**: Excellent scaling with thread count (up to disk IOPS limit)
//! - **Parallel writes**: Good scaling for SSDs; HDDs may see diminishing returns
//! - **Reflink/hardlink**: Thread-safe and atomic
//!
//! ## MTP (Media Transfer Protocol)
//!
//! **⚠️ MTP is inherently single-stream and does NOT benefit from multi-threading.**
//!
//! MTP uses a single USB Bulk Transfer pipe with these characteristics:
//! - **Half-duplex protocol**: Only one operation (read OR write) at a time
//! - **USB bandwidth limit**: ~40-60 MB/s for USB 2.0, ~300-500 MB/s for USB 3.0
//! - **Device-side processing**: Camera/phone has limited CPU; concurrency adds overhead
//! - **Session locking**: Most devices serialize commands at the protocol level
//!
//! ### MTP Performance Best Practices
//!
//! ```ignore
//! // ❌ BAD: Parallel MTP reads (contention, no speedup)
//! let files = vec!["IMG_001.jpg", "IMG_002.jpg", ...];
//! files.par_iter().for_each(|f| {
//!     transfer_file(TransferStrategy::StreamCopy, mtp_backend, f, &local_fs, &dest); // Slow due to USB contention
//! });
//!
//! // ✅ GOOD: Sequential MTP reads with pipelining
//! for f in files {
//!     transfer_file(TransferStrategy::StreamCopy, mtp_backend, f, &local_fs, &dest); // Optimal: saturates USB pipe
//! }
//! ```
//!
//! The import pipeline in `vfs_import.rs` detects MTP sources and automatically
//! disables parallel copy for them. Local-to-local operations remain parallel.
//!
//! # Backend Selection
//!
//! Use `VfsManager` to open backends by URL:
//! - `file:///path` or `/path` → SystemFs
//! - `mtp://device_id/storage/path` → MtpFs (requires `mtp` feature)

pub mod system;
pub mod manager;
pub mod transfer;

#[cfg(feature = "mtp")]
pub mod mtp;

pub use manager::{VfsManager, VfsUrl, VfsSource, VfsProvider};
pub use transfer::transfer_file;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

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
    ///
    /// Note: hardlink is intentionally excluded from automatic selection
    /// because it shares the source inode, which is unsafe for an archive.
    pub fn best_strategy(&self, dst: &FsCapabilities) -> TransferStrategy {
        if self.reflink && dst.reflink {
            TransferStrategy::Reflink
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

    /// Returns true if this backend benefits from parallel operations.
    ///
    /// Default is `true` (local filesystems, network shares). Backends that
    /// use single-pipe protocols like MTP should return `false`.
    ///
    /// This guides the import pipeline to choose between `par_iter()` and `iter()`.
    ///
    /// # Examples
    ///
    /// - `SystemFs` → `true` (parallel helps for SSDs, multiple disks)
    /// - `MtpFs` → `false` (USB pipe is shared, device CPU is bottleneck)
    fn is_parallel_capable(&self) -> bool {
        true
    }

    /// Lists directory entries one level deep (non-recursive).
    fn list(&self, dir: &Path) -> VfsResult<Vec<DirEntry>>;

    /// Opens the file at `path` for reading and returns a boxed reader.
    fn open_read(&self, path: &Path) -> VfsResult<Box<dyn Read>>;

    /// Opens the file at `path` for writing and returns a boxed writer.
    ///
    /// Default implementation returns `Unsupported`.
    fn open_write(&self, path: &Path) -> VfsResult<Box<dyn Write>> {
        let _ = path;
        Err(VfsError::Unsupported("open_write not supported"))
    }

    /// Attempt to create a reflink clone from `src` on this backend to `dst`
    /// on `dst_backend`.
    ///
    /// Default implementation returns `Unsupported`.
    fn reflink_to(&self, _src: &Path, _dst_backend: &dyn VfsBackend, _dst: &Path) -> VfsResult<()> {
        Err(VfsError::Unsupported("reflink not supported"))
    }

    /// Attempt to create a hard link from `src` on this backend to `dst`
    /// on `dst_backend`.
    ///
    /// Default implementation returns `Unsupported`.
    fn hard_link_to(&self, _src: &Path, _dst_backend: &dyn VfsBackend, _dst: &Path) -> VfsResult<()> {
        Err(VfsError::Unsupported("hardlink not supported"))
    }

    /// Creates all missing parent directories for `path`.
    fn create_dir_all(&self, path: &Path) -> VfsResult<()>;

    /// Downcast helper used by the transfer engine to resolve absolute paths
    /// between two local filesystem backends.
    ///
    /// Default implementation returns `None`.
    fn as_system_fs(&self) -> Option<&system::SystemFs> {
        None
    }

    /// Stream directory entries via channel.
    ///
    /// This is the preferred API for directory traversal. It returns a receiver
    /// that yields entries as they are discovered, allowing for streaming
    /// processing without loading all entries into memory at once.
    ///
    /// # Arguments
    /// * `dir` - The directory to walk (relative to VFS root)
    /// * `extensions` - File extensions to include (empty = all files)
    ///
    /// # Returns
    /// A receiver that yields `DirEntry` results. The channel closes when
    /// scanning completes or an unrecoverable error occurs.
    ///
    /// # Implementation Notes
    /// - Local filesystem (SystemFs): Uses parallel directory traversal via
    ///   background thread, streaming entries immediately as discovered.
    /// - MTP (MtpFs): Must collect all entries first (single-threaded due to
    ///   USB pipe limitation), then stream via channel for API consistency.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let rx = backend.walk_stream(Path::new(""), &["jpg", "png"])?;
    /// for entry_result in rx {
    ///     match entry_result {
    ///         Ok(entry) => println!("Found: {}", entry.path.display()),
    ///         Err(e) => eprintln!("Error: {}", e),
    ///     }
    /// }
    /// ```
    fn walk_stream(
        &self,
        dir: &Path,
        extensions: &[&str],
    ) -> VfsResult<mpsc::Receiver<VfsResult<DirEntry>>>;
}
