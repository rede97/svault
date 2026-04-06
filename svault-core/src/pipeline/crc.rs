//! Stage B: CRC32C fingerprint computation.

use std::path::PathBuf;

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
/// * `entries` - File entries from Stage A (paths must be absolute)
/// * `progress` - Optional progress bar
///
/// # Returns
/// List of CRC results (preserving order)
pub fn compute_crcs(entries: Vec<FileEntry>, progress: Option<&ProgressBar>) -> Vec<CrcResult> {
    entries
        .into_par_iter()
        .map(|e| {
            // Compute format-specific CRC32C
            let format = MediaFormat::from_path(&e.path)
                .unwrap_or(MediaFormat::Unknown(""));
            let crc = compute_checksum(&e.path, &format)
                .map_err(|err| err.to_string());

            // Extract RAW ID for RAW files
            let ext = e.path
                .extension()
                .and_then(|ex| ex.to_str())
                .unwrap_or("");
            let raw_unique_id = if is_raw_file(ext) {
                extract_raw_id_if_raw(&e.path)
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

/// Split CRC results into successful entries and errors.
///
/// # Returns
/// (successful_entries, error_entries)
pub fn split_results(results: Vec<CrcResult>) -> (Vec<CrcEntry>, Vec<CrcResult>) {
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
            Err(_) => {
                errors.push(r);
            }
        }
    }

    (entries, errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_compute_crcs_basic() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.txt");
        fs::write(&test_file, "hello world").unwrap();

        let entries = vec![FileEntry {
            path: test_file.clone(),
            size: 11,
            mtime_ms: 0,
        }];

        let results = compute_crcs(entries, None);
        
        assert_eq!(results.len(), 1);
        assert!(results[0].crc.is_ok());
    }

    #[test]
    fn test_compute_crcs_missing_file() {
        let entries = vec![FileEntry {
            path: PathBuf::from("/nonexistent/file.txt"),
            size: 0,
            mtime_ms: 0,
        }];

        let results = compute_crcs(entries, None);
        
        assert_eq!(results.len(), 1);
        assert!(results[0].crc.is_err());
    }

    #[test]
    fn test_split_results() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.txt");
        fs::write(&test_file, "hello").unwrap();

        let results = vec![
            CrcResult {
                file: FileEntry { path: test_file, size: 5, mtime_ms: 0 },
                crc: Ok(12345),
                raw_unique_id: None,
            },
            CrcResult {
                file: FileEntry { path: PathBuf::from("/missing"), size: 0, mtime_ms: 0 },
                crc: Err("not found".to_string()),
                raw_unique_id: None,
            },
        ];

        let (success, errors) = split_results(results);
        
        assert_eq!(success.len(), 1);
        assert_eq!(errors.len(), 1);
        assert_eq!(success[0].crc32c, 12345);
    }
}
