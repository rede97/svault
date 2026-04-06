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
    compute_checksum_with_strategy(&mut file, strategy)
}

/// Compute checksum with a specific strategy.
pub(crate) fn compute_checksum_with_strategy(
    reader: &mut dyn MediaReader,
    strategy: CrcStrategy,
) -> Result<u32> {
    match strategy {
        CrcStrategy::Head(n) => compute_head(reader, n),
        CrcStrategy::Tail(n) => compute_tail(reader, n),
        CrcStrategy::HeadTail(head_n, tail_m) => {
            // Read head
            reader.seek(SeekFrom::Start(0))?;
            let mut head_buf = vec![0u8; head_n];
            let head_read = reader.read(&mut head_buf)?;
            head_buf.truncate(head_read);
            
            // Read tail
            let len = reader.len()?;
            let tail_start = len.saturating_sub(tail_m as u64);
            reader.seek(SeekFrom::Start(tail_start))?;
            let mut tail_buf = vec![0u8; tail_m];
            let tail_read = reader.read(&mut tail_buf)?;
            tail_buf.truncate(tail_read);
            
            // Combine and hash
            head_buf.extend_from_slice(&tail_buf);
            Ok(crate::media::crc32_bytes(&head_buf))
        }
        CrcStrategy::Full => compute_full(reader),
        CrcStrategy::Range(start, len) => compute_range(reader, start, len),
        CrcStrategy::Custom(func) => func(reader),
    }
}

/// Checksum strategy implementations.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum CrcStrategy {
    /// Read first N bytes from start of file.
    Head(usize),
    /// Read last N bytes from end of file.
    Tail(usize),
    /// Read first N bytes from start AND last M bytes from end.
    /// Total buffer size is N + M (128KB max recommended for performance).
    HeadTail(usize, usize),
    /// Read the entire file.
    Full,
    /// Read a specific byte range.
    Range(usize, usize),
    /// Custom strategy for complex formats.
    Custom(fn(&mut dyn MediaReader) -> Result<u32>),
}

impl CrcStrategy {
    /// Get the default strategy for a media format.
    /// 
    /// CRC strategy is format-specific to handle different metadata locations:
    /// - JPEG: Head (64KB) - image data starts early, metadata at start
    /// - PNG: Tail (64KB) - image data at end, metadata at start (can be modified)
    /// - MP4/MOV: Head + Tail (128KB total) - moov atom at end, mdat at start
    /// - RAW: Full file - these are precious, use entire content
    pub fn for_format(format: &MediaFormat) -> Self {
        use MediaFormat::*;

        match format {
            // Image formats: use head (64KB) - image data starts early
            Jpeg => CrcStrategy::Head(CHECKSUM_BUFFER_SIZE),
            Heif | Heic | Avif | Webp => CrcStrategy::Head(CHECKSUM_BUFFER_SIZE),

            // PNG: use tail (64KB) - image data at end, metadata at start
            // Metadata (text chunks) can be modified without changing image
            Png => CrcStrategy::Tail(CHECKSUM_BUFFER_SIZE),

            // Video formats: head + tail (128KB = 64KB head + 64KB tail)
            // MP4/MOV: moov atom (metadata) often at end, mdat (media) at start
            Mov | Mp4 => CrcStrategy::HeadTail(CHECKSUM_BUFFER_SIZE, CHECKSUM_BUFFER_SIZE),
            Avi | Mkv => CrcStrategy::Head(CHECKSUM_BUFFER_SIZE),

            // RAW formats: head + tail (128KB total)
            // RAW files are large; head+tail captures embedded JPEG preview and metadata
            Dng | Arw | Cr2 | Cr3 | Nef | Raf | Rw2 => CrcStrategy::HeadTail(CHECKSUM_BUFFER_SIZE, CHECKSUM_BUFFER_SIZE),

            // Unknown: full file to be safe
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
    let start = len.saturating_sub(n as u64);

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
        let expected = crate::media::crc32_bytes(&data[..CHECKSUM_BUFFER_SIZE]);

        let mut cursor = Cursor::new(data);
        let result = compute_head(&mut cursor, CHECKSUM_BUFFER_SIZE).unwrap();

        assert_eq!(result, expected);
    }

    #[test]
    fn test_tail_checksum() {
        let data = test_data();
        let expected = crate::media::crc32_bytes(&data[data.len() - CHECKSUM_BUFFER_SIZE..]);

        let mut cursor = Cursor::new(data);
        let result = compute_tail(&mut cursor, CHECKSUM_BUFFER_SIZE).unwrap();

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
