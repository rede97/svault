//! svault-ui — terminal presentation layer for Svault.
//!
//! This crate owns **everything** that touches the terminal (architecture
//! layer L3, see `docs/ARCHITECTURE.md`):
//!
//! | Module       | Provides                                        |
//! |--------------|-------------------------------------------------|
//! | [`terminal`] | [`TerminalSink`] — indicatif progress rendering |
//! | [`json`]     | [`JsonSink`] — one JSON object per event line   |
//! | [`pipe`]     | [`PipeSink`] — machine-readable scan protocol   |
//! | [`interact`] | [`SuspendingInteractor`] — y/N prompts          |
//! | [`status`]   | status report tables / JSON rendering           |
//!
//! All sinks implement [`svault_core::event::EventSink`] and are passed to
//! core operations by the CLI.

pub mod interact;
pub mod json;
pub mod messages;
pub mod path;
pub mod pipe;
pub mod status;
pub mod terminal;

pub use interact::SuspendingInteractor;
pub use json::JsonSink;
pub use pipe::PipeSink;
pub use terminal::TerminalSink;
