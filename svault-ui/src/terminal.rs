//! Terminal event sink — renders core [`Event`]s with indicatif progress bars.
//!
//! A single stateful sink replaces the old per-phase reporter structs.
//! Only one phase is active at a time (the pipeline is sequential); within a
//! phase, events arrive from multiple Rayon threads and are serialized
//! through a `Mutex`.
//!
//! All text output is batched into a single `String` per event and printed
//! via `ProgressBar::println`, which keeps it synchronized with the
//! progress-bar draw cycle.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use svault_core::event::{
    Event, EventSink, Hint, ItemStatus, MatchConfidence, Phase, PhaseContext, RecheckSummary,
    Summary,
};
use svault_core::ops::RecheckStatus;
use svault_core::verify::VerifyResult;

use crate::interact::SuspendingInteractor;
use crate::path::relative_display_path;

/// Braille pattern spinner characters.
const TICK_CHARS: &str = "⠁⠂⠄⡀⢀⠠⠐⠈ ";

/// Format byte size to human-readable string (B, KiB, MiB, GiB, TiB).
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let exp = (bytes as f64).log(1024.0).min((UNITS.len() - 1) as f64) as usize;
    let value = bytes as f64 / 1024f64.powi(exp as i32);
    if exp == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", value, UNITS[exp])
    }
}

/// Per-phase mutable rendering state.
struct PhaseState {
    pb: ProgressBar,
    phase: Phase,
    total: u64,
    context: PhaseContext,
    is_tty: bool,
    // scan phase counters
    scanned: u64,
    new_count: u64,
    duplicate_count: u64,
    recover_count: u64,
    moved_count: u64,
    failed_count: u64,
    // copy phase: currently transferring file names (across threads)
    active_files: Vec<String>,
    // hash phase: throughput + relocate matches
    bytes_processed: u64,
    start: Instant,
    matches: Vec<(String, String, MatchConfidence)>,
}

impl PhaseState {
    fn new(pb: ProgressBar, phase: Phase, total: u64, context: PhaseContext, is_tty: bool) -> Self {
        Self {
            pb,
            phase,
            total,
            context,
            is_tty,
            scanned: 0,
            new_count: 0,
            duplicate_count: 0,
            recover_count: 0,
            moved_count: 0,
            failed_count: 0,
            active_files: Vec::new(),
            bytes_processed: 0,
            start: Instant::now(),
            matches: Vec::new(),
        }
    }

    fn println<S: AsRef<str>>(&self, s: S) {
        if self.is_tty {
            self.pb.println(s);
        } else {
            // indicatif suppresses all draw-target output in non-tty mode,
            // including println — write directly instead.
            eprintln!("{}", s.as_ref());
        }
    }

    fn rel_source(&self, path: &Path) -> String {
        match &self.context.source {
            Some(base) => relative_display_path(path, base),
            None => path.display().to_string(),
        }
    }

    fn rel_vault(&self, path: &Path) -> String {
        match &self.context.vault_root {
            Some(base) => relative_display_path(path, base),
            None => path.display().to_string(),
        }
    }
}

/// Terminal sink rendering [`Event`]s as progress bars and styled text.
///
/// Create one per command invocation:
///
/// ```no_run
/// use svault_ui::TerminalSink;
/// let sink = TerminalSink::new(false);
/// let interactor = sink.interactor();
/// // pass `&sink` and `&interactor` to a core operation
/// ```
pub struct TerminalSink {
    multi: Arc<MultiProgress>,
    state: Mutex<Option<PhaseState>>,
    /// Whether to print per-file lines for duplicates during scan.
    show_dup: bool,
    /// Whether stderr is an interactive terminal. When false (piped/CI),
    /// bars are hidden and text lines go straight to stderr.
    is_tty: bool,
}

impl TerminalSink {
    /// Create a sink with a fresh `MultiProgress`.
    pub fn new(show_dup: bool) -> Self {
        Self {
            multi: Arc::new(MultiProgress::new()),
            state: Mutex::new(None),
            show_dup,
            is_tty: console::Term::stderr().is_term(),
        }
    }

    /// An interactor that suspends this sink's progress display while prompting.
    pub fn interactor(&self) -> SuspendingInteractor {
        SuspendingInteractor::new(Arc::clone(&self.multi))
    }

    /// Print a line synchronized with the progress display, even when no
    /// phase is active (uses a transient hidden bar).
    ///
    /// In non-tty mode (piped/CI) lines go straight to stderr — indicatif
    /// suppresses all draw-target output there, including `println`.
    fn println<S: AsRef<str>>(&self, s: S) {
        if !self.is_tty {
            eprintln!("{}", s.as_ref());
            return;
        }
        let guard = self.state.lock().unwrap();
        match guard.as_ref() {
            Some(st) => st.println(s),
            None => {
                let pb = self.multi.add(ProgressBar::hidden());
                pb.println(s);
                self.multi.remove(&pb);
            }
        }
    }

    /// Print a line through the phase's progress bar.
    ///
    /// Only used in tty mode — [`TerminalSink::println`] handles non-tty.
    fn with_state<R>(&self, f: impl FnOnce(&mut PhaseState) -> R) -> Option<R> {
        let mut guard = self.state.lock().unwrap();
        guard.as_mut().map(f)
    }

    // ── phase lifecycle ───────────────────────────────────────────────────

    fn phase_started(&self, phase: Phase, total: Option<u64>, context: PhaseContext) {
        // Non-tty: hidden bar (counters still work), text via direct eprintln.
        if !self.is_tty {
            let pb = self.multi.add(ProgressBar::hidden());
            let mut guard = self.state.lock().unwrap();
            if let Some(old) = guard.take() {
                old.pb.finish_and_clear();
            }
            *guard = Some(PhaseState::new(
                pb,
                phase,
                total.unwrap_or(0),
                context,
                false,
            ));
            return;
        }

        let pb = match phase {
            Phase::Scan => {
                let pb = self.multi.add(ProgressBar::new_spinner());
                pb.set_style(
                    ProgressStyle::default_spinner()
                        .template(
                            "{spinner:.cyan} {prefix:.cyan.bold} {msg} {pos:>7} files ({per_sec})",
                        )
                        .unwrap()
                        .tick_chars(TICK_CHARS),
                );
                pb.set_prefix("Scanning");
                let msg = context
                    .source
                    .as_ref()
                    .map(|s| truncate_path(&s.display().to_string(), 40))
                    .unwrap_or_default();
                pb.set_message(style(msg).color256(244).to_string());
                pb.enable_steady_tick(Duration::from_millis(100));
                pb
            }
            _ => {
                let pb = self.multi.add(ProgressBar::new(total.unwrap_or(0)));
                let (prefix, template) = match phase {
                    Phase::Copy => (
                        "Copying",
                        "  {prefix:.cyan.bold} [{bar:40}] {pos}/{len} ({percent}%): {wide_msg}",
                    ),
                    Phase::Hash => (
                        "Hashing",
                        "  {prefix:.cyan.bold} [{bar:40}] {pos}/{len} ({percent}%) {msg}",
                    ),
                    Phase::Insert => (
                        "Inserting",
                        "  {prefix:.cyan.bold} [{bar:40}] {pos}/{len} ({percent}%)",
                    ),
                    Phase::Apply => (
                        "Updating",
                        "  {prefix:.cyan.bold} [{bar:40}] {pos}/{len}  {msg}",
                    ),
                    Phase::Recheck => (
                        "Checking",
                        "  {prefix:.cyan.bold} [{bar:40}] {pos}/{len}  {msg}",
                    ),
                    Phase::Verify => (
                        "Verifying",
                        "  {prefix:.cyan.bold} [{bar:40}] {pos}/{len}  {msg}",
                    ),
                    Phase::Scan | Phase::Compare => unreachable!(),
                };
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template(template)
                        .unwrap()
                        .progress_chars("=> "),
                );
                pb.set_prefix(prefix);
                pb
            }
        };

        let mut guard = self.state.lock().unwrap();
        if let Some(old) = guard.take() {
            old.pb.finish_and_clear();
        }
        *guard = Some(PhaseState::new(
            pb,
            phase,
            total.unwrap_or(0),
            context,
            true,
        ));
    }

    fn phase_finished(&self, phase: Phase) {
        let mut guard = self.state.lock().unwrap();
        let Some(st) = guard.take() else { return };
        if st.phase != phase {
            // Defensive: unexpected phase ordering — restore and bail.
            let restored = PhaseState { ..st };
            *guard = Some(restored);
            return;
        }

        match phase {
            Phase::Scan => {
                st.println(format!(
                    "✓ Scan complete ({} files; new {}, duplicate {}, recover {}, moved {}, failed {})",
                    st.scanned, st.new_count, st.duplicate_count, st.recover_count,
                    st.moved_count, st.failed_count
                ));
            }
            Phase::Copy => {
                st.println(format!(
                    "✓ Copy complete ({}/{})",
                    st.pb.position(),
                    st.total
                ));
            }
            Phase::Hash => {
                let mut output = format!(
                    "✓ Fingerprint complete ({}/{})\n",
                    st.pb.position(),
                    st.total
                );
                if !st.matches.is_empty() {
                    output.push('\n');
                    output.push_str(&format!("{}\n", style("Matches found:").bold()));
                    for (old, new, conf) in &st.matches {
                        let label = match conf {
                            MatchConfidence::Definitive => style("[Definitive]").green(),
                            MatchConfidence::Fast => style("[Fast match]").yellow(),
                        };
                        output.push_str(&format!(
                            "  {} {} -> {}\n",
                            label,
                            style(old),
                            style(new).green()
                        ));
                    }
                    let definitive = st
                        .matches
                        .iter()
                        .filter(|(_, _, c)| *c == MatchConfidence::Definitive)
                        .count();
                    let fast = st.matches.len() - definitive;
                    if definitive > 0 {
                        output.push_str(&format!(
                            "    {} match(es) with SHA-256 (definitive)\n",
                            style(definitive).green().bold()
                        ));
                    }
                    if fast > 0 {
                        output.push_str(&format!(
                            "    {} match(es) with XXH3-128 only (fast)\n",
                            style(fast).yellow().bold()
                        ));
                    }
                }
                st.println(output.trim_end());
            }
            Phase::Insert => {
                st.println(format!(
                    "✓ Insert complete ({}/{})",
                    st.pb.position(),
                    st.total
                ));
            }
            Phase::Apply => {
                st.println(format!(
                    "✓ Update complete ({}/{})",
                    st.pb.position(),
                    st.total
                ));
            }
            Phase::Recheck => {
                st.println(format!(
                    "✓ Recheck complete ({}/{})",
                    st.pb.position(),
                    st.total
                ));
            }
            Phase::Verify | Phase::Compare => {} // summary follows immediately
        }
        st.pb.finish_and_clear();
    }

    // ── scan phase ────────────────────────────────────────────────────────

    fn scan_item(
        &self,
        path: &Path,
        size: u64,
        status: ItemStatus,
        error: Option<&str>,
        show_dup: bool,
    ) {
        self.with_state(|st| {
            st.scanned += 1;
            match status {
                ItemStatus::New => st.new_count += 1,
                ItemStatus::Duplicate => st.duplicate_count += 1,
                ItemStatus::MovedInVault => st.moved_count += 1,
                ItemStatus::Recover => st.recover_count += 1,
                ItemStatus::Failed => st.failed_count += 1,
            }
            st.pb.inc(1);

            if status == ItemStatus::Duplicate && !show_dup {
                return;
            }

            let rel_path = st.rel_source(path);
            let size_str = format_bytes(size);
            let (label, label_style) = match status {
                ItemStatus::New => ("Found", style("Found").green().bold()),
                ItemStatus::Duplicate => ("Duplicate", style("Duplicate").yellow().bold()),
                ItemStatus::MovedInVault => ("Moved", style("Moved").color256(208).bold()),
                ItemStatus::Recover => ("Recover", style("Recover").cyan().bold()),
                ItemStatus::Failed => ("Error", style("Error").red().bold()),
            };
            let _ = label;

            if let Some(err) = error {
                st.println(format!(
                    "  {} {} ({}) - {}",
                    label_style,
                    rel_path,
                    size_str,
                    style(err).red()
                ));
            } else {
                st.println(format!("  {} {} ({})", label_style, rel_path, size_str));
            }
        });
    }

    fn preflight(
        &self,
        source: &Path,
        total: usize,
        new: usize,
        duplicate: usize,
        moved: usize,
        failed: usize,
    ) {
        let mut output = String::new();

        if new == 0 {
            output.push_str(&format!(
                "{} Scanned {} files from {}\n\n",
                style("Finished:").bold().green(),
                style(total).green(),
                style(source.display()).color256(244),
            ));
            if duplicate > 0 {
                output.push_str(&format!(
                    "All {} files matched cache (no new files detected).",
                    style(duplicate).cyan()
                ));
            } else if moved > 0 {
                output.push_str(&format!(
                    "Found {} moved files. Run `svault update` to fix paths.",
                    style(moved).cyan()
                ));
            } else {
                output.push_str("No files to import.");
            }
            self.println(output);
            return;
        }

        output.push_str(&format!(
            "{} Scanned {} files from {}\n\n",
            style("Finished:").bold().green(),
            style(total).green(),
            style(source.display()).color256(244),
        ));
        output.push_str(&format!("{}\n", style("Pre-flight:").bold()));
        output.push_str(&format!(
            "  {}  {}\n",
            style(format!("Likely new:       {:>6}", new)).green(),
            style("will be imported")
        ));
        output.push_str(&format!(
            "  {}  {}\n",
            style(format!("Likely duplicate: {:>6}", duplicate)).yellow(),
            style("already in vault (cache hit)")
        ));
        if moved > 0 {
            output.push_str(&format!(
                "  {}  {}\n",
                style(format!("Moved in vault:   {:>6}", moved)).cyan(),
                style("path will be updated")
            ));
        }
        if failed > 0 {
            output.push_str(&format!(
                "  {}\n",
                style(format!("Errors:           {:>6}", failed)).red()
            ));
        }
        self.println(output.trim_end());
    }

    // ── copy phase ────────────────────────────────────────────────────────

    fn copy_started(&self, src: &Path, dst: &Path, bytes: u64) {
        self.with_state(|st| {
            let name = file_name(src);
            st.println(format!(
                "  {} {} ({}) -> {}",
                style("Copying").green().bold(),
                st.rel_source(src),
                format_bytes(bytes),
                style(st.rel_vault(dst)).color256(244)
            ));
            st.active_files.push(name);
            let msg = st.active_files.join(", ");
            st.pb.set_message(msg);
        });
    }

    fn copy_finished(&self, src: &Path, error: Option<&str>) {
        self.with_state(|st| {
            let name = file_name(src);
            st.active_files.retain(|f| f != &name);
            let msg = st.active_files.join(", ");
            st.pb.set_message(msg);
            st.pb.inc(1);

            if let Some(err) = error {
                st.println(format!(
                    "{} {}: {}",
                    style("Error:").red().bold(),
                    src.display(),
                    err
                ));
            }
        });
    }

    // ── hash phase ────────────────────────────────────────────────────────

    fn hash_finished(&self, path: &Path, bytes: u64, error: Option<&str>) {
        self.with_state(|st| {
            if let Some(err) = error {
                st.println(format!(
                    "  {} {}: {}",
                    style("Error").red(),
                    file_name(path),
                    err
                ));
            }

            st.bytes_processed += bytes;
            let elapsed = st.start.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 {
                st.bytes_processed as f64 / elapsed
            } else {
                0.0
            };
            st.pb
                .set_message(format!("{} /s", format_bytes(speed as u64)));
            st.pb.inc(1);
        });
    }

    // ── verify / recheck items ────────────────────────────────────────────

    fn verify_item(&self, path: &Path, result: &VerifyResult) {
        self.with_state(|st| {
            st.pb.inc(1);
            match result {
                VerifyResult::Ok | VerifyResult::HashNotAvailable => {}
                VerifyResult::Missing => {
                    st.println(format!(
                        "  {} {}",
                        style("Missing").red(),
                        style(path.display()).red(),
                    ));
                }
                VerifyResult::SizeMismatch { expected, actual } => {
                    st.println(format!(
                        "  {} {} (expected {} bytes, actual {} bytes)",
                        style("Size mismatch").red(),
                        path.display(),
                        expected,
                        actual,
                    ));
                }
                VerifyResult::HashMismatch { algo } => {
                    st.println(format!(
                        "  {} {} (hash algorithm: {:?})",
                        style("Hash mismatch").red(),
                        path.display(),
                        algo,
                    ));
                }
                VerifyResult::IoError { message } => {
                    st.println(format!(
                        "  {} {}: {}",
                        style("IO error").red(),
                        path.display(),
                        message,
                    ));
                }
            }
        });
    }

    fn recheck_started(&self, total: usize, session_id: &str, source: &Path) {
        self.phase_started(
            Phase::Recheck,
            Some(total as u64),
            PhaseContext::source(source.to_path_buf()),
        );
        let output = format!(
            "{} Rechecking {} files from session {}\n  Source: {}\n\n{} {}\n         {}\n         {}",
            style("Recheck:").bold().cyan(),
            style(total).cyan(),
            style(session_id),
            style(source.display()),
            style("Caution:").yellow().bold(),
            style("Recheck assumes the source device has not changed since import.").yellow(),
            style(
                "If you took new photos or modified files, filenames may be reused with different content."
            ),
            style("Please review the report carefully before deleting anything.")
        );
        self.println(output);
    }

    fn recheck_item(&self, src: &Path, status: &RecheckStatus) {
        self.with_state(|st| {
            st.pb.inc(1);
            match status {
                RecheckStatus::Ok => {}
                RecheckStatus::SourceModified => {
                    st.println(format!(
                        "  {} {}",
                        style("Source modified").yellow(),
                        src.display(),
                    ));
                }
                RecheckStatus::VaultCorrupted => {
                    st.println(format!(
                        "  {} {}",
                        style("Vault corrupted").red(),
                        src.display(),
                    ));
                }
                RecheckStatus::BothDiverged => {
                    st.println(format!(
                        "  {} {}",
                        style("Both diverged").red().bold(),
                        src.display(),
                    ));
                }
                RecheckStatus::SourceDeleted => {
                    st.println(format!(
                        "  {} {}",
                        style("Source deleted").yellow(),
                        src.display(),
                    ));
                }
                RecheckStatus::VaultDeleted => {
                    st.println(format!(
                        "  {} {}",
                        style("Vault deleted").red(),
                        src.display(),
                    ));
                }
                RecheckStatus::Error { message } => {
                    st.println(format!(
                        "  {} {}: {}",
                        style("Error").red().bold(),
                        src.display(),
                        message,
                    ));
                }
            }
        });
    }

    // ── summaries & hints ─────────────────────────────────────────────────

    fn summary(&self, summary: &Summary) {
        match summary {
            Summary::Import(s) => {
                let mut output = String::from("\n");
                output.push_str(&format!(
                    "{}\n",
                    style("Import operation completed").green().bold()
                ));
                output.push_str(&format!("  Total files processed: {}\n", s.total));
                if s.imported > 0 {
                    output.push_str(&format!(
                        "  {}\n",
                        style(format!("New files imported:  {}", s.imported)).green()
                    ));
                }
                if s.duplicate > 0 {
                    output.push_str(&format!(
                        "  {}\n",
                        style(format!("Duplicates skipped:  {}", s.duplicate)).yellow()
                    ));
                }
                if s.failed > 0 {
                    output.push_str(&format!(
                        "  {}\n",
                        style(format!("Failed:              {}", s.failed)).red()
                    ));
                }
                if let Some(p) = &s.manifest_path {
                    output.push_str(&format!(
                        "  Manifest: {}",
                        style(p.display()).italic().bold()
                    ));
                }
                self.println(output.trim_end());
            }
            Summary::Add(s) => {
                let mut output = format!(
                    "{} {} file(s) added\n",
                    style("Finished:").bold().green(),
                    style(s.added).green()
                );
                if s.duplicate > 0 {
                    output.push_str(&format!(
                        "         {} duplicate(s) skipped\n",
                        style(s.duplicate).yellow()
                    ));
                }
                if s.failed > 0 {
                    output.push_str(&format!(
                        "         {} file(s) failed",
                        style(s.failed).red()
                    ));
                }
                self.println(output.trim_end());
            }
            Summary::Verify(s) => {
                let mut output = String::from("\n");
                output.push_str(&format!("{}\n", style("Verify complete").green().bold()));
                output.push_str(&format!("  Total: {}\n", s.total));
                output.push_str(&format!("  OK: {}\n", style(s.ok).green()));
                if s.missing > 0 {
                    output.push_str(&format!("  Missing: {}\n", style(s.missing).red()));
                }
                if s.size_mismatch > 0 {
                    output.push_str(&format!(
                        "  Size mismatch: {}\n",
                        style(s.size_mismatch).red()
                    ));
                }
                if s.hash_mismatch > 0 {
                    output.push_str(&format!(
                        "  Hash mismatch: {}\n",
                        style(s.hash_mismatch).red()
                    ));
                }
                if s.io_error > 0 {
                    output.push_str(&format!("  IO errors: {}", style(s.io_error).red()));
                }
                self.println(output.trim_end());
            }
            Summary::Recheck(s) => {
                self.println(recheck_summary_text(s));
            }
            Summary::Clone(s) => {
                let mut output = String::from(
                    "
",
                );
                output.push_str(&format!(
                    "{}
",
                    style("Clone complete").green().bold()
                ));
                output.push_str(&format!(
                    "  Files matched: {}
",
                    s.total
                ));
                if s.copied > 0 {
                    output.push_str(&format!(
                        "  {}
",
                        style(format!(
                            "Copied:        {} ({})",
                            s.copied,
                            format_bytes(s.bytes)
                        ))
                        .green()
                    ));
                }
                if s.failed > 0 {
                    output.push_str(&format!(
                        "  {}
",
                        style(format!("Failed:        {}", s.failed)).red()
                    ));
                }
                if let Some(p) = &s.manifest_path {
                    output.push_str(&format!(
                        "  Manifest: {}",
                        style(p.display()).italic().bold()
                    ));
                }
                self.println(output.trim_end());
            }
            Summary::Sync(s) => {
                let mut output = String::from(
                    "
",
                );
                output.push_str(&format!(
                    "{}
",
                    style("Sync complete").green().bold()
                ));
                output.push_str(&format!(
                    "  Identical: {} file(s) already in both vaults
",
                    s.identical
                ));
                if s.copied > 0 {
                    output.push_str(&format!(
                        "  {}
",
                        style(format!(
                            "Copied:    {} file(s) ({})",
                            s.copied,
                            format_bytes(s.bytes)
                        ))
                        .green()
                    ));
                }
                if s.failed > 0 {
                    output.push_str(&format!(
                        "  {}
",
                        style(format!("Failed:    {} file(s)", s.failed)).red()
                    ));
                }
                if s.skipped > 0 {
                    output.push_str(&format!(
                        "  Skipped:   {} file(s) without hashes
",
                        style(s.skipped).yellow()
                    ));
                }
                if s.moved > 0 {
                    output.push_str(&format!(
                        "  Moved:     {} file(s) at different paths (reported only)
",
                        style(s.moved).cyan()
                    ));
                }
                if s.only_dest > 0 {
                    output.push_str(&format!(
                        "  Only local: {} file(s) not in source (kept)
",
                        style(s.only_dest).cyan()
                    ));
                }
                if s.conflicts > 0 {
                    output.push_str(&format!(
                        "  {}
",
                        style(format!(
                            "Conflicts: {} path(s) differ (local kept):",
                            s.conflicts
                        ))
                        .yellow()
                        .bold()
                    ));
                    for p in s.conflict_paths.iter().take(5) {
                        output.push_str(&format!(
                            "    {}
",
                            style(p).yellow()
                        ));
                    }
                    if s.conflict_paths.len() > 5 {
                        output.push_str(&format!(
                            "    ... and {} more
",
                            s.conflict_paths.len() - 5
                        ));
                    }
                }
                if let Some(p) = &s.manifest_path {
                    output.push_str(&format!(
                        "  Manifest: {}",
                        style(p.display()).italic().bold()
                    ));
                }
                self.println(output.trim_end());
            }
            Summary::Update(s) => {
                let mut output = String::from("\n");
                output.push_str(&format!("{}\n", style("Summary:").bold()));
                output.push_str(&format!("  Scanned: {} file(s) on disk\n", s.scanned));
                output.push_str(&format!("  Missing: {} file(s) from DB\n", s.missing));
                output.push_str(&format!(
                    "  Matched: {} file(s) relocated\n",
                    style(s.matched).green()
                ));
                if s.unmatched > 0 {
                    output.push_str(&format!(
                        "  Unmatched: {} file(s) not found\n",
                        style(s.unmatched).yellow()
                    ));
                }
                if s.updated > 0 {
                    output.push_str(&format!(
                        "  Updated: {} file(s) path corrected",
                        style(s.updated).green().bold()
                    ));
                }
                self.println(output.trim_end());
            }
        }
    }

    fn hint(&self, hint: &Hint) {
        match hint {
            Hint::OnlyMoved { moved, vault_root } | Hint::MovedHint { moved, vault_root } => {
                let mut output = String::from("\n");
                output.push_str(&format!("{}\n", style("Note:").bold().cyan()));
                output.push_str(&format!(
                    "  {} file(s) appear to have been moved within the vault.\n",
                    style(moved.len()).cyan()
                ));
                output.push_str(&format!(
                    "  Use {} to update their paths:\n",
                    style("svault update").bold()
                ));
                for (current, old) in moved.iter().take(3) {
                    let rel = current.strip_prefix(vault_root).unwrap_or(current);
                    output.push_str(&format!(
                        "    {} -> {}\n",
                        style(old).color256(244),
                        style(rel.display()).cyan()
                    ));
                }
                if moved.len() > 3 {
                    output.push_str(&format!("    ... and {} more", moved.len() - 3));
                }
                self.println(output.trim_end());
            }
            Hint::NothingToUpdate => {
                self.println("All tracked files exist. Nothing to reconcile.");
            }
            Hint::DryRunMissing { count } => {
                self.println(format!("Files to mark as missing: {}", count));
            }
        }
    }
}

impl EventSink for TerminalSink {
    fn emit(&self, event: &Event) {
        match event {
            Event::PhaseStarted {
                phase,
                total,
                context,
            } => self.phase_started(*phase, *total, context.clone()),
            Event::PhaseFinished { phase } => self.phase_finished(*phase),

            Event::ScanItem {
                path,
                size,
                status,
                error,
                ..
            } => self.scan_item(path, *size, *status, error.as_deref(), self.show_dup),
            Event::Preflight {
                source,
                total,
                new,
                duplicate,
                moved,
                failed,
            } => self.preflight(source, *total, *new, *duplicate, *moved, *failed),

            Event::CopyStarted { src, dst, bytes } => self.copy_started(src, dst, *bytes),
            Event::CopyProgress { .. } => {} // bar shows overall progress only
            Event::CopyFinished { src, error, .. } => self.copy_finished(src, error.as_deref()),

            Event::HashStarted { .. } => {}
            Event::HashFinished { path, bytes, error } => {
                self.hash_finished(path, *bytes, error.as_deref())
            }
            Event::RelocateMatched {
                old_path,
                new_path,
                confidence,
            } => {
                self.with_state(|st| {
                    st.matches
                        .push((old_path.clone(), new_path.clone(), *confidence));
                });
            }

            Event::Progress { done, .. } => {
                self.with_state(|st| st.pb.set_position(*done));
            }
            Event::ApplyError { path, message } => {
                self.println(format!(
                    "{} Failed to update {}: {}",
                    style("Error:").red().bold(),
                    style(path),
                    message
                ));
            }

            Event::RecheckStarted {
                total,
                session_id,
                source,
            } => self.recheck_started(*total, session_id, source),
            Event::RecheckItem { src, status, .. } => self.recheck_item(src, status),

            Event::VerifyItem { path, result } => self.verify_item(path, result),

            Event::SyncPlan {
                source_vault,
                identical,
                to_copy,
                copy_bytes,
                moved,
                only_dest,
                conflicts,
            } => {
                let mut output = format!(
                    "{} Compared against {}

{}
  {}  {}
",
                    style("Sync:").bold().cyan(),
                    style(source_vault.display()).color256(244),
                    style("Plan:").bold(),
                    style(format!("Identical:      {:>6}", identical)).green(),
                    style("already in both vaults")
                );
                if *to_copy > 0 {
                    output.push_str(&format!(
                        "  {}  {}
",
                        style(format!("To copy:        {:>6}", to_copy))
                            .green()
                            .bold(),
                        style(format!("({})", format_bytes(*copy_bytes)))
                    ));
                }
                if *moved > 0 {
                    output.push_str(&format!(
                        "  {}  {}
",
                        style(format!("Moved:          {:>6}", moved)).cyan(),
                        style("same content at different paths")
                    ));
                }
                if *only_dest > 0 {
                    output.push_str(&format!(
                        "  {}  {}
",
                        style(format!("Only local:     {:>6}", only_dest)).cyan(),
                        style("not in source (will be kept)")
                    ));
                }
                if *conflicts > 0 {
                    output.push_str(&format!(
                        "  {}  {}
",
                        style(format!("Conflicts:      {:>6}", conflicts))
                            .yellow()
                            .bold(),
                        style("same path, different content (local kept)")
                    ));
                }
                self.println(output.trim_end());
            }

            Event::Summary(s) => self.summary(s),
            Event::Hint(h) => self.hint(h),
        }
    }
}

impl Drop for TerminalSink {
    fn drop(&mut self) {
        let _ = self.multi.clear();
    }
}

impl Default for TerminalSink {
    fn default() -> Self {
        Self::new(false)
    }
}

/// Render the recheck tally as a text block (shared by terminal tests).
fn recheck_summary_text(s: &RecheckSummary) -> String {
    let mut output = String::new();
    output.push_str(&format!("{}\n", style("Results:").bold()));
    output.push_str(&format!("  {} OK\n", style(format!("{:>4}", s.ok)).green()));
    if s.source_modified > 0 {
        output.push_str(&format!(
            "  {} Source modified\n",
            style(format!("{:>4}", s.source_modified)).yellow()
        ));
    }
    if s.vault_corrupted > 0 {
        output.push_str(&format!(
            "  {} Vault corrupted\n",
            style(format!("{:>4}", s.vault_corrupted)).red()
        ));
    }
    if s.both_diverged > 0 {
        output.push_str(&format!(
            "  {} Both diverged\n",
            style(format!("{:>4}", s.both_diverged)).red()
        ));
    }
    if s.source_deleted > 0 {
        output.push_str(&format!(
            "  {} Source deleted\n",
            style(format!("{:>4}", s.source_deleted)).yellow()
        ));
    }
    if s.vault_deleted > 0 {
        output.push_str(&format!(
            "  {} Vault deleted\n",
            style(format!("{:>4}", s.vault_deleted)).red()
        ));
    }
    if s.errors > 0 {
        output.push_str(&format!(
            "  {} Errors\n",
            style(format!("{:>4}", s.errors)).red()
        ));
    }
    if s.sha256_verified > 0 {
        output.push_str(&format!(
            "  ({} files verified with SHA-256)\n",
            s.sha256_verified
        ));
    }
    output.push('\n');
    output.push_str(&format!(
        "{} Report written to {}",
        style("Report:").bold(),
        style(s.report_path.display()).italic().bold()
    ));
    output
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn truncate_path(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("...{}...", &s[s.len() - (max - 3)..])
    } else {
        s.to_string()
    }
}
