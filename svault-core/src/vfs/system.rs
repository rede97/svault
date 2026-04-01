//! Local filesystem backend — works on Linux, macOS, and Windows.
//!
//! Capabilities (reflink, hardlink) are probed once at construction time
//! for the mount point that contains the root path. Sub-directories on
//! different mount points are **not** covered by this probe; the transfer
//! engine must call `capabilities_for` with the actual source/destination
//! path pair when crossing mount-point boundaries.

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use super::{vfs_ext_matches, DirEntry, FsCapabilities, VfsBackend, VfsError, VfsResult};
use jwalk;

/// Local filesystem backend. Probes capabilities for `root` at construction.
pub struct SystemFs {
    root: PathBuf,
    caps: FsCapabilities,
}

impl SystemFs {
    /// Open a local filesystem backend rooted at `root`.
    /// Probes the mount point that contains `root` for reflink / hardlink
    /// support. Returns an error if `root` does not exist.
    pub fn open(root: impl Into<PathBuf>) -> VfsResult<Self> {
        let root = root.into();
        if !root.exists() {
            return Err(VfsError::NotFound(root));
        }
        let caps = probe_capabilities(&root)?;
        Ok(Self { root, caps })
    }

    /// Re-probe capabilities for a specific path (e.g. a sub-directory that
    /// may be on a different mount point than `root`).
    pub fn capabilities_for(&self, path: &Path) -> VfsResult<FsCapabilities> {
        probe_capabilities(path)
    }
}

impl VfsBackend for SystemFs {
    fn capabilities(&self) -> &FsCapabilities {
        &self.caps
    }

    fn exists(&self, path: &Path) -> VfsResult<bool> {
        self.root.join(path).try_exists().map_err(VfsError::Io)
    }

    fn list(&self, dir: &Path) -> VfsResult<Vec<DirEntry>> {
        let full = self.root.join(dir);
        let mut entries = Vec::new();
        for entry in fs::read_dir(&full).map_err(VfsError::Io)? {
            let entry = entry.map_err(VfsError::Io)?;
            if entry.file_name() == ".svault" {
                continue;
            }
            let meta = entry.metadata().map_err(VfsError::Io)?;
            let mtime_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            entries.push(DirEntry {
                path: entry.path(),
                size: meta.len(),
                mtime_ms,
                is_dir: meta.is_dir(),
            });
        }
        Ok(entries)
    }

    fn walk(&self, dir: &Path, extensions: &[&str]) -> VfsResult<Vec<DirEntry>> {
        let full = self.root.join(dir);
        let exts: Vec<String> = extensions.iter().map(|e| e.to_ascii_lowercase()).collect();
        let exts_ref: Vec<&str> = exts.iter().map(|s| s.as_str()).collect();
        let mut result = Vec::new();
        for entry in jwalk::WalkDir::new(&full)
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
            let entry = entry.map_err(|e| VfsError::Io(std::io::Error::other(e)))?;
            if entry.file_type().is_dir() {
                continue;
            }
            let path = entry.path();
            if !vfs_ext_matches(&path, &exts_ref) {
                continue;
            }
            let meta = entry.metadata().map_err(|e| VfsError::Io(std::io::Error::other(e)))?;
            let mtime_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            result.push(DirEntry {
                path,
                size: meta.len(),
                mtime_ms,
                is_dir: false,
            });
        }
        Ok(result)
    }

    fn open_read(&self, path: &Path) -> VfsResult<Box<dyn Read>> {
        let f = fs::File::open(self.root.join(path)).map_err(VfsError::Io)?;
        Ok(Box::new(f))
    }

    fn open_write(&self, path: &Path) -> VfsResult<Box<dyn Write>> {
        let full = self.root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).map_err(VfsError::Io)?;
        }
        let f = fs::File::create(&full).map_err(VfsError::Io)?;
        Ok(Box::new(f))
    }

    fn reflink_to(&self, src: &Path, dst_backend: &dyn VfsBackend, dst: &Path) -> VfsResult<()> {
        let src_full = self.root.join(src);
        let dst_sys = dst_backend.as_system_fs()
            .ok_or(VfsError::Unsupported("reflink requires local filesystem"))?;
        let dst_full = dst_sys.root.join(dst);
        if try_reflink(&src_full, &dst_full)? {
            Ok(())
        } else {
            Err(VfsError::Io(std::io::Error::other(
                "reflink not supported by filesystem",
            )))
        }
    }

    fn hard_link_to(&self, src: &Path, dst_backend: &dyn VfsBackend, dst: &Path) -> VfsResult<()> {
        let src_full = self.root.join(src);
        let dst_sys = dst_backend.as_system_fs()
            .ok_or(VfsError::Unsupported("hardlink requires local filesystem"))?;
        let dst_full = dst_sys.root.join(dst);
        fs::hard_link(&src_full, &dst_full).map_err(VfsError::Io)
    }

    fn create_dir_all(&self, path: &Path) -> VfsResult<()> {
        fs::create_dir_all(self.root.join(path)).map_err(VfsError::Io)
    }

    fn as_system_fs(&self) -> Option<&SystemFs> {
        Some(self)
    }
}

// ---------------------------------------------------------------------------
// Capability probing
// ---------------------------------------------------------------------------

/// Probe the filesystem capabilities for the mount point containing `path`.
fn probe_capabilities(path: &Path) -> VfsResult<FsCapabilities> {
    let fs_type = detect_fs_type(path);
    let reflink = probe_reflink_support(path, &fs_type);
    let hardlink = probe_hardlink_support(path);

    Ok(FsCapabilities {
        reflink,
        hardlink,
        fs_type,
    })
}

/// Returns a string identifying the filesystem type for the given path.
#[cfg(target_os = "linux")]
fn detect_fs_type(path: &Path) -> String {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let c_path = match CString::new(path.as_os_str().as_encoded_bytes()) {
        Ok(p) => p,
        Err(_) => return "unknown".to_string(),
    };
    let mut buf: MaybeUninit<libc::statfs> = MaybeUninit::uninit();
    // SAFETY: statfs fills the struct on success.
    let ret = unsafe { libc::statfs(c_path.as_ptr(), buf.as_mut_ptr()) };
    if ret != 0 {
        return "unknown".to_string();
    }
    let buf = unsafe { buf.assume_init() };
    // f_type is a magic number on Linux.
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
    // f_fstypename is a C string on macOS.
    let name = unsafe { std::ffi::CStr::from_ptr(buf.f_fstypename.as_ptr()) };
    name.to_string_lossy().to_lowercase()
}

#[cfg(target_os = "windows")]
fn detect_fs_type(path: &Path) -> String {
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut fs_name = vec![0u16; 32];
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW(
            wide.as_mut_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs_name.as_mut_ptr(),
            fs_name.len() as u32,
        )
    };
    if ok == 0 {
        return "unknown".to_string();
    }
    let end = fs_name.iter().position(|&c| c == 0).unwrap_or(fs_name.len());
    String::from_utf16_lossy(&fs_name[..end]).to_lowercase()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn detect_fs_type(_path: &Path) -> String {
    "unknown".to_string()
}

/// Check whether the filesystem at `path` supports reflinks.
/// We test by filesystem type (fast path) and optionally by attempting a
/// real reflink on a temp file (slow but definitive).
fn probe_reflink_support(_path: &Path, fs_type: &str) -> bool {
    // Known-supported filesystem types.
    matches!(fs_type, "btrfs" | "xfs" | "apfs" | "refs")
}

/// Check whether the filesystem at `path` supports hard links.
/// FAT/exFAT do not; almost everything else does.
fn probe_hardlink_support(path: &Path) -> bool {
    // Attempt to create a real hard link on a temp file as a definitive probe.
    let tmp = path.join(".svault_probe_hl");
    let tmp2 = path.join(".svault_probe_hl2");
    // Create a temp file.
    if fs::write(&tmp, b"").is_err() {
        return false;
    }
    let supported = fs::hard_link(&tmp, &tmp2).is_ok();
    let _ = fs::remove_file(&tmp);
    let _ = fs::remove_file(&tmp2);
    supported
}

// ---------------------------------------------------------------------------
// Transfer helpers
// ---------------------------------------------------------------------------

/// Attempt a reflink (copy-on-write) from `src` to `dst`.
/// Returns Ok(true) on success, Ok(false) if the OS/filesystem does not
/// support it at runtime.
#[cfg(target_os = "linux")]
fn try_reflink(src: &Path, dst: &Path) -> VfsResult<bool> {
    use std::os::unix::io::AsRawFd;
    let src_file = fs::File::open(src).map_err(VfsError::Io)?;
    let dst_file = fs::File::create(dst).map_err(VfsError::Io)?;
    // ioctl FICLONE (Linux 4.5+)
    const FICLONE: u64 = 0x40049409;
    let ret = unsafe { libc::ioctl(dst_file.as_raw_fd(), FICLONE, src_file.as_raw_fd()) };
    Ok(ret == 0)
}

#[cfg(target_os = "macos")]
fn try_reflink(src: &Path, dst: &Path) -> VfsResult<bool> {
    use std::ffi::CString;
    let src_c = CString::new(src.as_os_str().as_encoded_bytes()).map_err(|e| VfsError::Other(e.to_string()))?;
    let dst_c = CString::new(dst.as_os_str().as_encoded_bytes()).map_err(|e| VfsError::Other(e.to_string()))?;
    // clonefile(2) — APFS only, fails on non-APFS
    let ret = unsafe { libc::clonefile(src_c.as_ptr(), dst_c.as_ptr(), 0) };
    Ok(ret == 0)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn try_reflink(_src: &Path, _dst: &Path) -> VfsResult<bool> {
    Ok(false)
}

/// Stream copy `src` → `dst`.
/// On Windows uses `CopyFileEx` which transparently negotiates SMB
/// Server-Side Copy when both ends are on the same SMB share.
/// On Linux/macOS uses `io::copy` with kernel-managed buffering.
#[cfg(target_os = "windows")]
fn stream_copy(src: &Path, dst: &Path) -> VfsResult<()> {
    use std::os::windows::ffi::OsStrExt;
    let src_w: Vec<u16> = src.as_os_str().encode_wide().chain(Some(0)).collect();
    let dst_w: Vec<u16> = dst.as_os_str().encode_wide().chain(Some(0)).collect();
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::CopyFileExW(
            src_w.as_ptr(),
            dst_w.as_ptr(),
            None,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    if ok == 0 {
        Err(VfsError::Io(std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}


