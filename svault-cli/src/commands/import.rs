use std::io::{self, BufRead};
use std::path::PathBuf;

use svault_core::config::SyncStrategy;
use svault_core::context::VaultContext;
use svault_core::event::{Interactor, YesInteractor};
use svault_core::ops::ImportOptions;
use svault_core::ops::import::normalize_path;

use crate::cli::OutputFormat;
use crate::commands::SinkSet;

#[allow(clippy::too_many_arguments)]
pub fn run(
    output: OutputFormat,
    quiet: bool,
    dry_run: bool,
    yes: bool,
    source: PathBuf,
    files_from: Option<PathBuf>,
    target: Option<PathBuf>,
    strategy: Vec<svault_core::config::TransferStrategyArg>,
    force: bool,
    full_id: bool,
    show_dup: bool,
    max_depth: usize,
    include: Vec<String>,
    exclude: Vec<String>,
) -> anyhow::Result<()> {
    // Parse file-list input (stdin or file) into Vec<PathBuf> before entering core.
    let file_list: Option<Vec<PathBuf>> = match files_from {
        None => None,
        Some(ref path) => {
            let lines: Vec<String> = if path.as_os_str() == "-" {
                io::stdin()
                    .lock()
                    .lines()
                    .map_while(Result::ok)
                    .filter(|l| !l.is_empty())
                    .collect()
            } else {
                let file = std::fs::File::open(path).map_err(|e| {
                    anyhow::anyhow!("cannot open file list '{}': {}", path.display(), e)
                })?;
                io::BufReader::new(file)
                    .lines()
                    .map_while(Result::ok)
                    .filter(|l| !l.is_empty())
                    .collect()
            };

            // Parse scan-output format: SCAN:<prefix> new:file1 dup:file2 …
            // Only "new:" entries are imported; relative paths are joined with source.
            // Paths are escape-encoded by PipeSink (`\ `, `\:`, `\\`), so splitting
            // must be escape-aware — a naive split_whitespace() would shred
            // escaped spaces (e.g. "new:my\ photo.jpg" → "new:my\" + "photo.jpg").
            let source_normalized = normalize_path(&source);
            let source_canon = dunce::canonicalize(&source_normalized)
                .unwrap_or_else(|_| source_normalized.clone());
            let mut paths: Vec<PathBuf> = Vec::new();

            for line in &lines {
                for part in split_escaped_whitespace(line) {
                    if let Some(rel) = part.strip_prefix("new:") {
                        let unescaped = rel
                            .replace("\\ ", " ")
                            .replace("\\:", ":")
                            .replace("\\\\", "\\");
                        if !unescaped.is_empty() {
                            paths.push(source_canon.join(unescaped));
                        }
                    }
                }
            }

            if paths.is_empty() {
                return Err(anyhow::anyhow!(
                    "no new files to import (all files are duplicates or failed)"
                ));
            }

            Some(paths)
        }
    };

    let source_normalized = normalize_path(&source);
    let source_canon =
        dunce::canonicalize(&source_normalized).unwrap_or_else(|_| source_normalized.clone());
    let ctx = VaultContext::open(target, &source_canon)?;

    let opts = ImportOptions {
        source: source_canon,
        vault_root: ctx.vault_root().to_path_buf(),
        strategy: SyncStrategy(strategy),
        dry_run,
        yes,
        import_config: ctx.config().import.clone(),
        force,
        full_id,
        show_dup,
        files_from: file_list,
        max_depth,
        include,
        exclude,
    };

    // JSON mode requires --yes flag for non-interactive execution
    if matches!(output, OutputFormat::Json) && !yes && !dry_run {
        return Err(anyhow::anyhow!(
            "JSON output mode requires --yes flag to confirm non-interactive execution"
        ));
    }

    let sink = SinkSet::new(&output, quiet, show_dup);
    let yes_i = YesInteractor;
    let term_i;
    let interactor: &dyn Interactor = if yes {
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

    let _summary = opts.run_import(ctx.db(), sink.as_sink(), interactor)?;
    Ok(())
}

/// Split a scan-protocol line into tokens on **unescaped** whitespace.
///
/// `\X` is kept intact as an escape pair (decoded later by the caller), so
/// escaped spaces (`\ `) inside paths do not split the token. Matches the
/// encoding produced by `svault_ui::PipeSink`'s `escape`.
fn split_escaped_whitespace(line: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                cur.push('\\');
                // Keep the escaped char verbatim (may itself be whitespace).
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
            }
            c if c.is_whitespace() => {
                if !cur.is_empty() {
                    parts.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::split_escaped_whitespace;

    /// Unescape helper mirroring the caller's decode order.
    fn unescape(s: &str) -> String {
        s.replace("\\ ", " ")
            .replace("\\:", ":")
            .replace("\\\\", "\\")
    }

    #[test]
    fn split_respects_escaped_space() {
        let parts = split_escaped_whitespace("SCAN:/src new:my\\ photo\\ 01.jpg dup:x.jpg");
        assert_eq!(parts, ["SCAN:/src", "new:my\\ photo\\ 01.jpg", "dup:x.jpg"]);
        assert_eq!(
            unescape(parts[1].strip_prefix("new:").unwrap()),
            "my photo 01.jpg"
        );
    }

    #[test]
    fn split_handles_escaped_backslash_and_colon() {
        // Path `a\b:c` → escaped as `a\\b\:c`
        let parts = split_escaped_whitespace("new:a\\\\b\\:c new:z.jpg");
        assert_eq!(parts.len(), 2);
        assert_eq!(unescape(parts[0].strip_prefix("new:").unwrap()), "a\\b:c");
    }

    #[test]
    fn split_roundtrips_trailing_escape_chars() {
        // Trailing lone backslash must not panic and stays verbatim.
        let parts = split_escaped_whitespace("new:end\\");
        assert_eq!(parts, ["new:end\\"]);
    }
}
