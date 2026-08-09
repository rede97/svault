//! Local filesystem primitives used by import/update pipelines.

use crate::event::{Event, EventSink};

use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};

#[cfg(not(target_os = "windows"))]
use std::io::Write;

/// File transfer strategies, ordered from most to least efficient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TransferStrategy {
    /// Copy-on-write clone (btrfs/xfs FICLONE, APFS clonefile, ReFS).
    Reflink,
    /// Hard link (same filesystem only).
    Hardlink,
    /// Streaming copy fallback.
    StreamCopy,
}

/// Filesystem capabilities for a specific root path.
#[derive(Debug, Clone, Default)]
pub struct FsCapabilities {
    pub reflink: bool,
    pub hardlink: bool,
    pub fs_type: String,
}

impl FsCapabilities {
    /// Select best automatic strategy for a source/destination pair.
    pub fn best_strategy(&self, dst: &FsCapabilities) -> TransferStrategy {
        if self.reflink && dst.reflink {
            TransferStrategy::Reflink
        } else {
            TransferStrategy::StreamCopy
        }
    }
}

/// Single file entry discovered during scanning.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub path: PathBuf,
    pub size: u64,
    pub mtime_ms: i64,
    pub is_dir: bool,
}

/// Errors from filesystem operations.
#[derive(Debug)]
pub enum FsError {
    NotFound(PathBuf),
    Unsupported(&'static str),
    Io(std::io::Error),
    Other(String),
}

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsError::NotFound(p) => write!(f, "not found: {}", p.display()),
            FsError::Unsupported(op) => write!(f, "operation not supported: {op}"),
            FsError::Io(e) => write!(f, "io error: {e}"),
            FsError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for FsError {}

impl From<std::io::Error> for FsError {
    fn from(e: std::io::Error) -> Self {
        FsError::Io(e)
    }
}

pub type FsResult<T> = Result<T, FsError>;

fn resolve_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn ensure_root_exists(root: &Path) -> FsResult<()> {
    if root.exists() {
        Ok(())
    } else {
        Err(FsError::NotFound(root.to_path_buf()))
    }
}

/// Stream directory entries from local filesystem rooted at `root`.
pub fn walk_stream(
    root: &Path,
    dir: &Path,
    extensions: &[&str],
) -> FsResult<mpsc::Receiver<FsResult<DirEntry>>> {
    ensure_root_exists(root)?;
    let full_root = resolve_path(root, dir);
    let exts: Vec<String> = extensions.iter().map(|e| e.to_ascii_lowercase()).collect();

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        walk_stream_recursive(&full_root, &full_root, &exts, &tx);
    });

    Ok(rx)
}

fn walk_stream_recursive(
    root: &Path,
    current: &Path,
    exts: &[String],
    tx: &mpsc::Sender<FsResult<DirEntry>>,
) {
    for entry_result in jwalk::WalkDir::new(current)
        .skip_hidden(false)
        .process_read_dir(|_depth, _path, _state, children| {
            children.iter_mut().for_each(|child_result| {
                if let Ok(child) = child_result
                    && child.file_name == std::ffi::OsStr::new(".svault")
                {
                    child.read_children_path = None;
                }
            });
        })
    {
        match entry_result {
            Ok(entry) => {
                if entry.file_type().is_dir() {
                    continue;
                }

                let abs_path = entry.path();
                let path = abs_path
                    .strip_prefix(root)
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|_| abs_path.to_path_buf());

                if !exts.is_empty() {
                    let ext_matches = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| exts.iter().any(|ext| ext.eq_ignore_ascii_case(e)))
                        .unwrap_or(false);
                    if !ext_matches {
                        continue;
                    }
                }

                match entry.metadata() {
                    Ok(meta) => {
                        let mtime_ms = meta
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0);
                        let dir_entry = DirEntry {
                            path,
                            size: meta.len(),
                            mtime_ms,
                            is_dir: false,
                        };
                        if tx.send(Ok(dir_entry)).is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(FsError::Io(std::io::Error::other(e))));
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(Err(FsError::Io(std::io::Error::other(e))));
            }
        }
    }
}

/// Transfer one file from `src_root/src_path` to `dst_root/dst_path`.
///
/// When `sink` is provided, emits [`Event::CopyStarted`] /
/// [`Event::CopyProgress`] (Windows only) / [`Event::CopyFinished`].
pub fn transfer_file(
    src_root: &Path,
    src_path: &Path,
    dst_root: &Path,
    dst_path: &Path,
    strategies: &[TransferStrategy],
    sink: Option<&dyn EventSink>,
) -> FsResult<()> {
    let src_full = resolve_path(src_root, src_path);
    let dst_full = resolve_path(dst_root, dst_path);
    let file_size = fs::metadata(&src_full).map_err(FsError::Io)?.len();

    if let Some(s) = sink {
        s.emit(&Event::CopyStarted {
            src: src_full.clone(),
            dst: dst_full.clone(),
            bytes: file_size,
        });
    }

    let result = try_transfer(src_root, src_path, dst_root, dst_path, strategies, sink);

    if let Some(s) = sink {
        let error = match &result {
            Ok(_) => None,
            Err(e) => Some(e.to_string()),
        };
        s.emit(&Event::CopyFinished {
            src: src_full,
            dst: dst_full,
            error,
        });
    }

    result
}

fn try_transfer(
    src_root: &Path,
    src_path: &Path,
    dst_root: &Path,
    dst_path: &Path,
    strategies: &[TransferStrategy],
    sink: Option<&dyn EventSink>,
) -> FsResult<()> {
    // Get file info for progress reporting
    let src_full = resolve_path(src_root, src_path);
    let file_size = fs::metadata(&src_full).map_err(FsError::Io)?.len();

    for strategy in strategies {
        match strategy {
            TransferStrategy::Reflink => {
                if reflink_to(src_root, src_path, dst_root, dst_path).is_ok() {
                    return Ok(());
                }
            }
            TransferStrategy::Hardlink => {
                if hard_link_to(src_root, src_path, dst_root, dst_path).is_ok() {
                    return Ok(());
                }
            }
            TransferStrategy::StreamCopy => {
                return stream_copy_with_progress(
                    src_root, src_path, dst_root, dst_path, sink, &src_full, file_size,
                );
            }
        }
    }

    // Fallback to stream copy
    stream_copy_with_progress(
        src_root, src_path, dst_root, dst_path, sink, &src_full, file_size,
    )
}

#[cfg(not(target_os = "windows"))]
fn open_write(root: &Path, path: &Path) -> FsResult<Box<dyn Write>> {
    let full = resolve_path(root, path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).map_err(FsError::Io)?;
    }
    let f = fs::File::create(&full).map_err(FsError::Io)?;
    Ok(Box::new(f))
}

fn reflink_to(src_root: &Path, src: &Path, dst_root: &Path, dst: &Path) -> FsResult<()> {
    let src_full = resolve_path(src_root, src);
    let dst_full = resolve_path(dst_root, dst);
    if let Some(parent) = dst_full.parent() {
        fs::create_dir_all(parent).map_err(FsError::Io)?;
    }
    if try_reflink(&src_full, &dst_full)? {
        Ok(())
    } else {
        Err(FsError::Io(std::io::Error::other(
            "reflink not supported by filesystem",
        )))
    }
}

fn hard_link_to(src_root: &Path, src: &Path, dst_root: &Path, dst: &Path) -> FsResult<()> {
    let src_full = resolve_path(src_root, src);
    let dst_full = resolve_path(dst_root, dst);
    if let Some(parent) = dst_full.parent() {
        fs::create_dir_all(parent).map_err(FsError::Io)?;
    }
    fs::hard_link(&src_full, &dst_full).map_err(FsError::Io)
}

/// Copy a file with optional progress reporting.
///
/// - Windows: Uses CopyFileExW for native progress callbacks
/// - Linux/macOS: Uses std::io::copy (no per-file progress)
fn stream_copy_with_progress(
    src_root: &Path,
    src_path: &Path,
    dst_root: &Path,
    dst_path: &Path,
    sink: Option<&dyn EventSink>,
    src_abs: &Path,
    total_size: u64,
) -> FsResult<()> {
    #[cfg(target_os = "windows")]
    {
        stream_copy_windows_with_progress(
            src_root, src_path, dst_root, dst_path, sink, src_abs, total_size,
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (sink, src_abs, total_size); // Unused on non-Windows
        stream_copy_unix(src_root, src_path, dst_root, dst_path)
    }
}

/// Unix/Linux/macOS: Standard library copy (no per-file progress).
#[cfg(not(target_os = "windows"))]
fn stream_copy_unix(
    src_root: &Path,
    src_path: &Path,
    dst_root: &Path,
    dst_path: &Path,
) -> FsResult<()> {
    let mut reader = fs::File::open(resolve_path(src_root, src_path)).map_err(FsError::Io)?;
    let mut writer = open_write(dst_root, dst_path)?;
    std::io::copy(&mut reader, &mut writer).map_err(FsError::Io)?;
    Ok(())
}

/// Windows: CopyFileExW with progress callback.
#[cfg(target_os = "windows")]
fn stream_copy_windows_with_progress(
    src_root: &Path,
    src_path: &Path,
    dst_root: &Path,
    dst_path: &Path,
    sink: Option<&dyn EventSink>,
    src_abs: &Path,
    total_size: u64,
) -> FsResult<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::CopyFileExW;

    let src_full = resolve_path(src_root, src_path);
    let dst_full = resolve_path(dst_root, dst_path);

    // Create parent directory if needed
    if let Some(parent) = dst_full.parent() {
        fs::create_dir_all(parent).map_err(FsError::Io)?;
    }

    // Convert paths to wide strings
    let src_wide: Vec<u16> = OsStr::new(&src_full).encode_wide().chain(Some(0)).collect();
    let dst_wide: Vec<u16> = OsStr::new(&dst_full).encode_wide().chain(Some(0)).collect();

    // If no sink, use simple copy
    if sink.is_none() {
        let success = unsafe {
            CopyFileExW(
                src_wide.as_ptr(),
                dst_wide.as_ptr(),
                None, // No progress callback
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };
        if success == 0 {
            return Err(FsError::Io(std::io::Error::last_os_error()));
        }
        return Ok(());
    }

    // Copy with progress callback
    struct ProgressContext<'a> {
        sink: &'a dyn EventSink,
        src_abs: &'a Path,
        last_reported: std::sync::atomic::AtomicU64,
    }

    unsafe extern "system" fn progress_callback(
        total_file_size: i64,
        total_bytes_transferred: i64,
        _stream_size: i64,
        _stream_bytes_transferred: i64,
        _dw_stream_number: u32,
        _dw_callback_reason: u32,
        _h_source_file: isize,
        _h_destination_file: isize,
        lp_data: *const std::ffi::c_void,
    ) -> u32 {
        let ctx = unsafe { &*(lp_data as *const ProgressContext) };
        let copied = total_bytes_transferred as u64;
        let total = total_file_size as u64;

        // Throttle progress reports to every 1% or 1MB
        let last = ctx.last_reported.load(std::sync::atomic::Ordering::Relaxed);
        let threshold = (total / 100).max(1024 * 1024);

        if copied >= last + threshold || copied == total {
            ctx.last_reported
                .store(copied, std::sync::atomic::Ordering::Relaxed);
            ctx.sink.emit(&Event::CopyProgress {
                src: ctx.src_abs.to_path_buf(),
                copied,
                total,
            });
        }

        0 // PROGRESS_CONTINUE
    }

    let ctx = ProgressContext {
        sink: sink.unwrap(),
        src_abs,
        last_reported: std::sync::atomic::AtomicU64::new(0),
    };

    let success = unsafe {
        CopyFileExW(
            src_wide.as_ptr(),
            dst_wide.as_ptr(),
            Some(progress_callback),
            &ctx as *const _ as *const _,
            std::ptr::null_mut(),
            0,
        )
    };

    if success == 0 {
        return Err(FsError::Io(std::io::Error::last_os_error()));
    }

    // Report final progress
    if let Some(s) = sink {
        s.emit(&Event::CopyProgress {
            src: src_abs.to_path_buf(),
            copied: total_size,
            total: total_size,
        });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Crash-durability primitives (import staging)
// ---------------------------------------------------------------------------

/// Flush a file's data and its parent directory entry to disk.
///
/// Called on a staged import file right after the transfer, so that the
/// subsequent hash read is guaranteed to match durable storage even if the
/// machine loses power before the final rename (F2 crash-consistency).
pub fn sync_file_and_dir(path: &Path) -> FsResult<()> {
    // Prefer a read+write handle: FlushFileBuffers on Windows requires write
    // access, while a read-only fd is sufficient for fsync on Unix (and the
    // file may be read-only, e.g. hardlinked from a read-only source).
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .or_else(|_| fs::File::open(path))
        .map_err(FsError::Io)?;
    file.sync_all().map_err(FsError::Io)?;
    sync_parent_dir(path)
}

/// Atomically move a staged file to its final destination.
///
/// Creates the destination's parent directories, renames (atomic within one
/// filesystem), then fsyncs the parent directory so the rename itself is
/// durable across a crash. The caller is responsible for having fsynced the
/// file data beforehand (see [`sync_file_and_dir`]).
pub fn atomic_commit(staged: &Path, dest: &Path) -> FsResult<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(FsError::Io)?;
    }
    fs::rename(staged, dest).map_err(FsError::Io)?;
    sync_parent_dir(dest)
}

#[cfg(unix)]
fn sync_parent_dir(path: &Path) -> FsResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        let dir = fs::File::open(parent).map_err(FsError::Io)?;
        dir.sync_all().map_err(FsError::Io)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_dir(_path: &Path) -> FsResult<()> {
    // Directory fsync is not supported on Windows; the file-level
    // FlushFileBuffers in `sync_file_and_dir` still applies.
    Ok(())
}

// ---------------------------------------------------------------------------
// Capability probing
// ---------------------------------------------------------------------------

/// Probe the filesystem capabilities for the mount point containing `path`.
pub fn capabilities_for(path: &Path) -> FsResult<FsCapabilities> {
    let fs_type = detect_fs_type(path);
    let reflink = probe_reflink_support(path, &fs_type);
    let hardlink = probe_hardlink_support(path);

    Ok(FsCapabilities {
        reflink,
        hardlink,
        fs_type,
    })
}

#[cfg(target_os = "linux")]
fn detect_fs_type(path: &Path) -> String {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let c_path = match CString::new(path.as_os_str().as_encoded_bytes()) {
        Ok(p) => p,
        Err(_) => return "unknown".to_string(),
    };
    let mut buf: MaybeUninit<libc::statfs> = MaybeUninit::uninit();
    let ret = unsafe { libc::statfs(c_path.as_ptr(), buf.as_mut_ptr()) };
    if ret != 0 {
        return "unknown".to_string();
    }
    let buf = unsafe { buf.assume_init() };
    match buf.f_type {
        0x9123683E => "btrfs".to_string(),
        0x58465342 => "xfs".to_string(),
        0xEF53 => "ext4".to_string(),
        0x6969 => "nfs".to_string(),
        0xFF534D42 | 0xFE534D42 => "smb".to_string(),
        0x4D44 => "vfat".to_string(),
        0x2011BAB0 => "exfat".to_string(),
        _ => format!("unknown(0x{:X})", buf.f_type),
    }
}

#[cfg(target_os = "macos")]
fn detect_fs_type(path: &Path) -> String {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let c_path = match CString::new(path.as_os_str().as_encoded_bytes()) {
        Ok(p) => p,
        Err(_) => return "unknown".to_string(),
    };
    let mut buf: MaybeUninit<libc::statfs> = MaybeUninit::uninit();
    let ret = unsafe { libc::statfs(c_path.as_ptr(), buf.as_mut_ptr()) };
    if ret != 0 {
        return "unknown".to_string();
    }
    let buf = unsafe { buf.assume_init() };
    let name = unsafe { std::ffi::CStr::from_ptr(buf.f_fstypename.as_ptr()) };
    name.to_string_lossy().to_lowercase()
}

#[cfg(target_os = "windows")]
fn detect_fs_type(_path: &Path) -> String {
    "ntfs".to_string()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn detect_fs_type(_path: &Path) -> String {
    "unknown".to_string()
}

fn probe_reflink_support(_path: &Path, fs_type: &str) -> bool {
    matches!(fs_type, "btrfs" | "xfs" | "apfs" | "refs")
}

fn probe_hardlink_support(path: &Path) -> bool {
    let tmp = path.join(".svault_probe_hl");
    let tmp2 = path.join(".svault_probe_hl2");
    if fs::write(&tmp, b"").is_err() {
        return false;
    }
    let supported = fs::hard_link(&tmp, &tmp2).is_ok();
    let _ = fs::remove_file(&tmp);
    let _ = fs::remove_file(&tmp2);
    supported
}

#[cfg(target_os = "linux")]
fn try_reflink(src: &Path, dst: &Path) -> FsResult<bool> {
    use std::os::unix::io::AsRawFd;
    let src_file = fs::File::open(src).map_err(FsError::Io)?;
    let dst_file = fs::File::create(dst).map_err(FsError::Io)?;
    const FICLONE: u64 = 0x40049409;
    let ret = unsafe { libc::ioctl(dst_file.as_raw_fd(), FICLONE, src_file.as_raw_fd()) };
    Ok(ret == 0)
}

#[cfg(target_os = "macos")]
fn try_reflink(src: &Path, dst: &Path) -> FsResult<bool> {
    use std::ffi::CString;
    let src_c = CString::new(src.as_os_str().as_encoded_bytes())
        .map_err(|e| FsError::Other(e.to_string()))?;
    let dst_c = CString::new(dst.as_os_str().as_encoded_bytes())
        .map_err(|e| FsError::Other(e.to_string()))?;
    let ret = unsafe { libc::clonefile(src_c.as_ptr(), dst_c.as_ptr(), 0) };
    Ok(ret == 0)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn try_reflink(_src: &Path, _dst: &Path) -> FsResult<bool> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_with_empty_strategy_list_uses_stream_copy_fallback() {
        let temp_dir = tempfile::tempdir().unwrap();
        let src_path = temp_dir.path().join("src.txt");
        std::fs::write(&src_path, "test").unwrap();

        let strategies: Vec<TransferStrategy> = vec![];
        let result = transfer_file(
            temp_dir.path(),
            Path::new("src.txt"),
            temp_dir.path(),
            Path::new("dst.txt"),
            &strategies,
            None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn transfer_creates_parent_directories() {
        let temp_dir = tempfile::tempdir().unwrap();
        let src_path = temp_dir.path().join("src.txt");
        std::fs::write(&src_path, "test content").unwrap();

        let strategies = vec![TransferStrategy::StreamCopy];
        let dst_path = Path::new("nested/deep/dir/output.txt");

        let result = transfer_file(
            temp_dir.path(),
            Path::new("src.txt"),
            temp_dir.path(),
            dst_path,
            &strategies,
            None,
        );
        assert!(result.is_ok());

        let final_path = temp_dir.path().join("nested/deep/dir/output.txt");
        assert!(final_path.exists());
        assert_eq!(
            std::fs::read_to_string(&final_path).unwrap(),
            "test content"
        );
    }

    #[test]
    fn transfer_preserves_content_integrity() {
        let temp_dir = tempfile::tempdir().unwrap();
        let test_data: Vec<u8> = (0..10000).map(|i| (i % 256) as u8).collect();
        let src_path = temp_dir.path().join("src.bin");
        std::fs::write(&src_path, &test_data).unwrap();

        let strategies = vec![TransferStrategy::StreamCopy];
        let result = transfer_file(
            temp_dir.path(),
            Path::new("src.bin"),
            temp_dir.path(),
            Path::new("dst.bin"),
            &strategies,
            None,
        );
        assert!(result.is_ok());

        let dst_data = std::fs::read(temp_dir.path().join("dst.bin")).unwrap();
        assert_eq!(dst_data, test_data);
    }

    #[test]
    fn empty_source_file_transfers_successfully() {
        let temp_dir = tempfile::tempdir().unwrap();
        let src_path = temp_dir.path().join("empty.txt");
        std::fs::write(&src_path, "").unwrap();

        let strategies = vec![TransferStrategy::StreamCopy];
        let result = transfer_file(
            temp_dir.path(),
            Path::new("empty.txt"),
            temp_dir.path(),
            Path::new("empty_copy.txt"),
            &strategies,
            None,
        );
        assert!(result.is_ok());

        let dst_path = temp_dir.path().join("empty_copy.txt");
        assert!(dst_path.exists());
        assert_eq!(std::fs::read(&dst_path).unwrap().len(), 0);
    }

    #[test]
    fn large_file_transfers_successfully() {
        let temp_dir = tempfile::tempdir().unwrap();
        let test_data = vec![0xABu8; 10 * 1024 * 1024];
        let src_path = temp_dir.path().join("large.bin");
        std::fs::write(&src_path, &test_data).unwrap();

        let strategies = vec![TransferStrategy::StreamCopy];
        let result = transfer_file(
            temp_dir.path(),
            Path::new("large.bin"),
            temp_dir.path(),
            Path::new("large_copy.bin"),
            &strategies,
            None,
        );
        assert!(result.is_ok());

        let dst_data = std::fs::read(temp_dir.path().join("large_copy.bin")).unwrap();
        assert_eq!(dst_data, test_data);
    }
}
