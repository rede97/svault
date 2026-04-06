//! Stage B: CRC32C fingerprint computation.

use std::path::{Path, PathBuf};

use indicatif::ProgressBar;
use rayon::prelude::*;

use crate::media::crc::compute_checksum;
use crate::media::raw_id::{extract_raw_id_if_raw, get_fingerprint_string, is_raw_file};
use crate::media::MediaFormat;
use crate::pipeline::types::{CrcEntry, FileEntry};

/// Result of CRC computation for a single file.
#[derive(Debug, Clone)]
pub struct CrcResult {
    pub file: FileEntry,
    pub crc: Result<u32, String>,
    pub raw_unique_id: Option<String>,
}

/// Compute CRC32C for all entries in parallel.
///
/// # Arguments
/// * `entries` - File entries from Stage A
/// * `source_root` - Root directory (for constructing full paths)
/// * `progress` - Optional progress bar
///
/// # Returns
/// List of CRC results (preserving order)
pub fn compute_crcs(
    entries: Vec<FileEntry>,
    source_root: &Path,
    progress: Option<&ProgressBar>,
) -> Vec<CrcResult> {
    entries
        .into_par_iter()
        .map(|e| {
            let abs_path = source_root.join(&e.path);
            
            // Compute format-specific CRC32C
            let format = MediaFormat::from_path(&abs_path)
                .unwrap_or(MediaFormat::Unknown(""));
            let crc = compute_checksum(&abs_path, &format)
                .map_err(|err| err.to_string());

            // Extract RAW ID for RAW files
            let ext = abs_path
                .extension()
                .and_then(|ex| ex.to_str())
                .unwrap_or("");
            let raw_unique_id = if is_raw_file(ext) {
                extract_raw_id_if_raw(&abs_path)
                    .and_then(|raw_id| get_fingerprint_string(&raw_id))
            } else {
                None
            };

            if let Some(pb) = progress {
                pb.inc(1);
            }

            CrcResult {
                file: e,
                crc,
                raw_unique_id,
            }
        })
        .collect()
}

/// Convert CrcResult to CrcEntry (filtering out errors).
pub fn filter_successful(results: Vec<CrcResult>) -> (Vec<CrcEntry>, Vec<(PathBuf, String)>) {
    let mut entries = Vec::new();
    let mut errors = Vec::new();

    for r in results {
        match r.crc {
            Ok(crc) => {
                entries.push(CrcEntry {
                    file: r.file,
                    crc32c: crc,
                    raw_unique_id: r.raw_unique_id,
                });
            }
            Err(e) => {
                errors.push((r.file.path, e));
            }
        }
    }

    (entries, errors)
}
