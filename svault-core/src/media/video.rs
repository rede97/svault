//! Video metadata extraction for MP4/MOV/MTS formats.
//!
//! Supports:
//! - MP4/MOV (QuickTime/ISO BMFF): Extracts creation_time from mvhd box
//! - MTS (MPEG-TS): Extracts PCR timestamp or capture time from metadata

use super::{MediaError, Result};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// Video metadata extracted from file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoMetadata {
    /// Creation timestamp in milliseconds since Unix epoch
    pub creation_time_ms: Option<i64>,
    /// Camera/device model
    pub device_model: Option<String>,
    /// Camera/device make
    pub device_make: Option<String>,
}

/// Extract metadata from a video file.
pub fn extract_video_metadata(path: &Path) -> Result<VideoMetadata> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "mp4" | "m4v" | "mov" => extract_mp4_metadata(path),
        "mts" | "m2ts" | "ts" => extract_mts_metadata(path),
        _ => {
            // For other formats, try MP4 parser as fallback
            extract_mp4_metadata(path).or_else(|_| Ok(VideoMetadata::default()))
        }
    }
}

/// Extract metadata from MP4/MOV file (ISO Base Media File Format).
fn extract_mp4_metadata(path: &Path) -> Result<VideoMetadata> {
    let file = File::open(path).map_err(MediaError::Io)?;
    let mut reader = BufReader::new(file);

    // Parse boxes and look for moov > mvhd
    let mut metadata = VideoMetadata::default();

    while let Ok(Some(box_info)) = read_box_header(&mut reader) {
        match box_info.box_type.as_str() {
            "moov" => {
                // Parse moov box contents
                metadata = parse_moov_box(&mut reader, box_info.size)?;
                // We found what we need
                break;
            }
            "ftyp" | "mdat" | "free" | "skip" => {
                // Skip these boxes
                skip_box(&mut reader, box_info.size, box_info.header_size)?;
            }
            _ => {
                // Unknown box, skip
                skip_box(&mut reader, box_info.size, box_info.header_size)?;
            }
        }
    }

    Ok(metadata)
}

/// Extract metadata from MTS/M2TS file (MPEG-Transport Stream).
fn extract_mts_metadata(_path: &Path) -> Result<VideoMetadata> {
    // MTS metadata extraction is complex and requires parsing MPEG-TS packets
    // For now, return empty metadata (fallback to file mtime)
    // TODO: Implement PCR and timestamp extraction
    Ok(VideoMetadata::default())
}

/// Box header information.
#[derive(Debug)]
struct BoxInfo {
    box_type: String,
    size: u64,
    header_size: u8,
}

/// Read box header from MP4 file.
fn read_box_header<R: Read + Seek>(reader: &mut R) -> Result<Option<BoxInfo>> {
    let pos = reader.stream_position().map_err(MediaError::Io)?;
    
    // Read size (4 bytes)
    let mut size_buf = [0u8; 4];
    match reader.read_exact(&mut size_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(MediaError::Io(e)),
    }
    let size = u32::from_be_bytes(size_buf) as u64;

    // Read type (4 bytes)
    let mut type_buf = [0u8; 4];
    reader.read_exact(&mut type_buf).map_err(MediaError::Io)?;
    let box_type = String::from_utf8_lossy(&type_buf).to_string();

    let header_size: u8;
    let final_size: u64;

    if size == 1 {
        // Extended size (64-bit)
        let mut ext_size_buf = [0u8; 8];
        reader.read_exact(&mut ext_size_buf).map_err(MediaError::Io)?;
        final_size = u64::from_be_bytes(ext_size_buf);
        header_size = 16;
    } else if size == 0 {
        // Box extends to end of file
        let current = reader.stream_position().map_err(MediaError::Io)?;
        let end = reader.seek(SeekFrom::End(0)).map_err(MediaError::Io)?;
        reader.seek(SeekFrom::Start(current)).map_err(MediaError::Io)?;
        final_size = end - pos;
        header_size = 8;
    } else {
        final_size = size;
        header_size = 8;
    }

    Ok(Some(BoxInfo {
        box_type,
        size: final_size,
        header_size,
    }))
}

/// Skip over a box.
fn skip_box<R: Read + Seek>(reader: &mut R, box_size: u64, header_size: u8) -> Result<()> {
    let data_size = box_size.saturating_sub(header_size as u64);
    let current = reader.stream_position().map_err(MediaError::Io)?;
    reader
        .seek(SeekFrom::Start(current + data_size))
        .map_err(MediaError::Io)?;
    Ok(())
}

/// Parse moov box to extract metadata.
fn parse_moov_box<R: Read + Seek>(reader: &mut R, box_size: u64) -> Result<VideoMetadata> {
    let end_pos = reader.stream_position().map_err(MediaError::Io)? + box_size.saturating_sub(8);
    let mut metadata = VideoMetadata::default();

    while reader.stream_position().map_err(MediaError::Io)? < end_pos {
        match read_box_header(reader)? {
            Some(box_info) => {
                match box_info.box_type.as_str() {
                    "mvhd" => {
                        // Movie header box - contains creation_time
                        metadata.creation_time_ms = parse_mvhd_box(reader, box_info.size)?;
                    }
                    "trak" | "udta" | "meta" => {
                        // We could extract more metadata from these
                        skip_box(reader, box_info.size, box_info.header_size)?;
                    }
                    _ => {
                        skip_box(reader, box_info.size, box_info.header_size)?;
                    }
                }
            }
            None => break,
        }
    }

    Ok(metadata)
}

/// Parse mvhd box to extract creation_time.
/// 
/// QuickTime/MP4 time epoch is different from Unix epoch:
/// - QuickTime epoch: 1904-01-01 00:00:00 UTC
/// - Unix epoch: 1970-01-01 00:00:00 UTC
/// - Difference: 2_082_844_800 seconds
fn parse_mvhd_box<R: Read + Seek>(reader: &mut R, box_size: u64) -> Result<Option<i64>> {
    let start_pos = reader.stream_position().map_err(MediaError::Io)?;
    let data_size = box_size.saturating_sub(8); // Subtract header size

    // Read version and flags (4 bytes)
    let mut ver_flags = [0u8; 4];
    reader.read_exact(&mut ver_flags).map_err(MediaError::Io)?;
    let version = ver_flags[0];

    // Read creation_time based on version
    let creation_time: u64;
    if version == 0 {
        // Version 0: 32-bit timestamps
        let mut buf = [0u8; 4];
        reader.read_exact(&mut buf).map_err(MediaError::Io)?;
        creation_time = u32::from_be_bytes(buf) as u64;
    } else {
        // Version 1: 64-bit timestamps
        let mut buf = [0u8; 8];
        reader.read_exact(&mut buf).map_err(MediaError::Io)?;
        creation_time = u64::from_be_bytes(buf);
    }

    // Skip to end of box
    let current = reader.stream_position().map_err(MediaError::Io)?;
    let remaining = data_size.saturating_sub((current - start_pos) as u64);
    reader
        .seek(SeekFrom::Current(remaining as i64))
        .map_err(MediaError::Io)?;

    // Convert QuickTime time to Unix milliseconds
    // QuickTime epoch starts at 1904-01-01, Unix at 1970-01-01
    // Difference: 66 years = 2_082_844_800 seconds
    const QT_TO_UNIX_OFFSET: u64 = 2_082_844_800;

    if creation_time > 0 && creation_time > QT_TO_UNIX_OFFSET {
        let unix_secs = creation_time - QT_TO_UNIX_OFFSET;
        Ok(Some((unix_secs * 1000) as i64))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Create a minimal MP4 file with mvhd box for testing.
    fn create_test_mp4(creation_time: u64, version: u8) -> Vec<u8> {
        let mut data = Vec::new();

        // ftyp box: size(4) + "ftyp"(4) + major_brand(4) + minor_version(4) + compatible_brands(N)
        let ftyp_content = b"isom\x00\x00\x00\x00isomiso2mp41";
        let ftyp_size = 8 + ftyp_content.len() as u32;
        data.extend_from_slice(&ftyp_size.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(ftyp_content);

        // moov box
        let mut moov_content = Vec::new();

        // mvhd box
        let mut mvhd = Vec::new();
        mvhd.push(version); // version
        mvhd.extend_from_slice(&[0, 0, 0]); // flags

        if version == 0 {
            // 32-bit timestamps
            mvhd.extend_from_slice(&(creation_time as u32).to_be_bytes()); // creation_time
            mvhd.extend_from_slice(&(creation_time as u32).to_be_bytes()); // modification_time
            mvhd.extend_from_slice(&1000u32.to_be_bytes()); // timescale
            mvhd.extend_from_slice(&0u32.to_be_bytes()); // duration
        } else {
            // 64-bit timestamps
            mvhd.extend_from_slice(&creation_time.to_be_bytes()); // creation_time
            mvhd.extend_from_slice(&creation_time.to_be_bytes()); // modification_time
            mvhd.extend_from_slice(&1000u32.to_be_bytes()); // timescale
            mvhd.extend_from_slice(&0u64.to_be_bytes()); // duration
        }
        // Add remaining mvhd fields (rate, volume, matrix, etc.)
        mvhd.extend_from_slice(&[0u8; 100]); // placeholder

        // mvhd box header
        moov_content.extend_from_slice(&(mvhd.len() as u32 + 8).to_be_bytes());
        moov_content.extend_from_slice(b"mvhd");
        moov_content.extend_from_slice(&mvhd);

        // moov box header + content
        data.extend_from_slice(&(moov_content.len() as u32 + 8).to_be_bytes());
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&moov_content);

        data
    }

    #[test]
    fn test_parse_mp4_creation_time_v0() {
        // Test with version 0 (32-bit) timestamp
        // 2024-01-01 00:00:00 UTC in QuickTime epoch
        // Unix timestamp for 2024-01-01: 1704067200
        // QuickTime timestamp: 1704067200 + 2082844800 = 3786912000
        let unix_secs: u64 = 1_704_067_200;
        let qt_secs: u64 = unix_secs + 2_082_844_800;

        let mp4_data = create_test_mp4(qt_secs, 0);
        let mut cursor = Cursor::new(mp4_data);

        // Skip ftyp box
        let mut size_buf = [0u8; 4];
        cursor.read_exact(&mut size_buf).unwrap();
        let ftyp_size = u32::from_be_bytes(size_buf) as u64;
        cursor.seek(SeekFrom::Start(ftyp_size)).unwrap();

        // Read moov box
        let box_info = read_box_header(&mut cursor).unwrap().unwrap();
        assert_eq!(box_info.box_type, "moov");

        // Parse moov
        let metadata = parse_moov_box(&mut cursor, box_info.size).unwrap();

        assert!(metadata.creation_time_ms.is_some());
        assert_eq!(metadata.creation_time_ms.unwrap() / 1000, unix_secs as i64);
    }

    #[test]
    fn test_parse_mp4_creation_time_v1() {
        // Test with version 1 (64-bit) timestamp
        let unix_secs: u64 = 1_704_067_200; // 2024-01-01
        let qt_secs: u64 = unix_secs + 2_082_844_800;

        let mp4_data = create_test_mp4(qt_secs, 1);
        let mut cursor = Cursor::new(mp4_data);

        // Skip ftyp
        let mut size_buf = [0u8; 4];
        cursor.read_exact(&mut size_buf).unwrap();
        let ftyp_size = u32::from_be_bytes(size_buf) as u64;
        cursor.seek(SeekFrom::Start(ftyp_size)).unwrap();

        // Read moov
        let box_info = read_box_header(&mut cursor).unwrap().unwrap();
        assert_eq!(box_info.box_type, "moov");

        let metadata = parse_moov_box(&mut cursor, box_info.size).unwrap();

        assert!(metadata.creation_time_ms.is_some());
        assert_eq!(metadata.creation_time_ms.unwrap() / 1000, unix_secs as i64);
    }

    #[test]
    fn test_invalid_creation_time() {
        // Creation time of 0 or before Unix epoch should return None
        let mp4_data = create_test_mp4(0, 0);
        let mut cursor = Cursor::new(mp4_data);

        // Skip ftyp
        let mut size_buf = [0u8; 4];
        cursor.read_exact(&mut size_buf).unwrap();
        let ftyp_size = u32::from_be_bytes(size_buf) as u64;
        cursor.seek(SeekFrom::Start(ftyp_size)).unwrap();

        let box_info = read_box_header(&mut cursor).unwrap().unwrap();
        let metadata = parse_moov_box(&mut cursor, box_info.size).unwrap();

        assert!(metadata.creation_time_ms.is_none());
    }
}
