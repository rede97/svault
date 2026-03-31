//! CRC32 checksum strategies for media files.
//!
//! This module is internal - users interact with checksums through `MediaInfo::checksum`.

use crate::media::formats::MediaFormat;
use crate::media::{MediaReader, Result, CHECKSUM_BUFFER_SIZE};
use std::fs::File;
use std::io::SeekFrom;
use std::path::Path;

/// Compute checksum for a file based on its format.
pub fn compute_checksum(path: &Path, format: &MediaFormat) -> Result<u32> {
    let mut file = File::open(path)?;
    let strategy = CrcStrategy::for_format(format);
    
    match strategy {
        CrcStrategy::Head(n) => compute_head(&mut file, n),
        CrcStrategy::Tail(n) => compute_tail(&mut file, n),
        CrcStrategy::Full => compute_full(&mut file),
        CrcStrategy::Range(start, len) => compute_range(&mut file, start, len),
        CrcStrategy::Custom(func) => func(&mut file),
    }
}

/// Checksum strategy implementations.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum CrcStrategy {
    /// Read first N bytes from start of file.
    Head(usize),
    /// Read last N bytes from end of file.
    Tail(usize),
    /// Read the entire file.
    Full,
    /// Read a specific byte range.
    Range(usize, usize),
    /// Custom strategy for complex formats.
    Custom(fn(&mut dyn MediaReader) -> Result<u32>),
}

impl CrcStrategy {
    /// Get the default strategy for a media format.
    pub fn for_format(format: &MediaFormat) -> Self {
        use MediaFormat::*;

        match format {
            // Image formats: use head to avoid mutable metadata
            Jpeg | Heif | Heic | Avif | Webp => CrcStrategy::Head(CHECKSUM_BUFFER_SIZE),

            // PNG: use tail (image data is at the end, metadata at start)
            Png => CrcStrategy::Tail(CHECKSUM_BUFFER_SIZE),

            // Video formats: head is usually enough (moov atom)
            Mov | Mp4 | Avi | Mkv => CrcStrategy::Head(CHECKSUM_BUFFER_SIZE),

            // RAW formats: these are sensitive, use full file
            Dng | Arw | Cr2 | Cr3 | Nef | Raf | Rw2 => CrcStrategy::Full,

            // Unknown: use full file to be safe
            Unknown(_) => CrcStrategy::Full,
        }
    }
}

/// Compute CRC32 of first N bytes.
fn compute_head(reader: &mut dyn MediaReader, n: usize) -> Result<u32> {

    reader.seek(SeekFrom::Start(0))?;
    let mut buffer = vec![0u8; n];
    let read = reader.read(&mut buffer)?;
    buffer.truncate(read);
    Ok(crate::media::crc32_bytes(&buffer))
}

/// Compute CRC32 of last N bytes.
fn compute_tail(reader: &mut dyn MediaReader, n: usize) -> Result<u32> {

    let len = reader.len()?;
    let start = if len > n as u64 { len - n as u64 } else { 0 };

    reader.seek(SeekFrom::Start(start))?;
    let mut buffer = vec![0u8; n];
    let read = reader.read(&mut buffer)?;
    buffer.truncate(read);
    Ok(crate::media::crc32_bytes(&buffer))
}

/// Compute CRC32 of entire file.
fn compute_full(reader: &mut dyn MediaReader) -> Result<u32> {

    reader.seek(SeekFrom::Start(0))?;
    let mut hasher = crc32fast::Hasher::new();
    let mut buffer = [0u8; 8192];

    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(hasher.finalize())
}

/// Compute CRC32 of a specific byte range.
fn compute_range(reader: &mut dyn MediaReader, start: usize, len: usize) -> Result<u32> {

    reader.seek(SeekFrom::Start(start as u64))?;
    let mut buffer = vec![0u8; len];
    let read = reader.read(&mut buffer)?;
    buffer.truncate(read);
    Ok(crate::media::crc32_bytes(&buffer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn test_data() -> Vec<u8> {
        vec![0u8; 128 * 1024] // 128KB of zeros
    }

    #[test]
    fn test_head_checksum() {
        let data = test_data();
        let expected = crate::media::crc32_bytes(&data[..64 * 1024]);

        let mut cursor = Cursor::new(data);
        let result = compute_head(&mut cursor, 64 * 1024).unwrap();

        assert_eq!(result, expected);
    }

    #[test]
    fn test_tail_checksum() {
        let data = test_data();
        let expected = crate::media::crc32_bytes(&data[data.len() - 64 * 1024..]);

        let mut cursor = Cursor::new(data);
        let result = compute_tail(&mut cursor, 64 * 1024).unwrap();

        assert_eq!(result, expected);
    }

    #[test]
    fn test_full_checksum() {
        let data = test_data();
        let expected = crate::media::crc32_bytes(&data);

        let mut cursor = Cursor::new(data);
        let result = compute_full(&mut cursor).unwrap();

        assert_eq!(result, expected);
    }
}
