//! CLI reporting wiring.

mod json;
mod pipe;
mod terminal;

pub use json::JsonReporterBuilder;
pub use pipe::PipeReporterBuilder;
pub use terminal::{SuspendingInteractor, TerminalReporterBuilder};

// This module provides reporter builders and interactors for CLI output.
//
// For import flows, construct the reporter builder and interactor directly:
// - Use `TerminalReporterBuilder` with `SuspendingInteractor` for human output
// - Use `JsonReporterBuilder` with `YesInteractor` for JSON output
//
// Example:
// ```rust
// let builder = Arc::new(TerminalReporterBuilder::new());
// let interactor = SuspendingInteractor::new(builder.multi_progress.clone());
// opts.run_import(db, &builder, &interactor)?;
// ```
