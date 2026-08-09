//! Import/Add shared pipeline stages.
//!
//! This module provides reusable pipeline stages for both `import` and `add` commands:
//!
//! ```text
//! Stage A (scan):    Scan directory -> Vec<FileEntry>
//! Stage B (fingerprint): XXH3-128 head/tail fingerprint -> Vec<FingerprintEntry>
//! Lookup:            DB duplicate check -> Vec<LookupResult>
//! Stage C (copy):    Copy files (import only) -> Vec<CopyResult>
//! Stage D (hash):    Strong hash verification -> Vec<HashResult>
//! Stage E (insert):  DB batch insert -> PipelineSummary
//! ```

pub mod fingerprint;
pub mod hash;
pub mod insert;
pub mod lookup;
pub mod scan;
pub mod types;

pub use types::{
    CheckResult, CopyResult, FileEntry, FileStatus, FingerprintEntry, HashResult, LookupResult,
    PipelineSummary,
};
