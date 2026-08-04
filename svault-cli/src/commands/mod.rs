//! Command implementations.
//!
//! Each module wires one CLI subcommand: parse inputs, build the appropriate
//! [`svault_core::event::EventSink`] / [`svault_core::event::Interactor`],
//! and call into `svault-core`. Rendering lives in `svault-ui`.

pub mod add;
pub mod clone;
pub mod db;
#[cfg(debug_assertions)]
pub mod debug_reporter;
pub mod import;
pub mod init;
pub mod recheck;
#[cfg(debug_assertions)]
pub mod scan;
pub mod status;
pub mod sync;
pub mod update;
pub mod verify;

use svault_core::event::{EventSink, NoopSink};
use svault_ui::{JsonSink, TerminalSink};

use crate::cli::OutputFormat;

/// Sink selection shared by all commands.
///
/// Owns the concrete sink value; pass [`SinkSet::as_sink`] to core operations.
///
/// Interactor pattern used by commands that confirm:
///
/// ```ignore
/// let sink = SinkSet::new(&output, quiet, show_dup);
/// let yes_i = YesInteractor;
/// let term_i; // declared here so it outlives the borrow
/// let interactor: &dyn Interactor = if yes {
///     &yes_i
/// } else {
///     match &sink {
///         SinkSet::Terminal(s) => {
///             term_i = s.interactor();
///             &term_i
///         }
///         // JSON / quiet mode never prompts (would corrupt the event stream)
///         _ => &yes_i,
///     }
/// };
/// ```
pub enum SinkSet {
    /// indicatif progress rendering (boxed: much larger than other variants).
    Terminal(Box<TerminalSink>),
    /// JSON event stream.
    Json(JsonSink),
    /// Discard all events (--quiet).
    Quiet(NoopSink),
}

impl SinkSet {
    /// Build the sink for the given output mode.
    pub fn new(output: &OutputFormat, quiet: bool, show_dup: bool) -> Self {
        if quiet {
            SinkSet::Quiet(NoopSink)
        } else {
            match output {
                OutputFormat::Human => SinkSet::Terminal(Box::new(TerminalSink::new(show_dup))),
                OutputFormat::Json => SinkSet::Json(JsonSink::new()),
            }
        }
    }

    /// The sink as a trait object for core operations.
    pub fn as_sink(&self) -> &dyn EventSink {
        match self {
            SinkSet::Terminal(s) => s.as_ref(),
            SinkSet::Json(s) => s,
            SinkSet::Quiet(s) => s,
        }
    }
}
