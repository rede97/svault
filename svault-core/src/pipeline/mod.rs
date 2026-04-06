//! Import/Add shared pipeline stages.
//!
//! This module provides reusable pipeline stages for both `import` and `add` commands:
//!
//! ```text
//! Stage A (scan):    Scan directory -> Vec<FileEntry>
//! Stage B (crc):     Compute CRC32C -> Vec<CrcEntry>  
//! Lookup:            DB duplicate check -> Vec<LookupResult>
//! Stage C (copy):    Copy files (import only) -> Vec<CopyResult>
//! Stage D (hash):    Strong hash verification -> Vec<HashResult>
//! Stage E (insert):  DB batch insert -> PipelineSummary
//! ```

pub mod types;
pub mod scan;
pub mod crc;
pub mod lookup;
pub mod hash;
pub mod insert;

pub use types::{
    CopyResult, CrcEntry, FileEntry, FileStatus, HashResult, LookupResult,
    PipelineSummary,
};
