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
