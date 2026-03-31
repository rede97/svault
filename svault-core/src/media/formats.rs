//! Media format definitions and detection.

use super::crc::CrcStrategy;
use super::{MediaError, Result};
use std::ffi::OsStr;
use std::fmt;
use std::path::Path;

/// Known media formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaFormat {
    // Image formats
    /// JPEG image
    Jpeg,
    /// PNG image
    Png,
    /// HEIF image (Apple, Samsung)
    Heif,
    /// HEIC image (Apple variant of HEIF)
    Heic,
    /// AVIF image
    Avif,
    /// WebP image
    Webp,

    // Video formats
    /// QuickTime movie (Apple, many cameras)
    Mov,
    /// MPEG-4 video
    Mp4,
    /// AVI video
    Avi,
    /// Matroska video
    Mkv,

    // RAW formats
    /// Adobe DNG
    Dng,
    /// Sony ARW
    Arw,
    /// Canon CR2
    Cr2,
    /// Canon CR3
    Cr3,
    /// Nikon NEF
    Nef,
    /// Fuji RAF
    Raf,
    /// Panasonic/Leica RW2
    Rw2,

    /// Unknown format with extension
    Unknown(&'static str),
}

impl MediaFormat {
    /// Detect format from file extension.
    pub fn from_path(path: &Path) -> Result<Self> {
        let ext = path
            .extension()
            .and_then(OsStr::to_str)
            .map(|s| s.to_lowercase())
            .ok_or_else(|| MediaError::UnsupportedFormat("no extension".to_string()))?;

        Self::from_extension(&ext)
    }

    /// Detect format from extension string.
    pub fn from_extension(ext: &str) -> Result<Self> {
        match ext.to_lowercase().as_str() {
            // Images
            "jpg" | "jpeg" => Ok(Self::Jpeg),
            "png" => Ok(Self::Png),
            "heif" => Ok(Self::Heif),
            "heic" => Ok(Self::Heic),
            "avif" => Ok(Self::Avif),
            "webp" => Ok(Self::Webp),

            // Video
            "mov" => Ok(Self::Mov),
            "mp4" | "m4v" => Ok(Self::Mp4),
            "avi" => Ok(Self::Avi),
            "mkv" => Ok(Self::Mkv),

            // RAW
            "dng" => Ok(Self::Dng),
            "arw" => Ok(Self::Arw),
            "cr2" => Ok(Self::Cr2),
            "cr3" => Ok(Self::Cr3),
            "nef" => Ok(Self::Nef),
            "raf" => Ok(Self::Raf),
            "rw2" => Ok(Self::Rw2),

            _ => Ok(Self::Unknown(Box::leak(ext.to_string().into_boxed_str()))),
        }
    }

    /// Get the file extension for this format.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Heif => "heif",
            Self::Heic => "heic",
            Self::Avif => "avif",
            Self::Webp => "webp",
            Self::Mov => "mov",
            Self::Mp4 => "mp4",
            Self::Avi => "avi",
            Self::Mkv => "mkv",
            Self::Dng => "dng",
            Self::Arw => "arw",
            Self::Cr2 => "cr2",
            Self::Cr3 => "cr3",
            Self::Nef => "nef",
            Self::Raf => "raf",
            Self::Rw2 => "rw2",
            Self::Unknown(ext) => ext,
        }
    }

    /// Get the primary category for this format.
    pub fn category(&self) -> FormatCategory {
        match self {
            Self::Jpeg | Self::Png | Self::Heif | Self::Heic | Self::Avif | Self::Webp => {
                FormatCategory::Image
            }
            Self::Mov | Self::Mp4 | Self::Avi | Self::Mkv => FormatCategory::Video,
            Self::Dng | Self::Arw | Self::Cr2 | Self::Cr3 | Self::Nef | Self::Raf | Self::Rw2 => {
                FormatCategory::Raw
            }
            Self::Unknown(_) => FormatCategory::Unknown,
        }
    }

    /// Check if this format can be part of a Live Photo.
    pub fn is_live_photo_component(&self) -> bool {
        matches!(
            self,
            Self::Heic | Self::Heif | Self::Jpeg | Self::Mov | Self::Mp4
        )
    }

    /// Check if this format is a RAW image.
    pub fn is_raw(&self) -> bool {
        self.category() == FormatCategory::Raw
    }

    /// Check if this format can be the primary image in RAW+JPG pair.
    pub fn is_raw_plus_jpg_primary(&self) -> bool {
        matches!(
            self,
            Self::Jpeg | Self::Heic | Self::Heif // The JPG/HEIF side
        )
    }

    /// Check if this format can be the RAW component in RAW+JPG pair.
    pub fn is_raw_plus_jpg_secondary(&self) -> bool {
        self.is_raw()
    }

    /// Get the checksum strategy for this format.
    pub(crate) fn checksum_strategy(&self) -> CrcStrategy {
        CrcStrategy::for_format(self)
    }

    /// Get MIME type for this format.
    pub fn mime_type(&self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Heif => "image/heif",
            Self::Heic => "image/heic",
            Self::Avif => "image/avif",
            Self::Webp => "image/webp",
            Self::Mov => "video/quicktime",
            Self::Mp4 => "video/mp4",
            Self::Avi => "video/avi",
            Self::Mkv => "video/x-matroska",
            Self::Dng => "image/dng",
            Self::Arw => "image/x-sony-arw",
            Self::Cr2 => "image/x-canon-cr2",
            Self::Cr3 => "image/x-canon-cr3",
            Self::Nef => "image/x-nikon-nef",
            Self::Raf => "image/x-fuji-raf",
            Self::Rw2 => "image/x-panasonic-rw2",
            Self::Unknown(_) => "application/octet-stream",
        }
    }
}

impl fmt::Display for MediaFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(ext) => write!(f, "Unknown({})", ext),
            _ => write!(f, "{:?}", self),
        }
    }
}

/// Format categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormatCategory {
    Image,
    Video,
    Raw,
    Unknown,
}

impl fmt::Display for FormatCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Image => write!(f, "image"),
            Self::Video => write!(f, "video"),
            Self::Raw => write!(f, "raw"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Registry for managing format support.
pub struct FormatRegistry {
    supported: Vec<MediaFormat>,
}

impl Default for FormatRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FormatRegistry {
    /// Create a new registry with all known formats.
    pub fn new() -> Self {
        use MediaFormat::*;

        Self {
            supported: vec![
                Jpeg, Png, Heif, Heic, Avif, Webp, Mov, Mp4, Avi, Mkv, Dng, Arw, Cr2, Cr3, Nef,
                Raf, Rw2,
            ],
        }
    }

    /// Create a registry with only photo formats (no video).
    pub fn photos_only() -> Self {
        use MediaFormat::*;

        Self {
            supported: vec![
                Jpeg, Png, Heif, Heic, Avif, Webp, Dng, Arw, Cr2, Cr3, Nef, Raf, Rw2,
            ],
        }
    }

    /// Check if a format is supported.
    pub fn is_supported(&self, format: &MediaFormat) -> bool {
        matches!(format, MediaFormat::Unknown(_)).then(|| false).unwrap_or(
            self.supported.iter().any(|f| f == format)
        )
    }

    /// Check if a file extension is supported.
    pub fn is_extension_supported(&self, ext: &str) -> bool {
        MediaFormat::from_extension(ext)
            .map(|f| self.is_supported(&f))
            .unwrap_or(false)
    }

    /// Get all supported formats.
    pub fn supported_formats(&self) -> &[MediaFormat] {
        &self.supported
    }

    /// Add support for a custom format.
    pub fn add_format(&mut self, format: MediaFormat) {
        if !self.supported.contains(&format) {
            self.supported.push(format);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_detection() {
        assert_eq!(
            MediaFormat::from_path(Path::new("photo.jpg")).unwrap(),
            MediaFormat::Jpeg
        );
        assert_eq!(
            MediaFormat::from_path(Path::new("photo.JPG")).unwrap(),
            MediaFormat::Jpeg
        );
        assert_eq!(
            MediaFormat::from_path(Path::new("movie.mov")).unwrap(),
            MediaFormat::Mov
        );
        assert_eq!(
            MediaFormat::from_path(Path::new("raw.dng")).unwrap(),
            MediaFormat::Dng
        );
    }

    #[test]
    fn test_category() {
        assert_eq!(MediaFormat::Jpeg.category(), FormatCategory::Image);
        assert_eq!(MediaFormat::Mov.category(), FormatCategory::Video);
        assert_eq!(MediaFormat::Dng.category(), FormatCategory::Raw);
    }

    #[test]
    fn test_is_raw() {
        assert!(!MediaFormat::Jpeg.is_raw());
        assert!(MediaFormat::Dng.is_raw());
        assert!(MediaFormat::Arw.is_raw());
    }
}
