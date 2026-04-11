//! RAW file unique identifier extraction.
//!
//! Extracts camera serial number and image unique ID from various RAW formats.
//! This information is used as an additional duplicate detection layer beyond CRC32.

use std::io::Read;
use std::path::Path;

use super::Result;

/// Unique identifier extracted from a RAW file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RawUniqueId {
    /// Camera body serial number (from EXIF)
    pub camera_serial: Option<String>,
    /// Image unique ID assigned by the camera
    pub image_unique_id: Option<String>,
}

impl RawUniqueId {
    /// Create an empty identifier (for files without EXIF data).
    pub fn empty() -> Self {
        Self {
            camera_serial: None,
            image_unique_id: None,
        }
    }

    /// Check if we have both components for reliable identification.
    pub fn is_complete(&self) -> bool {
        self.camera_serial.is_some() && self.image_unique_id.is_some()
    }

    /// Format as a compact string for database storage.
    /// Format: "<camera_serial>:<image_unique_id>" or partial if incomplete.
    pub fn to_db_string(&self) -> Option<String> {
        match (&self.camera_serial, &self.image_unique_id) {
            (Some(serial), Some(id)) => Some(format!("{}:{}", serial, id)),
            (Some(serial), None) => Some(format!("{}:", serial)),
            (None, Some(id)) => Some(format!(":{}", id)),
            (None, None) => None,
        }
    }
}

/// Extract unique identifier from a RAW file.
///
/// This function tries to extract EXIF data from RAW files. Most modern cameras
/// embed a thumbnail JPEG with full EXIF data that we can parse.
pub fn extract_raw_unique_id(path: &Path) -> Result<RawUniqueId> {
    // Try to read the first 256KB which usually contains embedded JPEG preview
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; 256 * 1024];
    let n = file.read(&mut buffer)?;
    buffer.truncate(n);

    // Look for JPEG SOI marker (FF D8) and extract EXIF from embedded preview
    if let Some(exif_data) = find_embedded_jpeg_exif(&buffer) {
        return parse_exif_unique_id(&exif_data);
    }

    // Fallback: try to parse as TIFF/RAW directly
    if buffer.starts_with(b"II*\0") || buffer.starts_with(b"MM\0*") {
        return parse_tiff_exif(&buffer);
    }

    Ok(RawUniqueId::empty())
}

/// Find embedded JPEG in RAW file and extract its EXIF segment.
fn find_embedded_jpeg_exif(data: &[u8]) -> Option<Vec<u8>> {
    // Look for JPEG SOI marker (FF D8)
    for i in 0..data.len().saturating_sub(2) {
        if data[i] == 0xFF && data[i + 1] == 0xD8 {
            // Found JPEG start, extract up to reasonable size (2MB max for preview)
            let end = (i + 2 * 1024 * 1024).min(data.len());
            return Some(data[i..end].to_vec());
        }
    }
    None
}

/// Parse EXIF data to extract camera serial and image unique ID.
fn parse_exif_unique_id(jpeg_data: &[u8]) -> Result<RawUniqueId> {
    use exif::{In, Tag};

    let mut camera_serial = None;
    let mut image_unique_id = None;

    match exif::Reader::new().read_raw(jpeg_data.to_vec()) {
        Ok(exif) => {
            // Extract camera serial number
            if let Some(field) = exif.get_field(Tag::BodySerialNumber, In::PRIMARY) {
                camera_serial = Some(field.display_value().to_string());
            }

            // Extract image unique ID
            if let Some(field) = exif.get_field(Tag::ImageUniqueID, In::PRIMARY) {
                image_unique_id = Some(field.display_value().to_string());
            }

            Ok(RawUniqueId {
                camera_serial,
                image_unique_id,
            })
        }
        Err(_) => Ok(RawUniqueId::empty()),
    }
}

/// Parse TIFF-based EXIF directly from RAW file header.
fn parse_tiff_exif(data: &[u8]) -> Result<RawUniqueId> {
    // For TIFF-based RAWs, the EXIF IFD is usually at a specific offset
    // This is a simplified parser - in production we might want more comprehensive handling

    // Check byte order
    let little_endian = data.starts_with(b"II*\0");
    let _big_endian = data.starts_with(b"MM\0*");

    if !little_endian && !_big_endian {
        return Ok(RawUniqueId::empty());
    }

    // For now, return empty - the embedded JPEG preview method is more reliable
    // and most modern RAWs have embedded JPEGs
    Ok(RawUniqueId::empty())
}

/// Check if a file extension indicates a RAW format.
pub fn is_raw_format(path: &Path) -> bool {
    use super::formats::MediaFormat;

    match MediaFormat::from_path(path) {
        Ok(format) => matches!(
            format,
            MediaFormat::Dng
                | MediaFormat::Arw
                | MediaFormat::Cr2
                | MediaFormat::Cr3
                | MediaFormat::Nef
                | MediaFormat::Raf
                | MediaFormat::Rw2
        ),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_unique_id_empty() {
        let id = RawUniqueId::empty();
        assert!(!id.is_complete());
        assert!(id.to_db_string().is_none());
    }

    #[test]
    fn test_raw_unique_id_complete() {
        let id = RawUniqueId {
            camera_serial: Some("ABC123".to_string()),
            image_unique_id: Some("IMG001".to_string()),
        };
        assert!(id.is_complete());
        assert_eq!(id.to_db_string(), Some("ABC123:IMG001".to_string()));
    }

    #[test]
    fn test_raw_unique_id_partial() {
        let id = RawUniqueId {
            camera_serial: Some("ABC123".to_string()),
            image_unique_id: None,
        };
        assert!(!id.is_complete());
        assert_eq!(id.to_db_string(), Some("ABC123:".to_string()));
    }
}
