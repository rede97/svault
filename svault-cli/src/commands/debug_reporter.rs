//! Debug command to test sink rendering.
//!
//! Simulates a full import workflow by emitting synthetic [`Event`]s,
//! exercising progress bars and output formatting without touching a vault.

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use rayon::prelude::*;

use svault_core::event::{Event, EventSink, ItemStatus, Phase, PhaseContext, Summary};
use svault_core::ops::ImportSummary;
use svault_ui::TerminalSink;

/// Run sink rendering simulation.
pub fn run(count: usize, delay_ms: u64, show_dup: bool) -> anyhow::Result<()> {
    let sink = TerminalSink::new(show_dup);
    let delay = Duration::from_millis(delay_ms);
    let source = PathBuf::from("/source");
    let vault = PathBuf::from("/vault");

    // ── Phase: Scan ───────────────────────────────────────────────────────────
    sink.emit(&Event::PhaseStarted {
        phase: Phase::Scan,
        total: None,
        context: PhaseContext::source(source.clone()),
    });
    thread::sleep(delay * 5);

    for i in 0..count {
        let status = if show_dup && i % 3 == 0 {
            ItemStatus::Duplicate
        } else {
            ItemStatus::New
        };
        sink.emit(&Event::ScanItem {
            path: source.join(format!("photo_{:04}.jpg", i + 1)),
            size: 1024 * 1024 + (i as u64 * 1000),
            mtime_ms: 1_234_567_890_000 + (i as i64 * 1000),
            status,
            error: None,
        });
        thread::sleep(delay);
    }

    sink.emit(&Event::Preflight {
        source: source.clone(),
        total: count,
        new: count,
        duplicate: 0,
        moved: 0,
        failed: 0,
    });
    sink.emit(&Event::PhaseFinished { phase: Phase::Scan });

    // ── Phase: Copy (parallel) ────────────────────────────────────────────────
    sink.emit(&Event::PhaseStarted {
        phase: Phase::Copy,
        total: Some(count as u64),
        context: PhaseContext::both(source.clone(), vault.clone()),
    });
    (0..count).into_par_iter().for_each(|i| {
        let src = source.join(format!("photo_{:04}.jpg", i + 1));
        let dst = vault.join(format!("2024/photo_{:04}.jpg", i + 1));
        sink.emit(&Event::CopyStarted {
            src: src.clone(),
            dst: dst.clone(),
            bytes: 1024 * 1024,
        });
        thread::sleep(delay * 2);
        sink.emit(&Event::CopyFinished {
            src,
            dst,
            error: None,
        });
    });
    sink.emit(&Event::PhaseFinished { phase: Phase::Copy });

    // ── Phase: Hash (parallel) ────────────────────────────────────────────────
    sink.emit(&Event::PhaseStarted {
        phase: Phase::Hash,
        total: Some(count as u64),
        context: PhaseContext::both(source.clone(), vault.clone()),
    });
    (0..count).into_par_iter().for_each(|i| {
        let path = vault.join(format!("2024/photo_{:04}.jpg", i + 1));
        sink.emit(&Event::HashStarted {
            path: path.clone(),
            bytes: 1024 * 1024,
        });
        thread::sleep(delay);
        sink.emit(&Event::HashFinished {
            path,
            bytes: 1024 * 1024,
            error: None,
        });
    });
    sink.emit(&Event::PhaseFinished { phase: Phase::Hash });

    // ── Phase: Insert ─────────────────────────────────────────────────────────
    sink.emit(&Event::PhaseStarted {
        phase: Phase::Insert,
        total: Some(count as u64),
        context: PhaseContext::both(source.clone(), vault.clone()),
    });
    for i in 0..count {
        sink.emit(&Event::Progress {
            phase: Phase::Insert,
            done: (i + 1) as u64,
            total: count as u64,
        });
        thread::sleep(delay / 2);
    }
    sink.emit(&Event::PhaseFinished {
        phase: Phase::Insert,
    });

    sink.emit(&Event::Summary(Summary::Import(ImportSummary {
        total: count,
        imported: count,
        duplicate: 0,
        failed: 0,
        manifest_path: Some(vault.join(".svault/staging/manifest_debug.json")),
        all_cache_hit: false,
    })));

    Ok(())
}
