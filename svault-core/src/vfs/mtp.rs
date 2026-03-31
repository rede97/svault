//! MTP (Media Transfer Protocol) VFS backend for Android devices and cameras.
//!
//! This module provides a [`VfsBackend`] implementation for MTP devices using
//! the pure-Rust `mtp-rs` crate. MTP is the standard protocol used by Android
//! phones and many digital cameras for file transfer over USB.
//!
//! # URL Format
//!
//! MTP devices are accessed via URL-style paths:
//! - `mtp://1/DCIM/Camera` - Device #1, first storage, path /DCIM/Camera
//! - `mtp://1/SD Card/Photos` - Device #1, "SD Card" storage
//! - `mtp://SN:serial/DCIM` - Access by serial number
//!
//! # Multi-Storage Support
//!
//! Android devices often have multiple storages:
//! - Internal shared storage (usually the default)
//! - SD card (removable)
//! - USB OTG (if connected)
//!
//! Use `svault mtp ls` to see available storages for each device.
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
//! let (backend, path) = manager.open_url("mtp://1/DCIM/Camera")?;
//! # Ok(())
//! # }
//! ```

use std::{
    collections::HashMap,
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
    fn get_storage_info(serial: &str) -> Vec<(String, u64, u64)> {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build() 
        {
            Ok(rt) => rt,
            Err(_) => return vec![],
        };

        let device = match runtime.block_on(MtpDevice::open_by_serial(serial)) {
            Ok(d) => d,
            Err(_) => return vec![],
        };

        let storages = match runtime.block_on(device.storages()) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        storages.iter().map(|s| {
            let info = s.info();
            (info.description.clone(), info.free_space_bytes, info.max_capacity)
        }).collect()
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
            let manufacturer = info.manufacturer.as_deref().unwrap_or("Unknown");
            let product = info.product.as_deref().unwrap_or("Device");
            let device_name = format!("{} {}", manufacturer, product);
            let serial = info.serial_number.clone().unwrap_or_default();
            
            // Try to get storage info
            let storages = if !serial.is_empty() {
                Self::get_storage_info(&serial)
            } else {
                vec![]
            };

            let root_names: Vec<String> = if storages.is_empty() {
                vec!["Internal Storage".to_string()]
            } else {
                storages.iter().map(|(name, _, _)| name.clone()).collect()
            };

            sources.push(VfsSource {
                id: format!("mtp://{}", idx + 1),
                name: device_name.clone(),
                scheme: "mtp".to_string(),
                device_type: format!("{} {}", manufacturer, product),
                roots: root_names.clone(),
                unique_id: serial.clone(),
            });

            // Also add entry with serial number for direct access
            if !serial.is_empty() {
                sources.push(VfsSource {
                    id: format!("mtp://SN:{}", serial),
                    name: device_name,
                    scheme: "mtp".to_string(),
                    device_type: format!("{} {}", manufacturer, product),
                    roots: root_names,
                    unique_id: serial,
                });
            }
        }

        Ok(sources)
    }

    fn open(&self, source_id: &str) -> VfsResult<Box<dyn VfsBackend>> {
        // Parse source_id like "mtp://1" or "mtp://SN:ABC123"
        let identifier = source_id
            .strip_prefix("mtp://")
            .ok_or_else(|| VfsError::Other(format!("Invalid MTP source ID: {}", source_id)))?;

        if identifier.starts_with("SN:") {
            let serial = &identifier[3..];
            Ok(Box::new(MtpFs::open_by_serial(serial)?))
        } else {
            // Try to parse as numeric index (1, 2, 3...)
            let idx: usize = identifier
                .parse()
                .map_err(|_| VfsError::Other(format!("Invalid device identifier: {}", identifier)))?;
            
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
///
/// Wraps an MTP device connection and provides synchronous VFS access.
/// Supports multiple storages (internal + SD card).
pub struct MtpFs {
    /// The underlying MTP device connection.
    device: Arc<Mutex<MtpDevice>>,
    /// Cached device capabilities (MTP doesn't support reflink/hardlink).
    caps: FsCapabilities,
    /// Tokio runtime for executing async operations.
    runtime: tokio::runtime::Runtime,
    /// All available storages on this device.
    storages: HashMap<String, StorageId>,
    /// Default storage (usually internal storage).
    default_storage: StorageId,
}

impl MtpFs {
    /// Open the first available MTP device with retry.
    pub fn open_first() -> VfsResult<Self> {
        Self::open_first_with_retry(3)
    }

    /// Open the first available MTP device with retry logic.
    fn open_first_with_retry(max_retries: u32) -> VfsResult<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| VfsError::Other(format!("Failed to create Tokio runtime: {e}")))?;

        let mut last_error = None;
        for attempt in 1..=max_retries {
            match runtime.block_on(MtpDevice::open_first()) {
                Ok(device) => {
                    return Self::finish_open(runtime, device);
                }
                Err(e) => {
                    let err_str = e.to_string();
                    last_error = Some(e);
                    
                    // Check if it's a busy/locked error
                    if err_str.contains("busy") || err_str.contains("locked") || err_str.contains("access") {
                        if attempt < max_retries {
                            eprintln!("MTP device busy (attempt {}/{}), waiting...", attempt, max_retries);
                            std::thread::sleep(std::time::Duration::from_millis(500));
                            continue;
                        }
                    }
                    break;
                }
            }
        }

        let err = last_error.unwrap();
        let err_str = err.to_string();
        
        // Provide helpful error messages for common issues
        let help_msg = if err_str.contains("busy") || err_str.contains("interface is busy") {
            format!(
                "Failed to open MTP device: interface is busy\n\n\
                This usually means another program is using the device:\n\
                - File manager (Nautilus/Thunar/Dolphin)\n\
                - gvfs-mtp service\n\
                - Another svault instance\n\n\
                Solutions:\n\
                1. Close the file manager completely\n\
                2. Run: killall gvfsd-mtp\n\
                3. Or unplug and reconnect the USB cable\n\n\
                Original error: {}",
                err
            )
        } else if err_str.contains("not found") || err_str.contains("No such device") {
            format!(
                "Failed to open MTP device: device not found\n\n\
                Make sure:\n\
                1. The device is connected via USB\n\
                2. The device is unlocked (screen on)\n\
                3. USB mode is set to 'File transfer' / 'MTP'\n\
                4. You have granted MTP permission on the device\n\n\
                Original error: {}",
                err
            )
        } else {
            format!("Failed to open MTP device: {}", err)
        };
        
        Err(VfsError::Other(help_msg))
    }

    /// Complete device initialization after successful open.
    fn finish_open(runtime: tokio::runtime::Runtime, device: MtpDevice) -> VfsResult<Self> {
        let mtp_storages = runtime
            .block_on(device.storages())
            .map_err(|e| VfsError::Other(format!("Failed to get storages: {e}")))?;

        if mtp_storages.is_empty() {
            return Err(VfsError::Other("No storage available on MTP device".to_string()));
        }

        // Build storage name -> ID mapping
        let mut storages = HashMap::new();
        let mut default_storage = None;

        for (idx, storage) in mtp_storages.iter().enumerate() {
            let info = storage.info();
            let name = info.description.clone();
            let id = storage.id();
            
            if idx == 0 {
                default_storage = Some(id);
            }
            storages.insert(name, id);
        }

        let caps = FsCapabilities {
            reflink: false,
            hardlink: false,
            fs_type: "mtp".to_string(),
        };

        Ok(Self {
            device: Arc::new(Mutex::new(device)),
            caps,
            runtime,
            storages,
            default_storage: default_storage.unwrap(),
        })
    }

    /// Open a specific MTP device by serial number with retry.
    pub fn open_by_serial(serial: &str) -> VfsResult<Self> {
        Self::open_by_serial_with_retry(serial, 3)
    }

    /// Open a specific MTP device by serial number with retry logic.
    fn open_by_serial_with_retry(serial: &str, max_retries: u32) -> VfsResult<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| VfsError::Other(format!("Failed to create Tokio runtime: {e}")))?;

        let mut last_error = None;
        for attempt in 1..=max_retries {
            match runtime.block_on(MtpDevice::open_by_serial(serial)) {
                Ok(device) => {
                    return Self::finish_open(runtime, device);
                }
                Err(e) => {
                    let err_str = e.to_string();
                    last_error = Some(e);
                    
                    if err_str.contains("busy") || err_str.contains("locked") || err_str.contains("access") {
                        if attempt < max_retries {
                            eprintln!("MTP device busy (attempt {}/{}), waiting...", attempt, max_retries);
                            std::thread::sleep(std::time::Duration::from_millis(500));
                            continue;
                        }
                    }
                    break;
                }
            }
        }

        let err = last_error.unwrap();
        let err_str = err.to_string();
        
        let help_msg = if err_str.contains("busy") || err_str.contains("interface is busy") {
            format!(
                "Failed to open MTP device: interface is busy\n\n\
                The device is in use by another program:\n\
                - File manager (Nautilus/Thunar/Dolphin)\n\
                - gvfs-mtp service\n\
                - Another svault instance\n\n\
                Try:\n\
                1. Close all file manager windows\n\
                2. Run: killall gvfsd-mtp\n\
                3. Unplug and reconnect USB\n\n\
                Original error: {}",
                err
            )
        } else {
            format!("Failed to open MTP device '{}': {}", serial, err)
        };
        
        Err(VfsError::Other(help_msg))
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

    /// Get storage ID from path. 
    /// Path format: [storage_name]/rest/of/path
    /// If first non-root component matches a storage name, use it; otherwise use default.
    fn resolve_storage(&self, path: &Path) -> (StorageId, PathBuf) {
        let components: Vec<_> = path.components().collect();
        
        if components.is_empty() {
            return (self.default_storage, PathBuf::new());
        }

        // Find first Normal component (skip RootDir, CurDir, etc.)
        for (i, component) in components.iter().enumerate() {
            if let std::path::Component::Normal(name) = component {
                let name_str = name.to_string_lossy();
                if let Some(&storage_id) = self.storages.get(name_str.as_ref()) {
                    // Build remaining path from components after the storage name
                    let remaining: PathBuf = components[i + 1..].iter().collect();
                    return (storage_id, remaining);
                }
                // First normal component is not a storage name, use default
                break;
            }
        }

        // Use default storage, return full path
        (self.default_storage, path.to_path_buf())
    }

    /// Get storage for operations.
    fn get_storage(&self, storage_id: StorageId) -> VfsResult<Storage> {
        let device = self.device.lock().map_err(|e| {
            VfsError::Other(format!("Failed to lock device: {e}"))
        })?;
        
        self.runtime
            .block_on(device.storage(storage_id))
            .map_err(|e| VfsError::Other(format!("Failed to get storage: {e}")))
    }

    /// List objects with fallback for devices that need ObjectHandle::ALL.
    /// Some cameras (like RICOH) return empty results when parent=None,
    /// but work correctly with parent=ObjectHandle::ALL.
    fn list_objects_with_fallback(&self, storage: &Storage, parent: Option<ObjectHandle>) -> VfsResult<Vec<ObjectInfo>> {
        if parent.is_none() {
            // Try with ObjectHandle::ALL first (0xFFFFFFFF)
            match self.runtime.block_on(storage.list_objects(Some(mtp_rs::ptp::ObjectHandle::ALL))) {
                Ok(objs) if !objs.is_empty() => return Ok(objs),
                Ok(_) => {}, // Empty result, fall through
                Err(_) => {}, // Error, fall through
            }
        }
        
        self.runtime
            .block_on(storage.list_objects(parent))
            .map_err(|e| VfsError::Other(format!("MTP list_objects error: {e}")))
    }

    /// Find an object by path, returning its handle and info.
    fn find_object(&self, storage_id: StorageId, path: &Path) -> VfsResult<Option<(ObjectHandle, ObjectInfo)>> {
        if path.as_os_str().is_empty() || path == Path::new("/") {
            return Ok(None);
        }

        let storage = self.get_storage(storage_id)?;
        let components: Vec<_> = path.components().collect();
        
        let mut parent: Option<ObjectHandle> = None;
        
        for (i, component) in components.iter().enumerate() {
            let name = component.as_os_str().to_string_lossy();
            
            // Use fallback method for root listing
            let objects = self.list_objects_with_fallback(&storage, parent)?;
            
            let mut found = None;
            for obj in &objects {
                if obj.filename == name.as_ref() {
                    found = Some((obj.handle, obj.clone()));
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

    /// Get list of available storage names.
    pub fn storage_names(&self) -> Vec<&str> {
        self.storages.keys().map(|s| s.as_str()).collect()
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
        let (storage_id, subpath) = self.resolve_storage(path);
        self.find_object(storage_id, &subpath).map(|o| o.is_some())
    }

    fn list(&self, dir: &Path) -> VfsResult<Vec<DirEntry>> {
        let (storage_id, subpath) = self.resolve_storage(dir);

        let storage = self.get_storage(storage_id)?;
        
        // Check if we're listing storages (root of device)
        // This happens when path is empty or "/", OR when subpath is empty
        // (meaning the path was just a storage name like "/SD")
        if dir.as_os_str().is_empty() || dir == Path::new("/") {
            // Return storages as "directories"
            let mut entries = Vec::new();
            for (name, _) in &self.storages {
                entries.push(DirEntry {
                    path: PathBuf::from(name),
                    size: 0,
                    mtime_ms: 0,
                    is_dir: true,
                });
            }
            return Ok(entries);
        }
        
        // If subpath is empty, we're listing the root of a specific storage
        let parent = if subpath.as_os_str().is_empty() || subpath == Path::new("/") {
            None  // Root of storage
        } else {
            match self.find_object(storage_id, &subpath)? {
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

        // Use fallback method for listing objects
        let objects = self.list_objects_with_fallback(&storage, parent)?;

        let mut entries = Vec::new();
        for obj in objects {
            entries.push(DirEntry {
                path: dir.join(&obj.filename),
                size: obj.size,
                mtime_ms: obj
                    .modified
                    .map(|dt| {
                        // Convert DateTime to milliseconds since epoch (approximate)
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
        let (storage_id, subpath) = self.resolve_storage(path);
        let (handle, info) = match self.find_object(storage_id, &subpath)? {
            Some((h, i)) => (h, i),
            None => return Err(VfsError::NotFound(path.to_path_buf())),
        };

        if Self::is_folder(&info) {
            return Err(VfsError::Other(format!(
                "Is a directory: {}", path.display()
            )));
        }

        let storage = self.get_storage(storage_id)?;
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
