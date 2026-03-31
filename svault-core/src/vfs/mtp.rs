//! MTP (Media Transfer Protocol) VFS backend for Android devices and cameras.
//!
//! This module provides a [`VfsBackend`] implementation for MTP devices using
//! the pure-Rust `mtp-rs` crate. MTP is the standard protocol used by Android
//! phones and many digital cameras for file transfer over USB.
//!
//! # URL Format
//!
//! MTP devices are accessed via URL-style paths:
//! - `mtp://device_name/DCIM/Camera` - Access by device name/index
//! - `mtp://SN:serial_number/DCIM` - Access by serial number
//!
//! # Example
//!
//! ```rust,no_run
//! use svault_core::vfs::manager::VfsManager;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let manager = VfsManager::new();
//!
//! // List available MTP devices
//! for source in manager.probe_all()? {
//!     println!("{}: {} ({})", source.id, source.name, source.device_type);
//! }
//!
//! // Access a device
//! let (backend, path) = manager.open_url("mtp://MyPhone/DCIM/Camera")?;
//! # Ok(())
//! # }
//! ```

use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use mtp_rs::{
    mtp::{MtpDevice, Storage},
    ptp::ObjectInfo,
    ObjectHandle, StorageId,
};

use super::{
    manager::{VfsProvider, VfsSource},
    DirEntry, FsCapabilities, TransferStrategy, VfsBackend, VfsError, VfsResult,
};

/// MTP VFS Provider for device discovery and management.
pub struct MtpProvider;

impl MtpProvider {
    /// Get storage names for a device.
    fn get_storage_names(serial: &str) -> Vec<String> {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build() 
        {
            Ok(rt) => rt,
            Err(_) => return vec!["Internal".to_string()],
        };

        let device = match runtime.block_on(MtpDevice::open_by_serial(serial)) {
            Ok(d) => d,
            Err(_) => return vec!["Internal".to_string()],
        };

        runtime
            .block_on(device.storages())
            .map(|s| s.iter().map(|st| st.info().description.clone()).collect())
            .unwrap_or_else(|_| vec!["Internal".to_string()])
    }
}

impl VfsProvider for MtpProvider {
    fn scheme(&self) -> &str {
        "mtp"
    }

    fn probe(&self) -> VfsResult<Vec<VfsSource>> {
        // list_devices is synchronous
        let devices = MtpDevice::list_devices()
            .map_err(|e| VfsError::Other(format!("Failed to list MTP devices: {e}")))?;

        let mut sources = Vec::new();
        for (idx, info) in devices.into_iter().enumerate() {
            let device_name = format!("{} {}", 
                info.manufacturer.as_deref().unwrap_or("Unknown"),
                info.product.as_deref().unwrap_or("Device")
            );
            let serial = info.serial_number.clone().unwrap_or_default();
            
            // Try to get storage names
            let storages = if !serial.is_empty() {
                Self::get_storage_names(&serial)
            } else {
                vec!["Internal".to_string()]
            };

            sources.push(VfsSource {
                id: format!("mtp://device{}", idx + 1),
                name: device_name.clone(),
                scheme: "mtp".to_string(),
                device_type: format!("{} {}", 
                    info.manufacturer.as_deref().unwrap_or("Unknown"),
                    info.product.as_deref().unwrap_or("Device")
                ),
                roots: storages.clone(),
                unique_id: serial.clone(),
            });

            // Also add entry with serial number for direct access
            if !serial.is_empty() {
                sources.push(VfsSource {
                    id: format!("mtp://SN:{}", serial),
                    name: device_name,
                    scheme: "mtp".to_string(),
                    device_type: format!("{} {}", 
                    info.manufacturer.as_deref().unwrap_or("Unknown"),
                    info.product.as_deref().unwrap_or("Device")
                ),
                    roots: storages,
                    unique_id: serial,
                });
            }
        }

        Ok(sources)
    }

    fn open(&self, source_id: &str) -> VfsResult<Box<dyn VfsBackend>> {
        // Parse source_id like "mtp://device1" or "mtp://SN:ABC123"
        let identifier = source_id
            .strip_prefix("mtp://")
            .ok_or_else(|| VfsError::Other(format!("Invalid MTP source ID: {}", source_id)))?;

        if identifier.starts_with("SN:") {
            let serial = &identifier[3..];
            Ok(Box::new(MtpFs::open_by_serial(serial)?))
        } else if identifier.starts_with("device") {
            // Open by index (device1, device2, etc.)
            let idx: usize = identifier[6..]
                .parse()
                .map_err(|_| VfsError::Other(format!("Invalid device index: {}", identifier)))?;
            
            let devices = MtpDevice::list_devices()
                .map_err(|e| VfsError::Other(format!("Failed to list devices: {e}")))?;

            let device_info = devices
                .into_iter()
                .nth(idx - 1)
                .ok_or_else(|| VfsError::Other(format!("Device index {} not found", idx)))?;

            let serial = device_info.serial_number.unwrap_or_default();
            if serial.is_empty() {
                return Err(VfsError::Other("Device has no serial number".to_string()));
            }
            Ok(Box::new(MtpFs::open_by_serial(&serial)?))
        } else {
            Err(VfsError::Other(format!("Unknown MTP identifier: {}", identifier)))
        }
    }

    fn open_path(&self, authority: &str, path: &Path) -> VfsResult<Box<dyn VfsBackend>> {
        // authority is the device name/index/serial, path is the path within device
        if authority.is_empty() {
            // No authority specified, open first available device
            return Ok(Box::new(MtpFs::open_first()?));
        }

        if authority.starts_with("SN:") {
            let serial = &authority[3..];
            Ok(Box::new(MtpFs::open_by_serial(serial)?))
        } else {
            // Try to match by name or open by index
            match MtpFs::find_device_by_name(authority) {
                Some(serial) => Ok(Box::new(MtpFs::open_by_serial(&serial)?)),
                None => {
                    // Try as index
                    if let Ok(idx) = authority.parse::<usize>() {
                        let devices = MtpDevice::list_devices()
                            .map_err(|e| VfsError::Other(format!("Failed to list devices: {e}")))?;

                        if let Some(info) = devices.into_iter().nth(idx.saturating_sub(1)) {
                            let serial = info.serial_number.unwrap_or_default();
                            if !serial.is_empty() {
                                return Ok(Box::new(MtpFs::open_by_serial(&serial)?));
                            }
                        }
                    }
                    Err(VfsError::Other(format!("MTP device not found: {}", authority)))
                }
            }
        }
    }
}

/// MTP filesystem backend.
pub struct MtpFs {
    device: Arc<Mutex<MtpDevice>>,
    caps: FsCapabilities,
    runtime: tokio::runtime::Runtime,
    storage_id: StorageId,
}

impl MtpFs {
    /// Open the first available MTP device.
    pub fn open_first() -> VfsResult<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| VfsError::Other(format!("Failed to create Tokio runtime: {e}")))?;

        let device = runtime
            .block_on(MtpDevice::open_first())
            .map_err(|e| VfsError::Other(format!("Failed to open MTP device: {e}")))?;

        let storages = runtime
            .block_on(device.storages())
            .map_err(|e| VfsError::Other(format!("Failed to get storages: {e}")))?;

        let storage = storages
            .into_iter()
            .next()
            .ok_or_else(|| VfsError::Other("No storage available on MTP device".to_string()))?;

        let storage_id = storage.id();

        let caps = FsCapabilities {
            reflink: false,
            hardlink: false,
            fs_type: "mtp".to_string(),
        };

        Ok(Self {
            device: Arc::new(Mutex::new(device)),
            caps,
            runtime,
            storage_id,
        })
    }

    /// Open a specific MTP device by serial number.
    pub fn open_by_serial(serial: &str) -> VfsResult<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| VfsError::Other(format!("Failed to create Tokio runtime: {e}")))?;

        let device = runtime
            .block_on(MtpDevice::open_by_serial(serial))
            .map_err(|e| VfsError::Other(format!("Failed to open MTP device: {e}")))?;

        let storages = runtime
            .block_on(device.storages())
            .map_err(|e| VfsError::Other(format!("Failed to get storages: {e}")))?;

        let storage = storages
            .into_iter()
            .next()
            .ok_or_else(|| VfsError::Other("No storage available on MTP device".to_string()))?;

        let storage_id = storage.id();

        let caps = FsCapabilities {
            reflink: false,
            hardlink: false,
            fs_type: "mtp".to_string(),
        };

        Ok(Self {
            device: Arc::new(Mutex::new(device)),
            caps,
            runtime,
            storage_id,
        })
    }

    /// Find device serial by name (manufacturer + model).
    fn find_device_by_name(name: &str) -> Option<String> {
        let devices = MtpDevice::list_devices().ok()?;

        for info in devices {
            let product = info.product.as_deref().unwrap_or("");
            let manufacturer = info.manufacturer.as_deref().unwrap_or("");
            let full_name = format!("{} {}", manufacturer, product);
            if full_name.eq_ignore_ascii_case(name) || product.eq_ignore_ascii_case(name) {
                return info.serial_number;
            }
        }
        None
    }

    /// Get the storage for operations.
    fn get_storage(&self) -> VfsResult<Storage> {
        let device = self.device.lock().map_err(|e| {
            VfsError::Other(format!("Failed to lock device: {e}"))
        })?;
        
        self.runtime
            .block_on(device.storage(self.storage_id))
            .map_err(|e| VfsError::Other(format!("Failed to get storage: {e}")))
    }

    /// Find an object by path, returning its handle and info.
    fn find_object(&self, path: &Path) -> VfsResult<Option<(ObjectHandle, ObjectInfo)>> {
        if path.as_os_str().is_empty() || path == Path::new("/") {
            return Ok(None);
        }

        let storage = self.get_storage()?;
        let components: Vec<_> = path.components().collect();
        
        let mut parent: Option<ObjectHandle> = None;
        
        for (i, component) in components.iter().enumerate() {
            let name = component.as_os_str().to_string_lossy();
            
            let objects = self.runtime
                .block_on(storage.list_objects(parent))
                .map_err(|e| VfsError::Other(format!("MTP list_objects error: {e}")))?;
            
            let mut found = None;
            for obj in objects {
                if obj.filename == name.as_ref() {
                    found = Some((obj.handle, obj));
                    break;
                }
            }
            
            match found {
                Some((handle, info)) => {
                    if i == components.len() - 1 {
                        return Ok(Some((handle, info)));
                    } else if Self::is_folder(&info) {
                        parent = Some(handle);
                    } else {
                        return Ok(None);
                    }
                }
                None => return Ok(None),
            }
        }
        
        Ok(None)
    }

    /// Check if ObjectInfo represents a folder.
    fn is_folder(info: &ObjectInfo) -> bool {
        // Association type 0x0001 (Folder) indicates a folder
        matches!(info.association_type, mtp_rs::ptp::AssociationType::GenericFolder)
    }
}

impl VfsBackend for MtpFs {
    fn capabilities(&self) -> &FsCapabilities {
        &self.caps
    }

    fn exists(&self, path: &Path) -> VfsResult<bool> {
        if path.as_os_str().is_empty() || path == Path::new("/") {
            return Ok(true);
        }
        self.find_object(path).map(|o| o.is_some())
    }

    fn list(&self, dir: &Path) -> VfsResult<Vec<DirEntry>> {
        let storage = self.get_storage()?;
        
        let parent = if dir.as_os_str().is_empty() || dir == Path::new("/") {
            None
        } else {
            match self.find_object(dir)? {
                Some((handle, info)) => {
                    if !Self::is_folder(&info) {
                        return Err(VfsError::Other(format!(
                            "Not a directory: {}", dir.display()
                        )));
                    }
                    Some(handle)
                }
                None => return Err(VfsError::NotFound(dir.to_path_buf())),
            }
        };

        let objects = self.runtime
            .block_on(storage.list_objects(parent))
            .map_err(|e| VfsError::Other(format!("MTP list_objects error: {e}")))?;

        let mut entries = Vec::new();
        for obj in objects {
            entries.push(DirEntry {
                path: dir.join(&obj.filename),
                size: obj.size,
                mtime_ms: obj
                    .modified
                    .map(|dt| {
                    // Convert DateTime to milliseconds since epoch (approximate)
                    use std::time::{SystemTime, Duration};
                    let days_since_epoch = (dt.year as u64 - 1970) * 365 + (dt.month as u64) * 30 + (dt.day as u64);
                    let secs = days_since_epoch * 86400 + (dt.hour as u64) * 3600 + (dt.minute as u64) * 60 + (dt.second as u64);
                    secs as i64 * 1000
                })
                    .unwrap_or(0),
                is_dir: Self::is_folder(&obj),
            });
        }

        Ok(entries)
    }

    fn open_read(&self, path: &Path) -> VfsResult<Box<dyn Read>> {
        let (handle, info) = match self.find_object(path)? {
            Some((h, i)) => (h, i),
            None => return Err(VfsError::NotFound(path.to_path_buf())),
        };

        if Self::is_folder(&info) {
            return Err(VfsError::Other(format!(
                "Is a directory: {}", path.display()
            )));
        }

        let storage = self.get_storage()?;
        let runtime_handle = self.runtime.handle().clone();
        
        let download = runtime_handle
            .block_on(storage.download_stream(handle))
            .map_err(|e| VfsError::Other(format!("MTP download error: {e}")))?;

        let reader = MtpObjectReader {
            download,
            runtime: runtime_handle,
            buffer: Vec::new(),
            position: 0,
        };

        Ok(Box::new(reader))
    }

    fn copy_to(
        &self,
        src: &Path,
        dest: &dyn VfsBackend,
        dst: &Path,
    ) -> VfsResult<TransferStrategy> {
        let _strategy = self.caps.best_strategy(dest.capabilities());
        let mut reader = self.open_read(src)?;
        
        let dst_full = std::path::PathBuf::from(dst);
        
        if let Some(parent) = dst_full.parent() {
            dest.create_dir_all(parent)?;
        }
        
        let mut file = std::fs::File::create(&dst_full)
            .map_err(VfsError::Io)?;
        
        std::io::copy(&mut reader, &mut file)
            .map_err(VfsError::Io)?;

        Ok(TransferStrategy::StreamCopy)
    }

    fn create_dir_all(&self, _path: &Path) -> VfsResult<()> {
        Err(VfsError::Unsupported(
            "MTP directory creation not yet implemented"
        ))
    }
}

/// Streaming reader for MTP file downloads.
struct MtpObjectReader {
    download: mtp_rs::mtp::FileDownload,
    runtime: tokio::runtime::Handle,
    buffer: Vec<u8>,
    position: usize,
}

impl Read for MtpObjectReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.position < self.buffer.len() {
            let available = self.buffer.len() - self.position;
            let to_read = buf.len().min(available);
            buf[..to_read].copy_from_slice(&self.buffer[self.position..self.position + to_read]);
            self.position += to_read;
            return Ok(to_read);
        }

        match self.runtime.block_on(self.download.next_chunk()) {
            Some(Ok(chunk)) => {
                if chunk.is_empty() {
                    return Ok(0);
                }
                
                let to_read = buf.len().min(chunk.len());
                buf[..to_read].copy_from_slice(&chunk[..to_read]);
                
                if to_read < chunk.len() {
                    self.buffer = chunk.into();
                    self.position = to_read;
                }
                
                Ok(to_read)
            }
            Some(Err(e)) => Err(io::Error::other(format!("MTP download error: {e}"))),
            None => Ok(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires physical MTP device"]
    fn test_list_devices() {
        let devices = MtpDevice::list_devices().unwrap();
        for (idx, info) in devices.iter().enumerate() {
            println!("{}: {} {} (SN: {:?})", 
                idx + 1, 
                info.manufacturer.as_deref().unwrap_or("Unknown"),
                info.product.as_deref().unwrap_or("Device"), 
                info.serial_number
            );
        }
    }

    #[test]
    #[ignore = "requires physical MTP device"]
    fn test_open_first() {
        let fs = MtpFs::open_first().unwrap();
        let caps = fs.capabilities();
        assert!(!caps.reflink);
        assert!(!caps.hardlink);
        assert_eq!(caps.fs_type, "mtp");
    }

    #[test]
    #[ignore = "requires physical MTP device"]
    fn test_list_root() {
        let fs = MtpFs::open_first().unwrap();
        let entries = fs.list(Path::new("/")).unwrap();
        for entry in entries {
            println!(
                "{} ({} bytes, dir={})",
                entry.path.display(),
                entry.size,
                entry.is_dir
            );
        }
    }
}
