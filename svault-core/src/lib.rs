//! svault-core — domain and application logic for the Svault archive tool.
//!
//! This crate is a **pure library** (architecture rules R1–R4 in
//! `docs/ARCHITECTURE.md`):
//!
//! - no terminal or CLI-parser dependencies
//! - no direct stdin/stdout/stderr access
//! - progress is reported exclusively through [`event::EventSink`]
//! - user confirmation goes through [`event::Interactor`]

pub mod config;
pub mod context;
pub mod db;
pub mod event;
pub mod fs;
pub mod hash;
pub mod lock;
pub mod media;
pub mod ops;
pub mod pipeline;
pub mod session;
pub mod status;
pub mod sync;
pub mod verify;
