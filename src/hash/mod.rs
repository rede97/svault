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
