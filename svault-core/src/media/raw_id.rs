//! Extract unique identifiers from RAW files using EXIF metadata.
//!
//! Many RAW formats embed camera serial number and unique image ID in EXIF,
//! providing stronger identity than CRC alone for duplicate detection.

use std::io::BufReader;
use std::path::Path;

/// Unique identifier extracted from a RAW file's EXIF metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawId {
    /// Camera body serial number
    pub camera_serial: Option<String>,
    /// Image unique ID (typically a UUID or counter assigned by camera)
    pub image_id: Option<String>,
    /// Combined fingerprint: "{serial}:{image_id}" (if both present)
    pub unique_fingerprint: Option<String>,
}

impl RawId {
    /// Returns true if this RawId has enough information for precise matching
    pub fn is_valid(&self) -> bool {
        self.unique_fingerprint.is_some()
    }
}

/// Check if file extension is a RAW format that we support for ID extraction
pub fn is_raw_file(ext: &str) -> bool {
    let ext = ext.to_lowercase();
    matches!(
        ext.as_str(),
        "dng" | "arw" | "cr2" | "cr3" | "nef" | "raf" | "rw2"
    )
}

/// Extract unique ID from a RAW file
pub fn extract_raw_id(path: &Path) -> anyhow::Result<RawId> {
    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);

    let exif = exif::Reader::new().read_from_container(&mut reader)?;

    let mut camera_serial = None;
    let mut image_id = None;

    for field in exif.fields() {
        match field.tag {
            // BodySerialNumber - Camera body serial
            exif::Tag::BodySerialNumber => {
                camera_serial = Some(field.display_value().with_unit(&exif).to_string());
            }
            // ImageUniqueID - Unique ID assigned by camera
            exif::Tag::ImageUniqueID => {
                image_id = Some(field.display_value().with_unit(&exif).to_string());
            }
            _ => {}
        }
    }

    // Clean up values (remove quotes, trim whitespace)
    camera_serial =
        camera_serial.map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string());
    image_id = image_id.map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string());

    // Create combined fingerprint if both values are present
    let unique_fingerprint = match (&camera_serial, &image_id) {
        (Some(serial), Some(id)) if !serial.is_empty() && !id.is_empty() => {
            Some(format!("{}:{}", serial, id))
        }
        _ => None,
    };

    Ok(RawId {
        camera_serial,
        image_id,
        unique_fingerprint,
    })
}

/// Extract unique ID if path is a RAW file, otherwise return None
pub fn extract_raw_id_if_raw(path: &Path) -> Option<RawId> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    if !is_raw_file(ext) {
        return None;
    }

    extract_raw_id(path).ok()
}

/// Get the unique fingerprint string for database storage
pub fn get_fingerprint_string(raw_id: &RawId) -> Option<String> {
    raw_id.unique_fingerprint.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_raw_file() {
        assert!(is_raw_file("ARW"));
        assert!(is_raw_file("cr2"));
        assert!(is_raw_file("DNG"));
        assert!(!is_raw_file("jpg"));
        assert!(!is_raw_file("png"));
    }
}
