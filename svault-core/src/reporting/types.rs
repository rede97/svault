//! Shared types used across reporting traits.

/// Classification status of an item after scanning / deduplication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    New,
    Duplicate,
    Recover,
    MovedInVault,
    Failed,
}

/// Result of a single file copy operation.
#[derive(Debug, Clone)]
pub enum CopyItemResult {
    /// File was successfully copied.
    Ok,
    /// File copy failed with an error message.
    Failed { message: String },
}

/// Confidence level of a file-path match found by `svault update`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchConfidence {
    /// Matched by SHA-256 — cryptographically definitive.
    Definitive,
    /// Matched by XXH3-128 only — fast but theoretically collidable.
    Fast,
}
