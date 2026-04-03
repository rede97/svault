//! VFS Manager - unified discovery and access for all storage backends.
//!
//! This module provides a unified interface for discovering and accessing
//! different storage backends (local filesystem, MTP devices, etc.) via
//! URL-style paths like:
//! - `file:///home/user/photos` (local filesystem)
//! - `mtp://phone/DCIM/Camera` (MTP device by index name)
//! - `mtp://SN:ABC123/DCIM` (MTP device by serial number)

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use super::{transfer::transfer_file, TransferStrategy, VfsBackend, VfsError, VfsResult};

/// A URL-style VFS location.
///
/// Format: `scheme://authority/path`
///
/// Examples:
/// - `file:///home/user/photos` → local path /home/user/photos
/// - `mtp://MyPhone/DCIM/Camera` → MTP device "MyPhone", path /DCIM/Camera
/// - `mtp://SN:ABC123/DCIM` → MTP device with serial ABC123
#[derive(Debug, Clone)]
pub struct VfsUrl {
    pub scheme: String,
    pub authority: String, // device name, serial, or empty for file://
    pub path: PathBuf,
}

impl VfsUrl {
    /// Parse a VFS URL string.
    ///
    /// # Examples
    ///
    /// ```
    /// use svault_core::vfs::manager::VfsUrl;
    ///
    /// let url = VfsUrl::parse("mtp://MyPhone/DCIM/Camera").unwrap();
    /// assert_eq!(url.scheme, "mtp");
    /// assert_eq!(url.authority, "MyPhone");
    /// assert_eq!(url.path, std::path::Path::new("/DCIM/Camera"));
    /// ```
    pub fn parse(url: &str) -> VfsResult<Self> {
        // Handle plain paths (treat as file://)
        if !url.contains("://") {
            return Ok(Self {
                scheme: "file".to_string(),
                authority: String::new(),
                path: PathBuf::from(url),
            });
        }

        let (scheme, rest) = url.split_once("://")
            .ok_or_else(|| VfsError::Other(format!("Invalid VFS URL: {}", url)))?;

        let (authority, path) = match rest.find('/') {
            Some(idx) => (&rest[..idx], &rest[idx..]),
            None => (rest, "/"),
        };

        Ok(Self {
            scheme: scheme.to_lowercase(),
            authority: authority.to_string(),
            path: PathBuf::from(path),
        })
    }

    /// Convert back to URL string.
    pub fn to_url(&self) -> String {
        if self.scheme == "file" && self.authority.is_empty() {
            self.path.to_string_lossy().to_string()
        } else {
            format!("{}://{}{}", self.scheme, self.authority, self.path.display())
        }
    }
}

/// Metadata about a discovered VFS source.
#[derive(Debug, Clone)]
pub struct VfsSource {
    /// Unique identifier for this source (e.g., "mtp://phone1", "file://")
    pub id: String,
    /// Human-readable display name (e.g., "My Android Phone", "Local Filesystem")
    pub name: String,
    /// Scheme type
    pub scheme: String,
    /// For MTP: manufacturer + model, for file: "Local"
    pub device_type: String,
    /// Available storages/roots (e.g., ["Internal", "SD Card"])
    pub roots: Vec<String>,
    /// Unique identifier (serial for MTP, machine-id for local, etc.)
    pub unique_id: String,
}

/// A provider that can discover and open VFS backends.
pub trait VfsProvider: Send + Sync {
    /// Returns the scheme this provider handles (e.g., "mtp", "file").
    fn scheme(&self) -> &str;

    /// Probe for available sources of this type.
    ///
    /// Returns a list of discovered sources that can be opened.
    fn probe(&self) -> VfsResult<Vec<VfsSource>>;

    /// Open a specific source by its ID.
    ///
    /// The source ID is the `id` field from `VfsSource`.
    fn open(&self, source_id: &str) -> VfsResult<Box<dyn VfsBackend>>;

    /// Open a source for a specific path/authority.
    ///
    /// This is used when the user specifies a path like `mtp://phone/DCIM`.
    /// The provider should find the matching device and return a backend
    /// positioned at that path.
    fn open_path(&self, authority: &str, path: &Path) -> VfsResult<Box<dyn VfsBackend>>;
}

/// Manager for all VFS providers and active connections.
pub struct VfsManager {
    providers: HashMap<String, Box<dyn VfsProvider>>,
    /// Cache of opened connections (reserved for future use)
    #[allow(dead_code)]
    connections: Mutex<HashMap<String, Arc<dyn VfsBackend>>>,
}

impl VfsManager {
    /// Create a new VFS manager with all available providers.
    pub fn new() -> Self {
        let mut providers: HashMap<String, Box<dyn VfsProvider>> = HashMap::new();

        // Register local filesystem provider
        providers.insert(
            "file".to_string(),
            Box::new(LocalFsProvider),
        );

        // Register MTP provider if feature is enabled
        #[cfg(feature = "mtp")]
        {
            providers.insert(
                "mtp".to_string(),
                Box::new(super::mtp::MtpProvider),
            );
        }

        Self {
            providers,
            connections: Mutex::new(HashMap::new()),
        }
    }

    /// Probe all providers for available sources.
    pub fn probe_all(&self) -> VfsResult<Vec<VfsSource>> {
        let mut all_sources = Vec::new();
        for provider in self.providers.values() {
            match provider.probe() {
                Ok(mut sources) => all_sources.append(&mut sources),
                Err(e) => {
                    log::debug!("Provider {} probe failed: {}", provider.scheme(), e);
                }
            }
        }
        Ok(all_sources)
    }

    /// Get a provider by scheme.
    fn get_provider(&self, scheme: &str) -> VfsResult<&dyn VfsProvider> {
        self.providers
            .get(scheme)
            .map(|p| p.as_ref())
            .ok_or_else(|| VfsError::Other(format!("Unknown VFS scheme: {}", scheme)))
    }

    /// Open a VFS backend from a URL string.
    ///
    /// Supports:
    /// - Plain paths (treated as file://)
    /// - URL format: `scheme://authority/path`
    ///
    /// Returns a backend and the path within that backend.
    pub fn open_url(&self, url: &str) -> VfsResult<(Box<dyn VfsBackend>, PathBuf)> {
        let parsed = VfsUrl::parse(url)?;
        let provider = self.get_provider(&parsed.scheme)?;
        let backend = provider.open_path(&parsed.authority, &parsed.path)?;
        Ok((backend, parsed.path))
    }

    /// Open a specific source by ID.
    ///
    /// Source IDs are returned by `probe_all()`.
    pub fn open_source(&self, source_id: &str) -> VfsResult<Box<dyn VfsBackend>> {
        // Parse source ID to determine scheme
        let scheme = if source_id.contains("://") {
            source_id.split("://").next().unwrap_or("file")
        } else {
            "file"
        };

        let provider = self.get_provider(scheme)?;
        provider.open(source_id)
    }

    /// Import a file or directory from source URL to destination path.
    ///
    /// This is a high-level convenience method.
    pub fn import(
        &self,
        source_url: &str,
        dest_backend: &dyn VfsBackend,
        dest_path: &Path,
        strategies: &[TransferStrategy],
    ) -> VfsResult<()> {
        let (src_backend, src_path) = self.open_url(source_url)?;
        
        // Check if source is a file or directory
        let entries = src_backend.list(&src_path)?;
        
        if entries.is_empty() {
            // Single file
            let file_name = src_path.file_name()
                .ok_or_else(|| VfsError::Other("Invalid source path".to_string()))?;
            let dest_file = dest_path.join(file_name);
            transfer_file(&*src_backend, &src_path, dest_backend, &dest_file, strategies)?;
        } else {
            // Directory - copy recursively
            for entry in entries {
                let src_entry_path = &entry.path;
                let rel_path = src_entry_path.strip_prefix(&src_path)
                    .unwrap_or(src_entry_path);
                let dest_entry_path = dest_path.join(rel_path);
                
                if entry.is_dir {
                    dest_backend.create_dir_all(&dest_entry_path)?;
                } else {
                    transfer_file(&*src_backend, src_entry_path, dest_backend, &dest_entry_path, strategies)?;
                }
            }
        }
        
        Ok(())
    }
}

impl Default for VfsManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Provider for local filesystem (always available).
pub struct LocalFsProvider;

impl VfsProvider for LocalFsProvider {
    fn scheme(&self) -> &str {
        "file"
    }

    fn probe(&self) -> VfsResult<Vec<VfsSource>> {
        // Local filesystem is always available
        Ok(vec![VfsSource {
            id: "file://".to_string(),
            name: "Local Filesystem".to_string(),
            scheme: "file".to_string(),
            device_type: "Local".to_string(),
            roots: vec!["/".to_string()],
            unique_id: "localhost".to_string(),
        }])
    }

    fn open(&self, _source_id: &str) -> VfsResult<Box<dyn VfsBackend>> {
        // For local filesystem, we open the root
        Ok(Box::new(super::system::SystemFs::open("/")?))
    }

    fn open_path(&self, _authority: &str, path: &Path) -> VfsResult<Box<dyn VfsBackend>> {
        // Authority is ignored for local filesystem
        let root = if path.as_os_str().is_empty() {
            PathBuf::from("/")
        } else {
            path.to_path_buf()
        };
        Ok(Box::new(super::system::SystemFs::open(root)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_url_file() {
        let url = VfsUrl::parse("file:///home/user/photos").unwrap();
        assert_eq!(url.scheme, "file");
        assert_eq!(url.authority, "");
        assert_eq!(url.path, Path::new("/home/user/photos"));
    }

    #[test]
    fn test_parse_url_mtp() {
        let url = VfsUrl::parse("mtp://MyPhone/DCIM/Camera").unwrap();
        assert_eq!(url.scheme, "mtp");
        assert_eq!(url.authority, "MyPhone");
        assert_eq!(url.path, Path::new("/DCIM/Camera"));
    }

    #[test]
    fn test_parse_plain_path() {
        let url = VfsUrl::parse("/home/user/photos").unwrap();
        assert_eq!(url.scheme, "file");
        assert_eq!(url.path, Path::new("/home/user/photos"));
    }

    #[test]
    fn test_url_roundtrip() {
        let original = "mtp://phone/DCIM/Camera";
        let url = VfsUrl::parse(original).unwrap();
        assert_eq!(url.to_url(), original);
    }
}
