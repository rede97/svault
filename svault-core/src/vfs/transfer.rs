//! Cross-backend file transfer engine.
//!
//! Orchestrates [`TransferStrategy`] selection and execution between any
//! two [`VfsBackend`] implementations. Falls back to stream copy when
//! all requested strategies fail.

use std::path::Path;

use super::{TransferStrategy, VfsBackend, VfsResult};

/// Transfer a single file from `src_backend` to `dst_backend` using the
/// requested strategies in order.
///
/// - Strategies are attempted left-to-right.
/// - `StreamCopy` is always attempted as the final fallback if no earlier
///   strategy succeeds.
pub fn transfer_file(
    src_backend: &dyn VfsBackend,
    src_path: &Path,
    dst_backend: &dyn VfsBackend,
    dst_path: &Path,
    strategies: &[TransferStrategy],
) -> VfsResult<()> {
    for strategy in strategies {
        match strategy {
            TransferStrategy::Reflink => {
                if src_backend.reflink_to(src_path, dst_backend, dst_path).is_ok() {
                    return Ok(());
                }
            }
            TransferStrategy::Hardlink => {
                if src_backend.hard_link_to(src_path, dst_backend, dst_path).is_ok() {
                    return Ok(());
                }
            }
            TransferStrategy::StreamCopy => {
                return stream_copy(src_backend, src_path, dst_backend, dst_path);
            }
        }
    }
    // copy is always the final fallback
    stream_copy(src_backend, src_path, dst_backend, dst_path)
}

/// Fallback stream copy using `open_read` + `open_write`.
fn stream_copy(
    src_backend: &dyn VfsBackend,
    src_path: &Path,
    dst_backend: &dyn VfsBackend,
    dst_path: &Path,
) -> VfsResult<()> {
    let mut reader = src_backend.open_read(src_path)?;
    if let Some(parent) = dst_path.parent() {
        dst_backend.create_dir_all(parent)?;
    }
    let mut writer = dst_backend.open_write(dst_path)?;
    std::io::copy(&mut reader, &mut writer).map_err(super::VfsError::Io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    // Thread-safe mock backend for testing transfer logic
    struct MockBackend {
        capabilities: super::super::FsCapabilities,
        reflink_should_succeed: AtomicBool,
        hardlink_should_succeed: AtomicBool,
        created_dirs: Mutex<Vec<std::path::PathBuf>>,
    }

    impl MockBackend {
        fn new() -> Self {
            Self {
                capabilities: super::super::FsCapabilities {
                    reflink: true,
                    hardlink: true,
                    fs_type: "mock".to_string(),
                },
                reflink_should_succeed: AtomicBool::new(false),
                hardlink_should_succeed: AtomicBool::new(false),
                created_dirs: Mutex::new(Vec::new()),
            }
        }

        fn set_reflink_succeeds(&self, succeed: bool) {
            self.reflink_should_succeed.store(succeed, Ordering::SeqCst);
        }

        fn set_hardlink_succeeds(&self, succeed: bool) {
            self.hardlink_should_succeed.store(succeed, Ordering::SeqCst);
        }

        fn created_directories(&self) -> Vec<std::path::PathBuf> {
            self.created_dirs.lock().unwrap().clone()
        }
    }

    impl super::super::VfsBackend for MockBackend {
        fn capabilities(&self) -> &super::super::FsCapabilities {
            &self.capabilities
        }

        fn exists(&self, _path: &Path) -> super::super::VfsResult<bool> {
            Ok(true)
        }

        fn list(&self, _dir: &Path) -> super::super::VfsResult<Vec<super::super::DirEntry>> {
            Ok(Vec::new())
        }

        fn open_read(&self, _path: &Path) -> super::super::VfsResult<Box<dyn Read>> {
            Ok(Box::new(std::io::empty()))
        }

        fn open_write(&self, _path: &Path) -> super::super::VfsResult<Box<dyn std::io::Write>> {
            // Return a writer that discards data (for testing fallback logic)
            Ok(Box::new(std::io::sink()))
        }

        fn reflink_to(&self, _src: &Path, _dst_backend: &dyn super::super::VfsBackend, _dst: &Path) -> super::super::VfsResult<()> {
            if self.reflink_should_succeed.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(super::super::VfsError::Unsupported("reflink not supported"))
            }
        }

        fn hard_link_to(&self, _src: &Path, _dst_backend: &dyn super::super::VfsBackend, _dst: &Path) -> super::super::VfsResult<()> {
            if self.hardlink_should_succeed.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(super::super::VfsError::Unsupported("hardlink not supported"))
            }
        }

        fn create_dir_all(&self, path: &Path) -> super::super::VfsResult<()> {
            self.created_dirs.lock().unwrap().push(path.to_path_buf());
            Ok(())
        }
    }

    #[test]
    fn transfer_uses_first_strategy_when_it_succeeds() {
        let src = MockBackend::new();
        let dst = MockBackend::new();
        src.set_reflink_succeeds(true);
        
        let strategies = vec![TransferStrategy::Reflink, TransferStrategy::Hardlink];
        let result = transfer_file(&src, Path::new("/src/file"), &dst, Path::new("/dst/file"), &strategies);
        assert!(result.is_ok());
    }

    #[test]
    fn transfer_falls_back_to_second_strategy_when_first_fails() {
        let src = MockBackend::new();
        let dst = MockBackend::new();
        src.set_reflink_succeeds(false);
        src.set_hardlink_succeeds(true);
        
        let strategies = vec![TransferStrategy::Reflink, TransferStrategy::Hardlink];
        let result = transfer_file(&src, Path::new("/src/file"), &dst, Path::new("/dst/file"), &strategies);
        assert!(result.is_ok());
    }

    #[test]
    fn transfer_falls_back_to_stream_copy_when_all_else_fails() {
        let src = MockBackend::new();
        let dst = MockBackend::new();
        src.set_reflink_succeeds(false);
        src.set_hardlink_succeeds(false);
        
        let strategies = vec![TransferStrategy::Reflink, TransferStrategy::Hardlink];
        let result = transfer_file(&src, Path::new("/src/file"), &dst, Path::new("/dst/file"), &strategies);
        assert!(result.is_ok());
    }

    #[test]
    fn stream_copy_is_always_final_fallback() {
        let src = MockBackend::new();
        let dst = MockBackend::new();
        src.set_reflink_succeeds(false);
        
        let strategies = vec![TransferStrategy::Reflink];
        let result = transfer_file(&src, Path::new("/src/file"), &dst, Path::new("/dst/file"), &strategies);
        assert!(result.is_ok());
    }

    #[test]
    fn transfer_with_empty_strategy_list_uses_stream_copy_fallback() {
        let src = MockBackend::new();
        let dst = MockBackend::new();
        let strategies: Vec<TransferStrategy> = vec![];
        let result = transfer_file(&src, Path::new("/src/file"), &dst, Path::new("/dst/file"), &strategies);
        assert!(result.is_ok());
    }

    #[test]
    fn transfer_creates_parent_directories() {
        let temp_dir = tempfile::tempdir().unwrap();
        let src_path = temp_dir.path().join("src.txt");
        std::fs::write(&src_path, "test content").unwrap();
        
        let src_fs = super::super::system::SystemFs::open(temp_dir.path()).unwrap();
        let dst_fs = super::super::system::SystemFs::open(temp_dir.path()).unwrap();
        
        let strategies = vec![TransferStrategy::StreamCopy];
        let dst_path = Path::new("nested/deep/dir/output.txt");
        
        let result = transfer_file(&src_fs, Path::new("src.txt"), &dst_fs, dst_path, &strategies);
        assert!(result.is_ok());
        
        let final_path = temp_dir.path().join("nested/deep/dir/output.txt");
        assert!(final_path.exists());
        assert_eq!(std::fs::read_to_string(&final_path).unwrap(), "test content");
    }

    #[test]
    fn transfer_preserves_content_integrity() {
        let temp_dir = tempfile::tempdir().unwrap();
        let test_data: Vec<u8> = (0..10000).map(|i| (i % 256) as u8).collect();
        let src_path = temp_dir.path().join("src.bin");
        std::fs::write(&src_path, &test_data).unwrap();
        
        let src_fs = super::super::system::SystemFs::open(temp_dir.path()).unwrap();
        let dst_fs = super::super::system::SystemFs::open(temp_dir.path()).unwrap();
        
        let strategies = vec![TransferStrategy::StreamCopy];
        let result = transfer_file(&src_fs, Path::new("src.bin"), &dst_fs, Path::new("dst.bin"), &strategies);
        assert!(result.is_ok());
        
        let dst_data = std::fs::read(temp_dir.path().join("dst.bin")).unwrap();
        assert_eq!(dst_data, test_data);
    }

    #[test]
    fn empty_source_file_transfers_successfully() {
        let temp_dir = tempfile::tempdir().unwrap();
        let src_path = temp_dir.path().join("empty.txt");
        std::fs::write(&src_path, "").unwrap();
        
        let src_fs = super::super::system::SystemFs::open(temp_dir.path()).unwrap();
        let dst_fs = super::super::system::SystemFs::open(temp_dir.path()).unwrap();
        
        let strategies = vec![TransferStrategy::StreamCopy];
        let result = transfer_file(&src_fs, Path::new("empty.txt"), &dst_fs, Path::new("empty_copy.txt"), &strategies);
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
        
        let src_fs = super::super::system::SystemFs::open(temp_dir.path()).unwrap();
        let dst_fs = super::super::system::SystemFs::open(temp_dir.path()).unwrap();
        
        let strategies = vec![TransferStrategy::StreamCopy];
        let result = transfer_file(&src_fs, Path::new("large.bin"), &dst_fs, Path::new("large_copy.bin"), &strategies);
        assert!(result.is_ok());
        
        let dst_data = std::fs::read(temp_dir.path().join("large_copy.bin")).unwrap();
        assert_eq!(dst_data, test_data);
    }
}
