//! Pipe reporter — formats scan results as the svault pipeable text protocol.
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
//! The `SCAN:` header is printed when the scan-phase reporter is created.
//! Each `classified` call produces one output line.
//! All other methods are no-ops so stdout stays clean for downstream
//! consumers (e.g. `svault import --files-from -`).
//!
//! Copy, Hash, and Insert phases use [`Noop`] — the pipe reporter is
//! scan-only by design.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use svault_core::reporting::{ItemStatus, Noop, ReporterBuilder, ScanReporter};

// ─────────────────────────────────────────────────────────────────────────────
// PipeScanReporter
// ─────────────────────────────────────────────────────────────────────────────

/// Phase reporter that writes classified files to stdout in the svault
/// pipeable scan format.
///
/// Receives absolute paths via [`ScanReporter::classified`] and strips the
/// source prefix before writing.  All other methods are silent.
pub struct PipeScanReporter {
    /// Canonical source root — used to compute relative paths.
    source: PathBuf,
    /// Whether to emit `dup:` lines for duplicate files.
    show_dup: bool,
    /// Whether the SCAN: header has been printed.
    header_printed: AtomicBool,
}

impl PipeScanReporter {
    /// Escape spaces and colons so the output can be parsed unambiguously by
    /// `svault import --files-from`.
    fn escape(s: &str) -> String {
        s.replace(' ', "\\ ").replace(':', "\\:")
    }

    /// Print the SCAN: header if not already printed.
    fn print_header(&self) {
        if !self.header_printed.swap(true, Ordering::SeqCst) {
            println!("SCAN:{}", self.source.display());
        }
    }

    fn print_line(&self, prefix: &str, path: &Path) {
        self.print_header();
        let rel = path.strip_prefix(&self.source).unwrap_or(path);
        println!("{}:{}", prefix, Self::escape(&rel.display().to_string()));
    }
}

impl ScanReporter for PipeScanReporter {
    fn discovered(&self, _path: &Path, _size: u64, _mtime_ms: i64) {}

    fn classified(&self, path: &Path, _size: u64, status: ItemStatus, _detail: Option<&str>) {
        match status {
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
        }
    }

    fn progress(&self, _completed: u64) {}
    fn warning(&self, _message: &str, _path: Option<&Path>) {}

    fn error(&self, _message: &str, path: Option<&Path>) {
        // IO / hash errors also produce a fail line so downstream consumers
        // know the file was not successfully scanned.
        if let Some(p) = path {
            self.print_line("fail", p);
        }
    }

    fn preflight(
        &self,
        _total_scanned: usize,
        _new_count: usize,
        _duplicate_count: usize,
        _moved_count: usize,
        _failed_count: usize,
        _source: &Path,
    ) {
    }

    fn finish(&self) {
        // No progress bar to clear.
    }
}

impl Drop for PipeScanReporter {
    fn drop(&mut self) {
        // No resources to release.
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PipeReporterBuilder
// ─────────────────────────────────────────────────────────────────────────────

/// Builder that produces a [`PipeScanReporter`] for the scan phase and
/// [`Noop`] reporters for all other phases.
///
/// The `SCAN:` header line is printed when `scan_reporter` is called,
/// so it always appears before any classified-file lines.
pub struct PipeReporterBuilder {
    show_dup: bool,
}

impl PipeReporterBuilder {
    pub fn new(show_dup: bool) -> Self {
        Self { show_dup }
    }
}

impl ReporterBuilder for PipeReporterBuilder {
    type Scan = PipeScanReporter;
    type Copy = Noop;
    type Hash = Noop;
    type Insert = Noop;
    type AddSummary = Noop;
    type Recheck = Noop;
    type UpdateApply = Noop;
    type Verify = Noop;

    fn scan_reporter(&self, src: &Path) -> PipeScanReporter {
        // Canonicalise so strip_prefix works correctly against absolute paths
        // emitted by the pipeline walk stages.
        let source = std::fs::canonicalize(src).unwrap_or_else(|_| src.to_path_buf());

        // Header is printed lazily on first classified file, so empty scans
        // produce no output (matches test expectations).
        PipeScanReporter {
            source,
            show_dup: self.show_dup,
            header_printed: AtomicBool::new(false),
        }
    }

    fn copy_reporter(&self, _source: &Path, _vault_root: &Path, _total: u64) -> Noop {
        Noop
    }

    fn hash_reporter(&self, _source: &Path, _total: u64) -> Noop {
        Noop
    }

    fn insert_reporter(&self, _source: &Path, _total: u64) -> Noop {
        Noop
    }

    fn add_summary_reporter(&self, _vault_root: &Path) -> Noop {
        Noop
    }

    fn recheck_reporter(&self, _total: u64) -> Noop {
        Noop
    }

    fn update_hash_reporter(&self, _source: &Path, _total: u64) -> Noop {
        Noop
    }

    fn update_apply_reporter(&self, _total: u64) -> Noop {
        Noop
    }

    fn verify_reporter(&self, _total: u64) -> Noop {
        Noop
    }
}
