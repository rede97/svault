use std::io::IsTerminal;
use std::path::PathBuf;

use svault_core::context::VaultContext;
use svault_core::event::{Interactor, YesInteractor};
use svault_core::ops::add::{AddOptions, run_add};

use crate::cli::OutputFormat;
use crate::commands::SinkSet;

pub fn run(
    output: OutputFormat,
    quiet: bool,
    yes: bool,
    paths: Vec<PathBuf>,
) -> anyhow::Result<()> {
    let ctx = VaultContext::open(None, &paths[0])?;
    let opts = AddOptions {
        paths,
        vault_root: ctx.vault_root().to_path_buf(),
        full_id: false, // Default to fast mode for add
        yes,
    };
    let sink = SinkSet::new(&output, quiet, false);
    // Piped/redirected IO can't answer a prompt: auto-confirm (matches the
    // documented non-terminal fallback; keeps add scriptable without --yes).
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let yes_i = YesInteractor;
    let term_i;
    let interactor: &dyn Interactor = if yes || !interactive {
        &yes_i
    } else {
        match &sink {
            SinkSet::Terminal(s) => {
                term_i = s.interactor();
                &term_i
            }
            _ => &yes_i,
        }
    };
    run_add(opts, ctx.db(), sink.as_sink(), interactor)?;
    Ok(())
}
