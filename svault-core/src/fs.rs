//! Local filesystem primitives used by import/update pipelines.

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};

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
pub enum VfsError {
    NotFound(PathBuf),
    Unsupported(&'static str),
    Io(std::io::Error),
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

fn resolve_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn ensure_root_exists(root: &Path) -> VfsResult<()> {
    if root.exists() {
        Ok(())
    } else {
        Err(VfsError::NotFound(root.to_path_buf()))
    }
}

/// Stream directory entries from local filesystem rooted at `root`.
pub fn walk_stream(
    root: &Path,
    dir: &Path,
    extensions: &[&str],
) -> VfsResult<mpsc::Receiver<VfsResult<DirEntry>>> {
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
    tx: &mpsc::Sender<VfsResult<DirEntry>>,
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
                        let _ = tx.send(Err(VfsError::Io(std::io::Error::other(e))));
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(Err(VfsError::Io(std::io::Error::other(e))));
            }
        }
    }
}

/// Transfer one file from `src_root/src_path` to `dst_root/dst_path`.
pub fn transfer_file(
    src_root: &Path,
    src_path: &Path,
    dst_root: &Path,
    dst_path: &Path,
    strategies: &[TransferStrategy],
) -> VfsResult<()> {
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
                return stream_copy(src_root, src_path, dst_root, dst_path);
            }
        }
    }
    stream_copy(src_root, src_path, dst_root, dst_path)
}

fn open_read(root: &Path, path: &Path) -> VfsResult<Box<dyn Read>> {
    let f = fs::File::open(resolve_path(root, path)).map_err(VfsError::Io)?;
    Ok(Box::new(f))
}

fn open_write(root: &Path, path: &Path) -> VfsResult<Box<dyn Write>> {
    let full = resolve_path(root, path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).map_err(VfsError::Io)?;
    }
    let f = fs::File::create(&full).map_err(VfsError::Io)?;
    Ok(Box::new(f))
}

fn reflink_to(src_root: &Path, src: &Path, dst_root: &Path, dst: &Path) -> VfsResult<()> {
    let src_full = resolve_path(src_root, src);
    let dst_full = resolve_path(dst_root, dst);
    if let Some(parent) = dst_full.parent() {
        fs::create_dir_all(parent).map_err(VfsError::Io)?;
    }
    if try_reflink(&src_full, &dst_full)? {
        Ok(())
    } else {
        Err(VfsError::Io(std::io::Error::other(
            "reflink not supported by filesystem",
        )))
    }
}

fn hard_link_to(src_root: &Path, src: &Path, dst_root: &Path, dst: &Path) -> VfsResult<()> {
    let src_full = resolve_path(src_root, src);
    let dst_full = resolve_path(dst_root, dst);
    if let Some(parent) = dst_full.parent() {
        fs::create_dir_all(parent).map_err(VfsError::Io)?;
    }
    fs::hard_link(&src_full, &dst_full).map_err(VfsError::Io)
}

fn stream_copy(
    src_root: &Path,
    src_path: &Path,
    dst_root: &Path,
    dst_path: &Path,
) -> VfsResult<()> {
    let mut reader = open_read(src_root, src_path)?;
    let mut writer = open_write(dst_root, dst_path)?;
    std::io::copy(&mut reader, &mut writer).map_err(VfsError::Io)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Capability probing
// ---------------------------------------------------------------------------

/// Probe the filesystem capabilities for the mount point containing `path`.
pub fn capabilities_for(path: &Path) -> VfsResult<FsCapabilities> {
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
fn try_reflink(src: &Path, dst: &Path) -> VfsResult<bool> {
    use std::os::unix::io::AsRawFd;
    let src_file = fs::File::open(src).map_err(VfsError::Io)?;
    let dst_file = fs::File::create(dst).map_err(VfsError::Io)?;
    const FICLONE: u64 = 0x40049409;
    let ret = unsafe { libc::ioctl(dst_file.as_raw_fd(), FICLONE, src_file.as_raw_fd()) };
    Ok(ret == 0)
}

#[cfg(target_os = "macos")]
fn try_reflink(src: &Path, dst: &Path) -> VfsResult<bool> {
    use std::ffi::CString;
    let src_c = CString::new(src.as_os_str().as_encoded_bytes())
        .map_err(|e| VfsError::Other(e.to_string()))?;
    let dst_c = CString::new(dst.as_os_str().as_encoded_bytes())
        .map_err(|e| VfsError::Other(e.to_string()))?;
    let ret = unsafe { libc::clonefile(src_c.as_ptr(), dst_c.as_ptr(), 0) };
    Ok(ret == 0)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn try_reflink(_src: &Path, _dst: &Path) -> VfsResult<bool> {
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
        );
        assert!(result.is_ok());

        let dst_data = std::fs::read(temp_dir.path().join("large_copy.bin")).unwrap();
        assert_eq!(dst_data, test_data);
    }
}
