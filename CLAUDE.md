# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project overview

Svault is a content-addressed multimedia archive tool written in Rust. It de-duplicates files by SHA-256 hash, stores them in a content-addressable vault, and tracks everything in an event-sourced SQLite database with a SHA-256 hash chain for tamper detection. All code is AI-written.

## Commands

```bash
# Build
cargo build                    # debug
cargo build --release          # release
./scripts/build-release.sh     # release (all targets, including zigbuild for CentOS 7)

# Lint
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings

# Unit tests
cargo test --workspace                         # all Rust unit tests
cargo test -p svault-core                       # core only
cargo test -p svault-core hash                   # specific module
cargo test -p svault-cli                         # CLI only

# E2E tests (must run from tests/e2e/)
cd tests/e2e && bash run.sh                     # default: debug build, exclude FUSE
cd tests/e2e && bash run.sh --release            # release build
cd tests/e2e && bash run.sh --fuse               # include FUSE fault-injection tests
cd tests/e2e && bash run.sh -k test_import       # filter by test name
cd tests/e2e && bash run.sh --test-dir /mnt/ext4 # test on specific filesystem
cd tests/e2e && bash run.sh --verbose            # verbose output

# Run a single E2E test file
cd tests/e2e && bash run.sh test_import.py

# Manual binary run (for dev; use RAMDisk, not project dir)
cargo run -p svault -- status
```

## CRITICAL: Never run svault init/import in the project directory

Always use RAMDisk or a dedicated test directory. The E2E framework handles this automatically. Running `svault init` in the project root will pollute it with vault metadata.

## Architecture

### Workspace layout

```
svault-core/    # library crate — all core logic, no CLI dependency (clap is optional via "cli" feature)
svault-cli/     # binary crate — CLI parsing (clap), terminal output, progress bars (indicatif)
svault-mtp/     # MTP daemon crate — MTP device WebDAV serving (experimental)
```

`svault-cli` depends on `svault-core` with `features = ["cli"]`. Core must never import from CLI.

### Module map (svault-core)

| Module | Purpose |
|--------|---------|
| `hash` | Three-tier hashing: CRC32C (fast fingerprint) → XXH3-128 (collision-resistant ID) → SHA-256 (cryptographic). SHA-256 is lazy — computed only on collision. |
| `pipeline` | 5-stage import pipeline: `scan` (directory walk) → `crc` (CRC32C) → `lookup` (DB dedup check) → `hash` (XXH3/SHA-256 for new files only) → `insert` (atomic DB write + manifest). Shared by `import` and `add` commands. |
| `db` | Event-sourced SQLite with SHA-256 hash chain. Each mutation appends an event; the chain can be verified via `db verify-chain`. Sub-modules: `stats`, `dump`. |
| `fs` | File transfer with fallback chain: reflink (CoW) → hardlink → stream copy. Automatic detection of filesystem capabilities. |
| `import` | Import logic: EXIF parsing, path resolution, staging area, recheck, update. Most sub-modules are under `import/`. |
| `media` | Media format detection: binding (Live Photo, RAW+JPEG pairs), CRC, format identifiers, RAW identification, video metadata. |
| `verify` | Integrity verification: background hash computation, hardlink→reflink upgrade. |
| `history` | Import session and item history queries. |
| `config` | TOML-based per-vault configuration (`svault.toml`). |
| `context` | Vault discovery: walks up from CWD looking for `.svault/vault.db`. |
| `lock` | Advisory file locking via `.svault/lock` (fs2) to prevent concurrent modifications. |
| `reporting` | Typed phase reporter traits organized by command (`scan.rs`, `hash.rs`, `sync.rs`, `clone.rs`, etc.) — core defines traits, CLI provides implementations via `ReporterBuilder`. |
| `status` | Vault statistics aggregation. |
| `sync` | Two-phase sync engine: SHA-256 diff (`diff.rs`) → file transfer (`transfer.rs`). Powers both `svault sync` (vault→vault) and `svault clone` (vault→directory), sharing transfer infrastructure with import. |

### CLI command dispatch

`main.rs` parses CLI with clap derive, extracts global flags (`output`, `dry_run`, `yes`), then routes to the matching command module under `commands/`. Each command gets its own file. Never pass `&Cli` into command modules — pass individual parameters.

### Key design invariants

- **No delete command** — safety-first: users delete sources manually after verifying the vault.
- **Vault self-protection** — import scan automatically skips `.svault/` directories and the vault root subtree.
- **Process locking** — any command that modifies state acquires an advisory lock on `<vault>/.svault/lock`.
- **Append-only DB** — all mutations are events; the event hash chain enables tamper detection.
- **Lazy SHA-256** — computed only when CRC32C+XXH3 collisions occur, saving ~99% of work.
- **reflink-first** — the default transfer strategy is `reflink` (CoW). `hardlink` is opt-in and not in the default list. `copy` is the unconditional fallback.

### Reporting architecture (3-layer)

1. **Traits** (`svault-core/src/reporting/`) — 14+ typed phase reporter traits in per-command files (e.g. `scan.rs`, `sync.rs`, `clone.rs`). `mod.rs` is a thin re-export layer.
2. **Builder** (`svault-core/src/reporting/builder.rs`) — `ReporterBuilder` trait with associated types for each phase reporter. CLI/GUI implement this to construct their concrete reporters.
3. **Implementations** (`svault-cli/src/reporting/terminal.rs`, `json.rs`, `pipe.rs`, `path.rs`) — `TerminalReporterBuilder`, `JsonReporterBuilder`, etc. with terminal progress bars, JSON streams, pipeable text.

Each reporter is obtained from the builder, used for exactly one phase, then dropped. `Drop` implementations guarantee progress indicators are cleared even on panic. `Interactor` uses `<I: Interactor>` generics (zero-cost) rather than `&dyn Interactor`. Always build a full `String` before calling `pb.println()` to avoid multi-threaded output interleaving.

### Rust edition and conventions

- Edition 2024 (workspace-level)
- All public API items must have `///` doc comments
- Commit messages: `<type>: <subject>` (types: `feat`, `fix`, `docs`, `test`, `refactor`)
- After adding tests, update `docs/UNIT_TESTS.md`

### E2E testing notes

- Tests live in `tests/e2e/` and use pytest with Hypothesis for property-based testing
- Key fixtures in `conftest.py`: `VaultEnv`, `RamDisk`, `source_factory`
- Never use `piexif` to write EXIF — always use `exiftool` CLI (ensures real camera EXIF compatibility)
- FUSE tests (`test_import_interruption.py`, `test_import_disk_full.py`) require `--fuse` flag
- `run.sh` auto-builds the binary before running tests

### CI

GitHub Actions on push/PR to `main`: builds and runs `cargo test --workspace` on ubuntu, macos, windows. Release workflow triggers on version tags, producing binaries for 5 targets (x86_64/aarch64 Linux, x86_64/aarch64 macOS, x86_64 Windows).
