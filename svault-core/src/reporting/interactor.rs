//! Interactive user prompt trait.

/// Trait for interactive user prompts (confirmation dialogs, etc.).
pub trait Interactor: Send + Sync {
    fn confirm(&self, message: &str) -> bool;
}

/// No-op interactor that always confirms without prompting.
#[derive(Debug, Clone, Copy, Default)]
pub struct YesInteractor;

impl Interactor for YesInteractor {
    fn confirm(&self, _message: &str) -> bool {
        true
    }
}
