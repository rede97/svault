//! Debug command to test reporter output.
//!
//! Simulates a full import workflow to verify progress bars and output formatting.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::reporting::TerminalReporterBuilder;
use crate::cli::OutputFormat;
use svault_core::reporting::{
    AddSummaryReporter, CopyReporter, HashReporter, InsertReporter, ItemStatus, ReporterBuilder,
    ScanReporter,
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

    {
        let add_reporter = builder.add_summary_reporter(&source);
        add_reporter.summary(100, 100, 0, 0);
    }

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
            reporter.classified(&path, status, None);
            reporter.progress((i + 1) as u64);

            thread::sleep(delay / 2);
        }

        let new_count = if show_dup { count - (count / 3) } else { count };
        reporter.preflight(count, new_count, count - new_count, 0, 0, &source);
        reporter.finish();
        thread::sleep(delay);
    }

    let new_count = if show_dup { count - (count / 3) } else { count };

    // ── Phase: Copy ───────────────────────────────────────────────────────────
    if new_count > 0 {
        let reporter = builder.copy_reporter(&source, new_count as u64);

        for i in 0..new_count {
            let path = PathBuf::from(format!("/source/photo_{:04}.jpg", i + 1));
            reporter.item_started(&path, Some(1024 * 1024));

            thread::sleep(delay / 4);

            reporter.progress((i + 1) as u64, new_count as u64);
            reporter.item_finished(&path);
        }

        reporter.finish();
        thread::sleep(delay);
    }

    // ── Phase: Hash ───────────────────────────────────────────────────────────
    if new_count > 0 {
        let reporter = builder.hash_reporter(&source, new_count as u64);

        for i in 0..new_count {
            reporter.progress((i + 1) as u64, new_count as u64);
            thread::sleep(delay / 2);
        }

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
