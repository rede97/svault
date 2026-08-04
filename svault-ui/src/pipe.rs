//! Pipe sink — formats scan results as the svault pipeable text protocol.
//!
//! Output format (one entry per line, written to **stdout**):
//! ```text
//! SCAN:/absolute/source/path
//! new:DCIM/IMG_0001.jpg
//! new:DCIM/IMG_0002.jpg
//! dup:DCIM/IMG_0003.jpg
//! fail:DCIM/broken.dng
//! ```
//!
//! The `SCAN:` header is printed when the scan phase starts.
//! All non-scan events are ignored so stdout stays clean for downstream
//! consumers (e.g. `svault import --files-from -`).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use svault_core::event::{Event, EventSink, ItemStatus, Phase};

use crate::path::relative_display_path;

/// Escape spaces and colons so the output can be parsed unambiguously by
/// `svault import --files-from`.
fn escape(s: &str) -> String {
    s.replace(' ', "\\ ").replace(':', "\\:")
}

/// Scan-only sink writing the pipeable scan protocol to stdout.
pub struct PipeSink {
    /// Whether to emit `dup:` lines for duplicate files.
    show_dup: bool,
    /// Source directory from the scan `PhaseStarted` event (for relative paths).
    source: Mutex<Option<PathBuf>>,
    /// Whether the SCAN: header has been printed (lazily, on first item —
    /// an empty scan produces no output at all).
    header_printed: std::sync::atomic::AtomicBool,
}

impl PipeSink {
    /// Create a new pipe sink.
    pub fn new(show_dup: bool) -> Self {
        Self {
            show_dup,
            source: Mutex::new(None),
            header_printed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn print_line(&self, prefix: &str, path: &Path) {
        use std::sync::atomic::Ordering;
        let source = self.source.lock().unwrap().clone();
        if !self.header_printed.swap(true, Ordering::SeqCst)
            && let Some(src) = &source
        {
            println!("SCAN:{}", src.display());
        }
        let rel = match &source {
            Some(base) => relative_display_path(path, base),
            None => path.display().to_string(),
        };
        println!("{}:{}", prefix, escape(&rel));
    }
}

impl EventSink for PipeSink {
    fn emit(&self, event: &Event) {
        match event {
            Event::PhaseStarted {
                phase: Phase::Scan,
                context,
                ..
            } => {
                let mut source = self.source.lock().unwrap();
                *source = context.source.clone();
            }
            Event::ScanItem { path, status, .. } => match status {
                ItemStatus::New | ItemStatus::Recover => {
                    self.print_line("new", path);
                }
                ItemStatus::Duplicate | ItemStatus::MovedInVault => {
                    if self.show_dup {
                        self.print_line("dup", path);
                    }
                }
                ItemStatus::Failed => {
                    self.print_line("fail", path);
                }
            },
            _ => {}
        }
    }
}
