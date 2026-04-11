//! Hash utilities used throughout Svault.
//!
//! - **CRC32C**: Hardware-accelerated (SSE4.2 / ARM CRC32). Used for the
//!   64 KB fingerprint probe in Stage 3 of the comparison pipeline.
//! - **XXH3-128**: High-throughput non-cryptographic hash. Used for fast
//!   full-file integrity verification (`svault verify --fast`).
//! - **SHA-256**: Cryptographic hash. The permanent content identity of every
//!   file. Computed lazily — only when a Stage 3 fingerprint collision occurs
//!   or during background-hash.

use std::{
    fs,
    io::{self, Read},
    path::Path,
};

// ---------------------------------------------------------------------------
// CRC32C
// ---------------------------------------------------------------------------

/// Computes CRC32C over `data`.
/// Uses the `crc32c` crate which selects SSE4.2 / ARM CRC32 at runtime.
pub fn crc32c(data: &[u8]) -> u32 {
    crc32c::crc32c(data)
}

/// Reads up to `max_bytes` from `path` starting at `offset` and computes
/// CRC32C over that region. Used for the Stage 3 fingerprint probe.
pub fn crc32c_region(path: &Path, offset: u64, max_bytes: usize) -> io::Result<u32> {
    let mut f = fs::File::open(path)?;
    if offset > 0 {
        io::Seek::seek(&mut f, io::SeekFrom::Start(offset))?;
    }
    let mut buf = vec![0u8; max_bytes];
    let n = f.read(&mut buf)?;
    Ok(crc32c::crc32c(&buf[..n]))
}

/// Reads the last `max_bytes` of `path` and computes CRC32C.
/// Used for formats like PNG where metadata lives at the end of the file.
pub fn crc32c_tail(path: &Path, max_bytes: usize) -> io::Result<u32> {
    let meta = fs::metadata(path)?;
    let size = meta.len();
    let offset = size.saturating_sub(max_bytes as u64);
    crc32c_region(path, offset, max_bytes)
}

// ---------------------------------------------------------------------------
// XXH3-128
// ---------------------------------------------------------------------------

/// 128-bit XXH3 digest represented as two u64 values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Xxh3Digest {
    pub low: u64,
    pub high: u64,
}

impl Xxh3Digest {
    /// Returns the digest as a 16-byte little-endian array.
    pub fn to_bytes(self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&self.low.to_le_bytes());
        b[8..].copy_from_slice(&self.high.to_le_bytes());
        b
    }
}

impl std::fmt::LowerHex for Xxh3Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}{:016x}", self.high, self.low)
    }
}

/// Computes XXH3-128 over the entire file at `path`.
/// Suitable for fast full-file integrity checks (`svault verify --fast`).
pub fn xxh3_128_file(path: &Path) -> io::Result<Xxh3Digest> {
    use xxhash_rust::xxh3::Xxh3;
    const BUF: usize = 4 * 1024 * 1024; // 4 MB chunks
    let mut f = fs::File::open(path)?;
    let mut hasher = Xxh3::new();
    let mut buf = vec![0u8; BUF];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.digest128();
    Ok(Xxh3Digest {
        low: digest as u64,
        high: (digest >> 64) as u64,
    })
}

// ---------------------------------------------------------------------------
// SHA-256
// ---------------------------------------------------------------------------

/// SHA-256 digest as a 32-byte array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    /// Returns the raw 32-byte digest.
    pub fn to_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns the digest as a lowercase hex string (64 characters).
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

impl std::fmt::Display for Sha256Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// Computes SHA-256 over the entire file at `path`.
/// Reads in 4 MB chunks to avoid excessive memory usage on large RAW files.
pub fn sha256_file(path: &Path) -> io::Result<Sha256Digest> {
    use sha2::{Digest, Sha256};
    const BUF: usize = 4 * 1024 * 1024;
    let mut f = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; BUF];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let result = hasher.finalize();
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&result);
    Ok(Sha256Digest(digest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_file_with(content: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(content).unwrap();
        drop(file);
        (dir, path)
    }

    // -------------------------------------------------------------------------
    // CRC32C: Focus on our wrapper logic, not the library
    // -------------------------------------------------------------------------

    #[test]
    fn crc32c_region_reads_from_offset() {
        let content = b"0123456789abcdef";
        let (_dir, path) = temp_file_with(content);

        // Offset 10 should read "abcdef"
        let hash = crc32c_region(&path, 10, 1024).unwrap();
        assert_eq!(hash, crc32c(b"abcdef"));
    }

    #[test]
    fn crc32c_region_handles_larger_buf_than_file() {
        let (_dir, path) = temp_file_with(b"short");
        // Request more than file size
        let hash = crc32c_region(&path, 0, 1024).unwrap();
        assert_eq!(hash, crc32c(b"short"));
    }

    #[test]
    fn crc32c_region_zero_offset_reads_full() {
        let (_dir, path) = temp_file_with(b"hello");
        let hash = crc32c_region(&path, 0, 1024).unwrap();
        assert_eq!(hash, crc32c(b"hello"));
    }

    #[test]
    fn crc32c_tail_reads_last_bytes() {
        let content = b"0123456789abcdef";
        let (_dir, path) = temp_file_with(content);

        // Last 6 bytes: "abcdef"
        let hash = crc32c_tail(&path, 6).unwrap();
        assert_eq!(hash, crc32c(b"abcdef"));
    }

    #[test]
    fn crc32c_tail_handles_larger_buf_than_file() {
        let (_dir, path) = temp_file_with(b"tiny");
        let hash = crc32c_tail(&path, 1024).unwrap();
        assert_eq!(hash, crc32c(b"tiny"));
    }

    #[test]
    fn crc32c_region_returns_io_error_for_missing_file() {
        let result = crc32c_region(Path::new("/nonexistent/file"), 0, 1024);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn crc32c_tail_returns_io_error_for_missing_file() {
        let result = crc32c_tail(Path::new("/nonexistent/file"), 1024);
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // XXH3-128: Focus on file I/O, chunking, and our type wrapper
    // -------------------------------------------------------------------------

    #[test]
    fn xxh3_128_file_is_deterministic() {
        let (_dir, path) = temp_file_with(b"test content");
        let d1 = xxh3_128_file(&path).unwrap();
        let d2 = xxh3_128_file(&path).unwrap();
        assert_eq!(d1, d2);
    }

    #[test]
    fn xxh3_128_file_produces_different_hashes_for_different_content() {
        let (_dir1, p1) = temp_file_with(b"content A");
        let (_dir2, p2) = temp_file_with(b"content B");
        let d1 = xxh3_128_file(&p1).unwrap();
        let d2 = xxh3_128_file(&p2).unwrap();
        assert_ne!(d1, d2);
    }

    #[test]
    fn xxh3_128_file_handles_empty_file() {
        let (_dir, path) = temp_file_with(b"");
        let _ = xxh3_128_file(&path).unwrap(); // Should not panic
    }

    #[test]
    fn xxh3_128_file_handles_large_file() {
        // 10MB file to test chunking
        let data = vec![0xABu8; 10 * 1024 * 1024];
        let (_dir, path) = temp_file_with(&data);
        let d1 = xxh3_128_file(&path).unwrap();
        let d2 = xxh3_128_file(&path).unwrap();
        assert_eq!(d1, d2);
    }

    #[test]
    fn xxh3_128_file_returns_io_error_for_missing_file() {
        let result = xxh3_128_file(Path::new("/nonexistent/file"));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn xxh3_digest_to_bytes_little_endian() {
        let digest = Xxh3Digest {
            low: 0x1234_5678_9ABC_DEF0,
            high: 0xFEDC_BA98_7654_3210,
        };
        let bytes = digest.to_bytes();
        // Verify little-endian encoding
        assert_eq!(
            u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            digest.low
        );
        assert_eq!(
            u64::from_le_bytes(bytes[8..].try_into().unwrap()),
            digest.high
        );
    }

    #[test]
    fn xxh3_digest_hex_formatting() {
        let digest = Xxh3Digest { low: 1, high: 2 };
        let hex_str = format!("{:x}", digest);
        assert_eq!(hex_str.len(), 32); // 128 bits = 32 hex chars
        // Verify format: high first, then low
        assert!(hex_str.starts_with("0000000000000002"));
        assert!(hex_str.ends_with("0000000000000001"));
    }

    // -------------------------------------------------------------------------
    // SHA-256: Focus on file I/O, chunking, and our type wrapper
    // -------------------------------------------------------------------------

    #[test]
    fn sha256_file_is_deterministic() {
        let (_dir, path) = temp_file_with(b"test content");
        let d1 = sha256_file(&path).unwrap();
        let d2 = sha256_file(&path).unwrap();
        assert_eq!(d1, d2);
    }

    #[test]
    fn sha256_file_produces_different_hashes_for_different_content() {
        let (_dir1, p1) = temp_file_with(b"content A");
        let (_dir2, p2) = temp_file_with(b"content B");
        let d1 = sha256_file(&p1).unwrap();
        let d2 = sha256_file(&p2).unwrap();
        assert_ne!(d1, d2);
    }

    #[test]
    fn sha256_file_handles_empty_file() {
        let (_dir, path) = temp_file_with(b"");
        let _ = sha256_file(&path).unwrap(); // Should not panic
    }

    #[test]
    fn sha256_file_handles_large_file() {
        // 10MB file to test chunking
        let data = vec![0xCDu8; 10 * 1024 * 1024];
        let (_dir, path) = temp_file_with(&data);
        let d1 = sha256_file(&path).unwrap();
        let d2 = sha256_file(&path).unwrap();
        assert_eq!(d1, d2);
    }

    #[test]
    fn sha256_file_returns_io_error_for_missing_file() {
        let result = sha256_file(Path::new("/nonexistent/file"));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn sha256_digest_to_hex_format() {
        let digest = Sha256Digest([0xAB; 32]);
        let hex_str = digest.to_hex();
        assert_eq!(hex_str.len(), 64);
        assert_eq!(hex_str, "ab".repeat(32));
    }

    #[test]
    fn sha256_digest_display_trait() {
        let (_dir, path) = temp_file_with(b"x");
        let digest = sha256_file(&path).unwrap();
        let display = format!("{}", digest);
        assert_eq!(display, digest.to_hex());
        assert_eq!(display.len(), 64);
    }

    #[test]
    fn sha256_digest_to_bytes_returns_inner_array() {
        let (_dir, path) = temp_file_with(b"test");
        let digest = sha256_file(&path).unwrap();
        let bytes = digest.to_bytes();
        assert_eq!(bytes.len(), 32);
    }
}
