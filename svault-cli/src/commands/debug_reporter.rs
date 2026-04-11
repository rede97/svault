//! Debug command to test reporter output.
//!
//! Simulates a full import workflow to verify progress bars and output formatting.
//!
//! # Multi-threaded Copy Phase (using Rayon)
//!
//! The Copy phase uses Rayon parallel iterators to demonstrate real concurrent
//! file operations. You'll see the active file list update dynamically as
//! multiple threads start and finish:
//!
//! ```text
//! Copying [===>          ] 3/10 (30%): photo_0002.jpg, photo_0003.jpg, +1 more
//! ```
//!
//! # Multi-threaded Hash Phase (using Rayon)
//!
//! The Hash phase also uses Rayon parallel processing to simulate concurrent
//! hash computation.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use rayon::prelude::*;

use crate::cli::OutputFormat;
use crate::reporting::TerminalReporterBuilder;
use svault_core::reporting::{
    CopyReporter, HashReporter, InsertReporter, ItemStatus, ReporterBuilder, ScanReporter,
};

/// Run reporter test simulation.
pub fn run(
    _output: OutputFormat,
    count: usize,
    delay_ms: u64,
    show_dup: bool,
) -> anyhow::Result<()> {
    let builder = Arc::new(TerminalReporterBuilder::new());
    let delay = Duration::from_millis(delay_ms);
    let source = PathBuf::from("/source");

    // ── Phase: Scan ───────────────────────────────────────────────────────────
    {
        let reporter = builder.scan_reporter(&source);
        thread::sleep(delay * 5);

        for i in 0..count {
            let path = PathBuf::from(format!("/source/photo_{:04}.jpg", i + 1));

            reporter.discovered(
                &path,
                1024 * 1024 + (i as u64 * 1000),
                1234567890000 + (i as i64 * 1000),
            );

            let status = if show_dup && i % 3 == 0 {
                ItemStatus::Duplicate
            } else {
                ItemStatus::New
            };
            let size = 1024 * 1024 + (i as u64 * 1000);
            reporter.classified(&path, size, status, None);
            reporter.progress((i + 1) as u64);

            thread::sleep(delay * 20 / (count as u32));
        }

        let new_count = if show_dup { count - (count / 3) } else { count };
        reporter.preflight(count, new_count, count - new_count, 0, 0, &source);
        reporter.finish();
        thread::sleep(delay);
    }

    let new_count = if show_dup { count - (count / 3) } else { count };

    // ── Phase: Copy ───────────────────────────────────────────────────────────
    // Demonstrates multi-threaded concurrent copy using Rayon.
    let vault_root = PathBuf::from("/vault");
    if new_count > 0 {
        let reporter = Arc::new(builder.copy_reporter(&source, &vault_root, new_count as u64));

        // Use Rayon parallel iterator for concurrent processing
        (0..new_count).into_par_iter().for_each(|i| {
            let src_path = PathBuf::from(format!("/source/photo_{:04}.jpg", i + 1));
            let dest_path = PathBuf::from(format!("/vault/2024/photo_{:04}.jpg", i + 1));

            let size = 1024 * 1024; // 1 MiB

            // Signal that this file is being copied
            reporter.item_started(&src_path, &dest_path, size);

            // Simulate variable copy time (some files take longer)
            let work_time = if i % 3 == 0 {
                delay * 2 // Every 3rd file takes longer
            } else {
                delay
            };
            thread::sleep(work_time);

            // Update progress and signal completion
            reporter.item_finished(&src_path, &dest_path, size);
        });

        reporter.finish();
        thread::sleep(delay);
    }

    // ── Phase: Hash ───────────────────────────────────────────────────────────
    // Demonstrates multi-threaded concurrent hash using Rayon.
    if new_count > 0 {
        let reporter = Arc::new(builder.hash_reporter(&source, new_count as u64));

        // Use Rayon parallel iterator for concurrent hash computation
        (0..new_count).into_par_iter().for_each(|i| {
            let file_path = PathBuf::from(format!("/vault/2024/photo_{:04}.jpg", i + 1));
            let size = 1024 * 1024; // 1 MiB

            // Signal start of hashing
            reporter.item_started(&file_path, size);

            // Simulate variable hash time
            let work_time = if i % 4 == 0 {
                delay // Every 4th file takes longer (larger file)
            } else {
                delay / 3
            };
            thread::sleep(work_time);

            // Signal end of hashing (no error)
            reporter.item_finished(&file_path, None);
        });

        reporter.finish();
        thread::sleep(delay);
    }

    // ── Phase: Insert + Summary ───────────────────────────────────────────────
    if new_count > 0 {
        let reporter = builder.insert_reporter(&source, new_count as u64);

        for i in 0..new_count {
            reporter.progress((i + 1) as u64, new_count as u64);
            thread::sleep(delay / 4);
        }

        reporter.finish();
        reporter.summary(
            count,
            new_count,
            count - new_count,
            0,
            Some(std::path::Path::new(".svault/manifests/import-test.json")),
        );
    }

    Ok(())
}
