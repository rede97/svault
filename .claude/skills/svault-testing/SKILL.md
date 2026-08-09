---
name: svault-testing
description: Run and extend Svault's test suites (Rust unit tests, Python E2E, FUSE fault injection). Use when executing tests, adding test cases, or debugging test environment issues (RAMDisk, exiftool, FUSE, loopback).
---

# Svault Testing Guide

Operational knowledge for Svault's test environment. Read this before running
or writing any test. Suite constitution (one contract per test, no weak
assertions, unique coverage ownership) lives in `tests/e2e/README.md`;
failure-injection criteria live in `docs/failure-handling.md` §8 — those two
are the authorities; this skill is the how-to.

## Hard rules

1. **NEVER run `svault init` / `svault import` in the project directory.**
   All tests run in a tmpfs RAMDisk (`/tmp/svault-ramdisk`) via `run.sh`.
2. **exiftool is a mandatory E2E dependency** — the standard tool for writing
   EXIF fixtures. `run.sh` hard-checks it at entry; `source_factory` calls
   `pytest.fail` when EXIF writing is requested but exiftool is missing
   (no silent degradation). NEVER use Python EXIF libraries (piexif etc.).
3. New behavior needs a unit test or E2E test; core tests must work with
   `NoopSink` (no terminal). Update the stats in `docs/UNIT_TESTS.md`.

## Commands

```bash
# Rust unit tests (zero clippy warnings required)
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# E2E (pytest; run.sh builds the debug binary and mounts the RAMDisk)
cd tests/e2e
bash run.sh                       # full suite, FUSE excluded
bash run.sh --fuse                # include FUSE fault-injection tests
bash run.sh -k "test_raw"         # select by keyword
bash run.sh --verbose             # verbose output
bash run.sh --test-dir /mnt/ext4  # run against an existing directory
bash run.sh --cleanup             # remove test dir afterwards
sudo bash run.sh -k "test_cross_fs"   # tests needing root

# Authoritative test listings (never hand-maintain test tables)
cargo test -p svault-core -- --list
.venv/bin/pytest --collect-only -q
```

## Environment prerequisites

| Requirement | Needed by | Setup |
|---|---|---|
| tmpfs RAMDisk | all E2E | `run.sh` mounts it automatically (`tests/setup_ramdisk.sh` for manual use) |
| exiftool | EXIF fixtures | `apt install libimage-exiftool-perl` |
| FUSE + `user_allow_other` | `fuse_tests/` | `echo user_allow_other \| sudo tee -a /etc/fuse.conf` (without it every FUSE test errors with mount timeout) |
| loopback + sudo | `test_import_disk_full.py`, cross-fs | `sudo -n` capable loop devices |
| strace with inject | signal-interruption tests | skipped automatically when unavailable |

After FUSE tests, stale mounts may remain: `mount | grep svault` and
`fusermount3 -u <mount>` to clean. A full RAMDisk breaks `init` with
misleading errors — check `df -h /tmp/svault-ramdisk` first.

## Layout

```text
tests/e2e/
  conftest.py          # VaultEnv fixture (vault_dir/source_dir/binary/
                       #   import_dir/db_files/db_query/run), fixture helpers
  fixtures/            # binary fixtures + source_factory (exiftool writing)
  run.sh               # entry point (RAMDisk, build, pytest)
  test_*.py            # one contract per test file area (see README table)
  fuse_tests/          # FUSE fault injection (pause/error/corrupt/delay)
```

## Writing tests

- **One test = one observable contract**; assert exit codes, event stream,
  DB state, and filesystem state precisely — no vacuous `if` assertions,
  no skip-on-missing fallbacks. See the constitution in `tests/e2e/README.md`.
- Failure-behavior criteria MUST match `docs/failure-handling.md`
  (exit-code contract G4, per-file isolation G3, staging invariant G7).
  If code and doc disagree, fix one of them immediately — never let both stand.
- Unit tests live inline in `src/` modules (`#[cfg(test)]`), use `tempfile`
  and `Db::open_in_memory()`, emit to `NoopSink`, confirm via `YesInteractor`.
- Session-journal layout (assert against this, not legacy paths):
  `.svault/sessions/{import,sync}/<ts-id>/{plan.json,manifest.json}`,
  import payload staging under `staging/`, recheck reports under
  `sessions/recheck/<ts-id>/report.json`.
- Interrupted-session residue is **reported, never deleted** — assert the
  hint (`session_residue` in JSON mode) and the residue's continued existence.
