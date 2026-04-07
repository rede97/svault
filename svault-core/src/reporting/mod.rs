//! Reporting abstraction layer for svault-core.
//!
//! This module provides a trait-based event reporting system that allows
//! svault-core to emit progress and status events without coupling to
//! specific output mechanisms (terminal, GUI, etc.).
//!
//! Design principles:
//! - Core emits structured events, not formatted strings
//! - CLI/GUI implement Reporter trait to handle events
//! - No direct dependency on indicatif, console, or println! in core

use std::path::PathBuf;
use std::sync::Arc;

/// High-level operation being performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Import,
    ImportVfs,
}

/// Phase within an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseKind {
    Scan,
    Fingerprint,
    DedupLookup,
    Copy,
    Verify,
    Insert,
}

/// Classification status of an item after scanning/deduplication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    New,
    Duplicate,
    Recover,
    MovedInVault,
    Failed,
}

/// Phase for per-item progress tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemPhase {
    Copy,
    Verify,
}

/// Core events emitted by svault-core operations.
/// 
/// These events are designed to support both CLI progress bars and
/// GUI real-time updates without mandating a specific rendering approach.
#[derive(Debug, Clone)]
pub enum CoreEvent {
    /// Operation started
    RunStarted {
        operation: OperationKind,
    },
    /// Operation completed with summary
    RunFinished {
        operation: OperationKind,
        total: usize,
        imported: usize,
        duplicate: usize,
        failed: usize,
    },

    /// Phase started (e.g., Scan, Copy)
    PhaseStarted {
        phase: PhaseKind,
        total: Option<u64>,
    },
    /// Phase progress update
    PhaseProgress {
        phase: PhaseKind,
        completed: u64,
        total: Option<u64>,
    },
    /// Phase completed
    PhaseFinished {
        phase: PhaseKind,
    },

    /// File discovered during scanning
    /// 
    /// Emitted as soon as a file is found, before classification.
    /// GUI can use this to populate the file list and request thumbnails.
    ItemDiscovered {
        path: PathBuf,
        size: u64,
        mtime_ms: i64,
    },
    /// File classified after deduplication check
    /// 
    /// Emitted after CRC/ hash lookup determines the file status.
    /// Maps to: New, Duplicate, Recover, MovedInVault, Failed
    ItemClassified {
        path: PathBuf,
        status: ItemStatus,
        detail: Option<String>,
    },

    /// Item processing started (e.g., copy beginning)
    ItemStarted {
        path: PathBuf,
        phase: ItemPhase,
        bytes_total: Option<u64>,
    },
    /// Item progress update
    /// 
    /// First version may only emit at start/end; future versions
    /// can add per-file byte progress without breaking the API.
    ItemProgress {
        path: PathBuf,
        phase: ItemPhase,
        bytes_done: u64,
        bytes_total: Option<u64>,
    },
    /// Item processing completed
    ItemFinished {
        path: PathBuf,
        phase: ItemPhase,
    },

    /// Warning message (non-fatal)
    Warning {
        message: String,
        path: Option<PathBuf>,
    },
    /// Error message (potentially fatal for this item)
    Error {
        message: String,
        path: Option<PathBuf>,
    },
}

/// Trait for receiving core events.
/// 
/// Implementations handle events according to their target medium:
/// - TerminalReporter (CLI): renders progress bars and status text
/// - ChannelReporter (GUI): forwards events via channel for async handling
/// - NoopReporter: silently discards all events (testing, JSON mode)
pub trait Reporter: Send + Sync {
    /// Emit a core event.
    fn emit(&self, event: CoreEvent);
}

/// No-op reporter that discards all events.
/// 
/// Useful for:
/// - Unit tests that don't care about progress
/// - JSON output mode where progress shouldn't clutter stdout
/// - Suppressing output entirely
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopReporter;

impl Reporter for NoopReporter {
    fn emit(&self, _event: CoreEvent) {}
}

/// Shared reporter type for convenient passing across threads.
pub type SharedReporter = Arc<dyn Reporter>;

/// Helper function to create a shared noop reporter.
pub fn noop_reporter() -> SharedReporter {
    Arc::new(NoopReporter)
}

/// Trait for user interaction (confirmation prompts, etc.)
/// 
/// This abstracts terminal interaction so core doesn't directly read stdin.
/// CLI implements this for terminal interaction, GUI would implement differently.
pub trait Interactor: Send + Sync {
    /// Confirm an action with the user.
    /// Returns true if user confirms, false otherwise.
    fn confirm(&self, message: &str) -> bool;
}

/// No-op interactor that always returns true (for --yes mode or automation)
#[derive(Debug, Clone, Copy, Default)]
pub struct YesInteractor;

impl Interactor for YesInteractor {
    fn confirm(&self, _message: &str) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Test reporter that collects all events for verification.
    #[derive(Debug, Default)]
    struct TestReporter {
        events: Mutex<Vec<CoreEvent>>,
    }

    impl Reporter for TestReporter {
        fn emit(&self, event: CoreEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[test]
    fn test_noop_reporter_silent() {
        let reporter = NoopReporter;
        // Should not panic or do anything observable
        reporter.emit(CoreEvent::RunStarted {
            operation: OperationKind::Import,
        });
    }

    #[test]
    fn test_reporter_can_emit_all_event_types() {
        let reporter = TestReporter::default();
        
        reporter.emit(CoreEvent::RunStarted {
            operation: OperationKind::Import,
        });
        reporter.emit(CoreEvent::PhaseStarted {
            phase: PhaseKind::Scan,
            total: Some(100),
        });
        reporter.emit(CoreEvent::ItemDiscovered {
            path: PathBuf::from("/test/file.jpg"),
            size: 1024,
            mtime_ms: 1234567890,
        });
        reporter.emit(CoreEvent::ItemClassified {
            path: PathBuf::from("/test/file.jpg"),
            status: ItemStatus::New,
            detail: None,
        });
        reporter.emit(CoreEvent::PhaseProgress {
            phase: PhaseKind::Scan,
            completed: 50,
            total: Some(100),
        });
        reporter.emit(CoreEvent::PhaseFinished {
            phase: PhaseKind::Scan,
        });
        reporter.emit(CoreEvent::RunFinished {
            operation: OperationKind::Import,
            total: 1,
            imported: 1,
            duplicate: 0,
            failed: 0,
        });

        let events = reporter.events.lock().unwrap();
        assert_eq!(events.len(), 7);
    }

    #[test]
    fn test_shared_reporter() {
        let reporter: SharedReporter = Arc::new(TestReporter::default());
        let reporter2 = Arc::clone(&reporter);
        
        // Simulate cross-thread usage
        reporter.emit(CoreEvent::RunStarted {
            operation: OperationKind::Import,
        });
        reporter2.emit(CoreEvent::RunFinished {
            operation: OperationKind::Import,
            total: 0,
            imported: 0,
            duplicate: 0,
            failed: 0,
        });
        
        // Both references work
    }
}
