//! Stage B: XXH3-128 fingerprint computation (head/tail regions).

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use rayon::prelude::*;

use crate::media::MediaFormat;
use crate::media::fingerprint::compute_fingerprint;
use crate::media::raw_id::{extract_raw_id_if_raw, get_fingerprint_string, is_raw_file};
use crate::pipeline::types::{FileEntry, FingerprintEntry};

/// Batch size for parallel CRC computation.
/// Chosen to balance memory usage and parallelism.
const FINGERPRINT_BATCH_SIZE: usize = 100;

/// Result of CRC computation for a single file.
#[derive(Debug, Clone)]
pub struct FingerprintResult {
    pub file: FileEntry,
    pub fingerprint: Result<Vec<u8>, String>,
    pub raw_unique_id: Option<String>,
}

/// Compute CRC32C from a stream of file entries with batching + parallel processing.
///
/// This function receives entries via channel, processes them in batches using
/// Rayon for parallel CRC computation, and streams results back via channel.
///
/// # Arguments
/// * `rx` - Input stream of FileEntry results (from scan_stream)
///
/// # Returns
/// Receiver that yields FingerprintResult as they are computed.
///
/// # Implementation
/// - Receives entries from input channel in batches (100 entries)
/// - Processes each batch in parallel using Rayon
/// - Streams results back via output channel
/// - Handles errors gracefully (error entries are still yielded)
pub fn compute_fingerprints_stream(
    rx: mpsc::Receiver<anyhow::Result<FileEntry>>,
) -> mpsc::Receiver<FingerprintResult> {
    let (tx, output_rx) = mpsc::channel();

    thread::spawn(move || {
        let mut batch = Vec::with_capacity(FINGERPRINT_BATCH_SIZE);

        for entry_result in rx {
            match entry_result {
                Ok(entry) => {
                    batch.push(entry);

                    // Process batch when full
                    if batch.len() >= FINGERPRINT_BATCH_SIZE {
                        process_fingerprint_batch(&mut batch, &tx);
                    }
                }
                Err(e) => {
                    // Forward error as a failed FingerprintResult
                    let error_result = FingerprintResult {
                        file: FileEntry {
                            path: PathBuf::from("<error>"),
                            size: 0,
                            mtime_ms: 0,
                        },
                        fingerprint: Err(e.to_string()),
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
            process_fingerprint_batch(&mut batch, &tx);
        }
    });

    output_rx
}

/// Process a batch of entries in parallel.
fn process_fingerprint_batch(batch: &mut Vec<FileEntry>, tx: &mpsc::Sender<FingerprintResult>) {
    let results: Vec<FingerprintResult> = batch
        .par_drain(..)
        .map(|e| compute_fingerprint_for_entry(&e))
        .collect();

    for result in results {
        if tx.send(result).is_err() {
            break; // Receiver dropped
        }
    }
}

/// Compute CRC for a single file entry.
fn compute_fingerprint_for_entry(e: &FileEntry) -> FingerprintResult {
    // Compute format-specific CRC32C
    let format = MediaFormat::from_path(&e.path).unwrap_or(MediaFormat::Unknown(""));
    let fingerprint = compute_fingerprint(&e.path, &format)
        .map(|d| d.to_vec())
        .map_err(|err| err.to_string());

    // Extract RAW ID for RAW files
    let ext = e.path.extension().and_then(|ex| ex.to_str()).unwrap_or("");
    let raw_unique_id = if is_raw_file(ext) {
        extract_raw_id_if_raw(&e.path).and_then(|raw_id| get_fingerprint_string(&raw_id))
    } else {
        None
    };

    FingerprintResult {
        file: e.clone(),
        fingerprint,
        raw_unique_id,
    }
}

/// Split CRC results into successful entries and errors.
///
/// # Returns
/// (successful_entries, error_entries)
pub fn split_results(
    results: Vec<FingerprintResult>,
) -> (Vec<FingerprintEntry>, Vec<FingerprintResult>) {
    let mut entries = Vec::new();
    let mut errors = Vec::new();

    for r in results {
        match r.fingerprint {
            Ok(fingerprint) => {
                entries.push(FingerprintEntry {
                    file: r.file,
                    src_path: None, // Path is already the source path
                    staged_path: None,
                    fingerprint,
                    raw_unique_id: r.raw_unique_id,
                    precomputed_hash: None,
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
            FingerprintResult {
                file: FileEntry {
                    path: test_file,
                    size: 5,
                    mtime_ms: 0,
                },
                fingerprint: Ok(vec![1, 2, 3]),
                raw_unique_id: None,
            },
            FingerprintResult {
                file: FileEntry {
                    path: PathBuf::from("/missing"),
                    size: 0,
                    mtime_ms: 0,
                },
                fingerprint: Err("not found".to_string()),
                raw_unique_id: None,
            },
        ];

        let (success, errors) = split_results(results);

        assert_eq!(success.len(), 1);
        assert_eq!(errors.len(), 1);
        assert_eq!(success[0].fingerprint, vec![1, 2, 3]);
    }

    // =========================================================================
    // compute_fingerprints_stream tests
    // =========================================================================

    #[test]
    fn test_compute_fingerprints_stream_basic() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.txt");
        fs::write(&test_file, "hello world").unwrap();

        let (tx, rx) = mpsc::channel();
        tx.send(Ok(FileEntry {
            path: test_file.clone(),
            size: 11,
            mtime_ms: 0,
        }))
        .unwrap();
        drop(tx);

        let results: Vec<_> = compute_fingerprints_stream(rx).into_iter().collect();

        assert_eq!(results.len(), 1);
        assert!(results[0].fingerprint.is_ok());
    }

    #[test]
    fn test_compute_fingerprints_stream_multiple_files() {
        let tmp = TempDir::new().unwrap();

        // Create test files
        let files: Vec<_> = (0..10)
            .map(|i| {
                let path = tmp.path().join(format!("test{}.txt", i));
                fs::write(&path, format!("content {}", i)).unwrap();
                path
            })
            .collect();

        let (tx, rx) = mpsc::channel();
        for path in &files {
            tx.send(Ok(FileEntry {
                path: path.clone(),
                size: 10,
                mtime_ms: 0,
            }))
            .unwrap();
        }
        drop(tx);

        let results: Vec<_> = compute_fingerprints_stream(rx).into_iter().collect();

        assert_eq!(results.len(), 10);
        // All should succeed
        assert!(results.iter().all(|r| r.fingerprint.is_ok()));
    }

    #[test]
    fn test_compute_fingerprints_stream_batch_processing() {
        let tmp = TempDir::new().unwrap();

        // Create more files than batch size to test batching
        let num_files = FINGERPRINT_BATCH_SIZE + 50;
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            for i in 0..num_files {
                let path = tmp.path().join(format!("file{}.txt", i));
                fs::write(&path, format!("content {}", i)).unwrap();
                tx.send(Ok(FileEntry {
                    path,
                    size: 10,
                    mtime_ms: 0,
                }))
                .unwrap();
            }
        });

        let results: Vec<_> = compute_fingerprints_stream(rx).into_iter().collect();

        assert_eq!(results.len(), num_files);
    }

    #[test]
    fn test_compute_fingerprints_stream_with_errors() {
        let tmp = TempDir::new().unwrap();
        let valid_file = tmp.path().join("valid.txt");
        fs::write(&valid_file, "hello").unwrap();

        let (tx, rx) = mpsc::channel();
        tx.send(Ok(FileEntry {
            path: valid_file,
            size: 5,
            mtime_ms: 0,
        }))
        .unwrap();
        tx.send(Err(anyhow::anyhow!("test error"))).unwrap();
        drop(tx);

        let results: Vec<_> = compute_fingerprints_stream(rx).into_iter().collect();

        assert_eq!(results.len(), 2);
        // One should be ok (valid file), one should be error
        let ok_count = results.iter().filter(|r| r.fingerprint.is_ok()).count();
        let err_count = results.iter().filter(|r| r.fingerprint.is_err()).count();
        assert_eq!(ok_count, 1, "Expected 1 ok result");
        assert_eq!(err_count, 1, "Expected 1 error result");
    }
}
