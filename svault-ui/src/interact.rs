//! Interactive confirmation prompts for the terminal.

use std::io::Write;
use std::sync::Arc;

use indicatif::MultiProgress;
use svault_core::event::Interactor;

/// Terminal interactor that suspends the `MultiProgress` while prompting.
///
/// This prevents the progress-bar redraw loop from overwriting the
/// confirmation prompt text on stderr.
pub struct SuspendingInteractor {
    multi_progress: Arc<MultiProgress>,
}

impl SuspendingInteractor {
    /// Create a new interactor that suspends the given `MultiProgress` during prompts.
    pub fn new(multi_progress: Arc<MultiProgress>) -> Self {
        Self { multi_progress }
    }
}

impl Interactor for SuspendingInteractor {
    fn confirm(&self, prompt: &str) -> bool {
        self.multi_progress.suspend(|| {
            eprint!("{} [y/N] ", prompt);
            std::io::stderr().flush().unwrap();

            let mut input = String::new();
            if std::io::stdin().read_line(&mut input).is_err() {
                return false;
            }
            eprintln!();
            matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
        })
    }
}
