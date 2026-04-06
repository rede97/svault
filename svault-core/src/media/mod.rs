//! Media format abstraction layer.
//!
//! Provides:
//! - Format-specific CRC32 checksum strategies (internal implementation detail)
//! - Media binding detection (Live Photo, RAW+JPG pairs, etc.)
//! - Format classification and metadata extraction
//!
//! # CRC32 Strategy
//!
//! The CRC32 calculation is an internal implementation detail. Different formats
//! use different strategies to compute a stable checksum:
//!
//! - **JPEG**: CRC32 of first 64KB (excludes mutable metadata at end)
//! - **PNG**: CRC32 of last 64KB (covers image data, excludes header)
//! - **HEIF**: CRC32 of first 64KB
//! - **MOV/MP4**: CRC32 of first 64KB (moov atom typically at start or end)
//! - **RAW**: Full file CRC32 (DNG, ARW, etc.)
//!
//! # Media Bindings
//!
//! Some cameras produce multiple files for a single capture:
//!
//! - **Live Photo**: .heic/.jpg + .mov (Apple, some Android)
//! - **RAW+JPG**: .dng/.arw/.cr2 + .jpg/.jpeg (dual recording)
//! - **Burst**: Multiple .jpg with sequence numbers
//!
//! Use `BindingDetector` to identify related files.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use thiserror::Error;

mod binding;
pub mod crc;
pub mod formats;
pub mod video;

pub use binding::{BindingDetector, BindingKind, MediaBinding};
pub use formats::{FormatRegistry, MediaFormat};
pub use video::{extract_video_metadata, VideoMetadata};

/// Errors that can occur during media operations.
#[derive(Error, Debug)]
pub enum MediaError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("Invalid media file: {0}")]
    InvalidFile(String),
}

/// Result type for media operations.
pub type Result<T> = std::result::Result<T, MediaError>;

/// Media file information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaInfo {
    /// File path
    pub path: std::path::PathBuf,
    /// Detected format
    pub format: MediaFormat,
    /// File size in bytes
    pub size: u64,
    /// Format-specific checksum (stable identifier)
    pub checksum: u32,
    /// Optional: capture timestamp from metadata
    pub capture_time: Option<chrono::DateTime<chrono::Utc>>,
    /// Optional: camera model
    pub camera_model: Option<String>,
}

impl MediaInfo {
    /// Analyze a media file and compute its checksum.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let format = MediaFormat::from_path(path)?;
        let size = std::fs::metadata(path)?.len();
        let checksum = crc::compute_checksum(path, &format)?;

        Ok(Self {
            path: path.to_path_buf(),
            format,
            size,
            checksum,
            capture_time: None,
            camera_model: None,
        })
    }

    /// Check if this media is part of a binding (e.g., Live Photo, RAW+JPG).
    pub fn binding_key(&self) -> Option<BindingKey> {
        binding::compute_binding_key(self)
    }
}

/// Key for matching related media files.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BindingKey {
    /// Base identifier (e.g., timestamp, sequence number)
    pub base: String,
    /// Binding type
    pub kind: BindingKind,
}



/// Trait for reading media data with seeking support.
pub(crate) trait MediaReader: Read + Seek {
    fn len(&mut self) -> io::Result<u64>;
}

impl<R: Read + Seek> MediaReader for R {
    fn len(&mut self) -> io::Result<u64> {
        let current = self.stream_position()?;
        let end = self.seek(SeekFrom::End(0))?;
        self.seek(SeekFrom::Start(current))?;
        Ok(end)
    }
}

/// Default 64KB buffer size for partial checksums.
pub(crate) const CHECKSUM_BUFFER_SIZE: usize = 64 * 1024;

/// Compute CRC32 of a byte slice.
pub(crate) fn crc32_bytes(data: &[u8]) -> u32 {
    crc32fast::hash(data)
}
