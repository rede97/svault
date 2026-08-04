//! Vault-to-vault comparison (Beyond Compare style, not git style).
//!
//! - [`diff`] — pure comparison engine (no IO)
//! - [`crate::ops::sync`] — the `svault sync` orchestrator

pub mod diff;

pub use diff::{DiffEntry, DiffPlan, FileRecord, diff_vaults};
