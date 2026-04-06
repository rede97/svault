//! Stage B: CRC32C fingerprint computation.

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use indicatif::ProgressBar;
use rayon::prelude::*;

use crate::media::crc::compute_checksum;
use crate::media::raw_id::{extract_raw_id_if_raw, get_fingerprint_string, is_raw_file};
use crate::media::MediaFormat;
use crate::pipeline::types::{CrcEntry, FileEntry};

/// Batch size for parallel CRC computation.
/// Chosen to balance memory usage and parallelism.
const CRC_BATCH_SIZE: usize = 100;

/// Result of CRC computation for a single file.
#[derive(Debug, Clone)]
pub struct CrcResult {
    pub file: FileEntry,
    pub crc: Result<u32, String>,
    pub raw_unique_id: Option<String>,
}

/// Compute CRC32C from a stream of file entries with batching + parallel processing.
///
/// This function receives entries via channel, processes them in batches using
/// Rayon for parallel CRC computation, and streams results back via channel.
///
/// # Arguments
/// * `rx` - Input stream of FileEntry results (from scan_stream)
/// * `progress` - Optional progress bar
///
/// # Returns
/// Receiver that yields CrcResult as they are computed.
///
/// # Implementation
/// - Receives entries from input channel in batches (100 entries)
/// - Processes each batch in parallel using Rayon
/// - Streams results back via output channel
/// - Handles errors gracefully (error entries are still yielded)
pub fn compute_crcs_stream(
    rx: mpsc::Receiver<anyhow::Result<FileEntry>>,
    progress: Option<ProgressBar>,
) -> mpsc::Receiver<CrcResult> {
    let (tx, output_rx) = mpsc::channel();

    thread::spawn(move || {
        let mut batch = Vec::with_capacity(CRC_BATCH_SIZE);

        for entry_result in rx {
            match entry_result {
                Ok(entry) => {
                    batch.push(entry);

                    // Process batch when full
                    if batch.len() >= CRC_BATCH_SIZE {
                        process_crc_batch(&mut batch, &tx, &progress);
                    }
                }
                Err(e) => {
                    // Forward error as a failed CrcResult
                    let error_result = CrcResult {
                        file: FileEntry {
                            path: PathBuf::from("<error>"),
                            size: 0,
                            mtime_ms: 0,
                        },
                        crc: Err(e.to_string()),
                        raw_unique_id: None,
                    };
                    if tx.send(error_result).is_err() {
                        return; // Receiver dropped
                    }
                }
            }
        }

        // Process remaining entries
        if !batch.is_empty() {
            process_crc_batch(&mut batch, &tx, &progress);
        }
    });

    output_rx
}

/// Process a batch of entries in parallel.
fn process_crc_batch(
    batch: &mut Vec<FileEntry>,
    tx: &mpsc::Sender<CrcResult>,
    progress: &Option<ProgressBar>,
) {
    let results: Vec<CrcResult> = batch
        .par_drain(..)
        .map(|e| compute_crc_for_entry(&e, progress))
        .collect();

    for result in results {
        if tx.send(result).is_err() {
            break; // Receiver dropped
        }
    }
}

/// Compute CRC for a single file entry.
fn compute_crc_for_entry(e: &FileEntry, progress: &Option<ProgressBar>) -> CrcResult {
    // Compute format-specific CRC32C
    let format = MediaFormat::from_path(&e.path).unwrap_or(MediaFormat::Unknown(""));
    let crc = compute_checksum(&e.path, &format).map_err(|err| err.to_string());

    // Extract RAW ID for RAW files
    let ext = e
        .path
        .extension()
        .and_then(|ex| ex.to_str())
        .unwrap_or("");
    let raw_unique_id = if is_raw_file(ext) {
        extract_raw_id_if_raw(&e.path).and_then(|raw_id| get_fingerprint_string(&raw_id))
    } else {
        None
    };

    if let Some(pb) = progress {
        pb.inc(1);
    }

    CrcResult {
        file: e.clone(),
        crc,
        raw_unique_id,
    }
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
                    src_path: None, // Path is already the source path
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

    // =========================================================================
    // compute_crcs_stream tests
    // =========================================================================

    #[test]
    fn test_compute_crcs_stream_basic() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.txt");
        fs::write(&test_file, "hello world").unwrap();

        let (tx, rx) = mpsc::channel();
        tx.send(Ok(FileEntry {
            path: test_file.clone(),
            size: 11,
            mtime_ms: 0,
        })).unwrap();
        drop(tx);

        let results: Vec<_> = compute_crcs_stream(rx, None).into_iter().collect();
        
        assert_eq!(results.len(), 1);
        assert!(results[0].crc.is_ok());
    }

    #[test]
    fn test_compute_crcs_stream_multiple_files() {
        let tmp = TempDir::new().unwrap();

        // Create test files
        let files: Vec<_> = (0..10).map(|i| {
            let path = tmp.path().join(format!("test{}.txt", i));
            fs::write(&path, format!("content {}", i)).unwrap();
            path
        }).collect();

        let (tx, rx) = mpsc::channel();
        for path in &files {
            tx.send(Ok(FileEntry {
                path: path.clone(),
                size: 10,
                mtime_ms: 0,
            })).unwrap();
        }
        drop(tx);

        let results: Vec<_> = compute_crcs_stream(rx, None).into_iter().collect();
        
        assert_eq!(results.len(), 10);
        // All should succeed
        assert!(results.iter().all(|r| r.crc.is_ok()));
    }

    #[test]
    fn test_compute_crcs_stream_batch_processing() {
        let tmp = TempDir::new().unwrap();

        // Create more files than batch size to test batching
        let num_files = CRC_BATCH_SIZE + 50;
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            for i in 0..num_files {
                let path = tmp.path().join(format!("file{}.txt", i));
                fs::write(&path, format!("content {}", i)).unwrap();
                tx.send(Ok(FileEntry {
                    path,
                    size: 10,
                    mtime_ms: 0,
                })).unwrap();
            }
        });

        let results: Vec<_> = compute_crcs_stream(rx, None).into_iter().collect();
        
        assert_eq!(results.len(), num_files);
    }

    #[test]
    fn test_compute_crcs_stream_with_errors() {
        let tmp = TempDir::new().unwrap();
        let valid_file = tmp.path().join("valid.txt");
        fs::write(&valid_file, "hello").unwrap();

        let (tx, rx) = mpsc::channel();
        tx.send(Ok(FileEntry {
            path: valid_file,
            size: 5,
            mtime_ms: 0,
        })).unwrap();
        tx.send(Err(anyhow::anyhow!("test error"))).unwrap();
        drop(tx);

        let results: Vec<_> = compute_crcs_stream(rx, None).into_iter().collect();
        
        assert_eq!(results.len(), 2);
        // One should be ok (valid file), one should be error
        let ok_count = results.iter().filter(|r| r.crc.is_ok()).count();
        let err_count = results.iter().filter(|r| r.crc.is_err()).count();
        assert_eq!(ok_count, 1, "Expected 1 ok result");
        assert_eq!(err_count, 1, "Expected 1 error result");
    }
}
