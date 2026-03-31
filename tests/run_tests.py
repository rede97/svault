#!/usr/bin/env python3
"""run_tests.py — svault import pipeline test framework.

Usage:
    python3 tests/run_tests.py [options]

Options:
    --no-build        Skip cargo build (use existing binary)
    --keep            Keep RAMDisk mounted after run (for inspection)
    --chaos           Also run chaos scenarios
    --ramdisk-size N  tmpfs size string, default '128m'
    --ramdisk-dir P   Mount point, default /tmp/svault-ramdisk
    --verbose         Print per-check details

Requirements:
    - sudo (for mount/umount) or user namespace
    - sqlite3 CLI
    - .venv with Pillow + piexif (only for gen_fixtures.py)
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------
SCRIPT_DIR = Path(__file__).parent.resolve()
PROJECT_ROOT = SCRIPT_DIR.parent
FIXTURES_DIR = SCRIPT_DIR / "fixtures"
PYTHON = SCRIPT_DIR / ".venv" / "bin" / "python3"


# ---------------------------------------------------------------------------
# ANSI helpers
# ---------------------------------------------------------------------------
def _c(code: str, text: str) -> str:
    return f"\033[{code}m{text}\033[0m"

def log(msg: str)  -> None: print(_c("1;36", "=="), msg)
def ok(msg: str)   -> None: print(_c("1;32", "  OK"), msg)
def warn(msg: str) -> None: print(_c("1;33", "WARN"), msg)
def fail(msg: str) -> None: print(_c("1;31", "FAIL"), msg, file=sys.stderr)


# ---------------------------------------------------------------------------
# RAMDisk
# ---------------------------------------------------------------------------
class RamDisk:
    """Mount / unmount a tmpfs RAMDisk."""

    def __init__(self, path: Path, size: str = "128m"):
        self.path = path
        self.size = size
        self._mounted = False

    def mount(self) -> None:
        self.path.mkdir(parents=True, exist_ok=True)
        if self._is_mounted():
            log(f"RAMDisk already mounted at {self.path}, reusing")
            self._mounted = True
            return
        cmd = ["mount", "-t", "tmpfs", "-o", f"size={self.size}", "tmpfs", str(self.path)]
        try:
            subprocess.run(cmd, check=True, capture_output=True)
        except subprocess.CalledProcessError:
            subprocess.run(["sudo"] + cmd, check=True)
        # Fix ownership if mounted as root
        uid, gid = os.getuid(), os.getgid()
        subprocess.run(["sudo", "chown", f"{uid}:{gid}", str(self.path)], check=False)
        self._mounted = True
        ok(f"RAMDisk mounted at {self.path} ({self.size})")

    def umount(self) -> None:
        if not self._mounted:
            return
        if not self._is_mounted():
            return
        try:
            subprocess.run(["umount", str(self.path)], check=True, capture_output=True)
        except subprocess.CalledProcessError:
            subprocess.run(["sudo", "umount", str(self.path)], check=False)
        log(f"RAMDisk unmounted: {self.path}")
        self._mounted = False

    def _is_mounted(self) -> bool:
        result = subprocess.run(["mountpoint", "-q", str(self.path)])
        return result.returncode == 0


# ---------------------------------------------------------------------------
# VaultEnv — set up binary + vault inside RAMDisk
# ---------------------------------------------------------------------------
class VaultEnv:
    """Prepare the vault environment inside the RAMDisk."""

    def __init__(self, ramdisk: Path, binary: Path):
        self.ramdisk = ramdisk
        self.binary_src = binary
        self.bin_dir = ramdisk / "bin"
        self.vault_dir = ramdisk / "vault"
        self.source_dir = ramdisk / "source"
        self.chaos_dir = ramdisk / "chaos"
        self.results_dir = ramdisk / "results"
        self.binary = self.bin_dir / binary.name

    def setup(self) -> None:
        for d in [self.bin_dir, self.vault_dir, self.source_dir,
                  self.chaos_dir, self.results_dir]:
            d.mkdir(parents=True, exist_ok=True)

        # Copy binary
        shutil.copy2(self.binary_src, self.binary)
        self.binary.chmod(0o755)

        # Regenerate fixtures
        log("Regenerating test fixtures")
        subprocess.run(
            [str(PYTHON), str(SCRIPT_DIR / "gen_fixtures.py")],
            check=True,
        )

        # Copy fixtures into RAMDisk (including subdirectories)
        source_fixture_dir = FIXTURES_DIR / "source"
        for f in source_fixture_dir.rglob("*"):
            if f.is_file():
                rel_path = f.relative_to(source_fixture_dir)
                dest = self.source_dir / rel_path
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(f, dest)
        if (FIXTURES_DIR / "chaos").exists():
            shutil.copytree(FIXTURES_DIR / "chaos", self.chaos_dir, dirs_exist_ok=True)
        shutil.copy2(FIXTURES_DIR / "test_rules.json", self.results_dir / "test_rules.json")
        ok("Assets copied into RAMDisk")

        # Init vault — svault init uses CWD, so run it from vault_dir
        # Clean up any leftover vault from a previous run first
        svault_meta = self.vault_dir / ".svault"
        if svault_meta.exists():
            shutil.rmtree(svault_meta)
        config_file = self.vault_dir / "svault.toml"
        if config_file.exists():
            config_file.unlink()
        log("Initializing vault")
        subprocess.run(
            [str(self.binary), "init"],
            cwd=str(self.vault_dir),
            check=True, capture_output=True,
        )
        ok("Vault initialized")

    def svault(self, *args: str, capture: bool = False) -> subprocess.CompletedProcess:
        """Run svault from vault_dir as CWD so vault discovery always works."""
        cmd = [str(self.binary)] + list(args)
        if capture:
            return subprocess.run(cmd, capture_output=True, text=True,
                                  cwd=str(self.vault_dir))
        return subprocess.run(cmd, text=True, cwd=str(self.vault_dir))

# ---------------------------------------------------------------------------
# DB dump helper
# ---------------------------------------------------------------------------
def dump_db(vault_dir: Path, out_path: Path) -> list[dict]:
    """Query the files table using Python's built-in sqlite3 module."""
    import sqlite3
    db_path = vault_dir / ".svault" / "vault.db"
    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    cur = conn.execute(
        "SELECT path, size, mtime, crc32c_val, xxh3_128, sha256, status, imported_at FROM files;"
    )
    def _coerce(v: Any) -> Any:
        if isinstance(v, bytes):
            return v.hex()
        return v
    rows = [{k: _coerce(v) for k, v in dict(r).items()} for r in cur.fetchall()]
    conn.close()
    out_path.write_text(json.dumps(rows, indent=2))
    return rows


# ---------------------------------------------------------------------------
# Result checker
# ---------------------------------------------------------------------------
def find_db_rows(db_rows: list[dict], filename: str) -> list[dict]:
    """Find DB rows by filename. filename may include subdirs like 'camera_a/DSC0001.jpg'."""
    # Extract just the basename for matching (import flattens subdirs)
    basename = Path(filename).name
    stem = Path(filename).stem
    ext = Path(filename).suffix
    
    matches = []
    for r in db_rows:
        path_name = Path(r["path"]).name
        # Exact match
        if path_name == basename:
            matches.append(r)
        # Renamed match: stem.N.ext (e.g., DSC0001.1.jpg)
        elif path_name.startswith(stem + ".") and path_name.endswith(ext):
            matches.append(r)
    return matches


def check_scenario(scenario: dict, db_rows: list[dict], vault_dir: Path,
                   import_log: str, verbose: bool) -> dict:
    sid = scenario["id"]
    src = scenario.get("src", "")
    checks: list[dict] = []
    passed = True

    def record(name: str, result: bool, detail: str) -> None:
        nonlocal passed
        checks.append({"check": name, "pass": result, "detail": detail})
        passed = passed and result
        if verbose:
            sym = _c("1;32", "PASS") if result else _c("1;31", "FAIL")
            print(f"    [{sym}] {name}: {detail}")

    rows = find_db_rows(db_rows, src)
    expect_db = scenario.get("expected_db_row", True)

    # DB row presence
    if expect_db:
        record("db_row_exists", len(rows) > 0, f"{len(rows)} row(s) found")
    else:
        record("db_row_absent", len(rows) == 0, f"{len(rows)} row(s) found (expected 0)")

    expected_status = scenario.get("expected_status")

    if expected_status == "imported" and rows:
        for row in rows:
            record("db_status_imported",
                   row.get("status") == "imported",
                   f"status={row.get('status')}")

    # crc32c_val non-NULL
    if scenario.get("expected_crc32c_nonnull") and rows:
        for row in rows:
            record("crc32c_nonnull",
                   row.get("crc32c_val") is not None,
                   f"crc32c_val={row.get('crc32c_val')}")

    # dest path contains expected substrings
    dest_contains = scenario.get("expected_dest_contains", [])
    if dest_contains and rows:
        # For conflict tests, check that at least one row matches
        require_all_rows = not scenario.get("check_renamed", False)
        
        for substr in dest_contains:
            if require_all_rows:
                # All rows must contain the substring
                all_match = all(substr in row.get("path", "") for row in rows)
                record(f"dest_contains:{substr}",
                       all_match,
                       f"all paths contain '{substr}'")
            else:
                # At least one row must contain the substring
                any_match = any(substr in row.get("path", "") for row in rows)
                record(f"dest_contains:{substr}",
                       any_match,
                       f"at least one path contains '{substr}'")

    # dest path must NOT contain certain substrings
    dest_not_contains = scenario.get("expected_dest_not_contains", [])
    if dest_not_contains and rows:
        # For each substring, all rows must NOT contain it
        for substr in dest_not_contains:
            all_match = all(substr not in row.get("path", "") for row in rows)
            record(f"dest_not_contains:{substr}",
                   all_match,
                   f"no path contains '{substr}'")
    
    # Check for specific original filename (not renamed)
    check_original = scenario.get("check_original_name")
    if check_original and rows:
        found = any(Path(r["path"]).name == check_original for r in rows)
        record("original_name_exists", found, f"looking for {check_original}")
    
    # Check for renamed file (DSC0001.1.jpg style)
    check_renamed_from = scenario.get("check_renamed_from")
    if check_renamed_from and rows:
        import re
        stem = Path(check_renamed_from).stem
        ext = Path(check_renamed_from).suffix
        pattern = re.compile(re.escape(stem) + r'\.\d+' + re.escape(ext))
        found = any(pattern.search(Path(r["path"]).name) for r in rows)
        record("renamed_file_exists", found, f"looking for {stem}.N{ext}")

    # Duplicate: must appear in import log
    if expected_status == "duplicate":
        dup_reasons = scenario.get("expected_dup_reason", [])
        found_dup_log = any(r in import_log for r in dup_reasons) or "duplicate" in import_log.lower()
        record("dup_logged", found_dup_log, "duplicate mentioned in import log")

    return {"id": sid, "src": src, "pass": passed, "checks": checks}


def run_checks(rules: dict, db_rows: list[dict], vault_dir: Path,
               import_log: str, output_path: Path, verbose: bool) -> bool:
    results = []
    for scenario in rules.get("scenarios", []):
        if verbose:
            print(f"  Checking {scenario['id']} ({scenario.get('scenario', '')}):")
        r = check_scenario(scenario, db_rows, vault_dir, import_log, verbose)
        sym = _c("1;32", "PASS") if r["pass"] else _c("1;31", "FAIL")
        print(f"  [{sym}] {r['id']}: {r['src']}")
        results.append(r)

    total = len(results)
    n_passed = sum(1 for r in results if r["pass"])
    n_failed = total - n_passed

    report = {"total": total, "passed": n_passed, "failed": n_failed, "results": results}
    output_path.write_text(json.dumps(report, indent=2))
    print(f"\nResults: {n_passed}/{total} passed" + (f", {n_failed} FAILED" if n_failed else " — all OK"))
    print(f"Report: {output_path}")
    return n_failed == 0


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="svault import pipeline test runner")
    p.add_argument("--no-build",     action="store_true", help="Skip cargo build")
    p.add_argument("--keep",         action="store_true", help="Keep RAMDisk after run")
    p.add_argument("--chaos",        action="store_true", help="Run chaos scenarios")
    p.add_argument("--ramdisk-size", default="128m",      metavar="SIZE")
    p.add_argument("--ramdisk-dir",  default="/tmp/svault-ramdisk", metavar="PATH")
    p.add_argument("--verbose",      action="store_true", help="Print per-check details")
    return p.parse_args()


def main() -> None:
    args = parse_args()
    ramdisk_path = Path(args.ramdisk_dir)
    ramdisk = RamDisk(ramdisk_path, args.ramdisk_size)

    # Step 1: Build
    if not args.no_build:
        log("Building svault (release)")
        subprocess.run(
            ["cargo", "build", "--release", "-p", "svault-cli", "-q"],
            cwd=PROJECT_ROOT, check=True,
        )
        ok("Build complete")

    binary = PROJECT_ROOT / "target" / "release" / "svault"
    if not binary.exists():
        fail(f"Binary not found: {binary}")
        sys.exit(1)

    # Step 2: Mount RAMDisk
    ramdisk.mount()
    vault_env = VaultEnv(ramdisk_path, binary)

    try:
        # Step 3–4: Setup
        vault_env.setup()

        # Step 5: Normal import
        log("Running import (normal scenarios)")
        r1 = vault_env.svault("--output", "json", "import", "--yes",
                               str(vault_env.source_dir), capture=True)
        (vault_env.results_dir / "import_normal.stdout").write_text(r1.stdout)
        (vault_env.results_dir / "import_normal.log").write_text(r1.stderr)
        ok("Import (normal) complete")

        # Step 6: Second import — dedup check
        log("Running import again (dedup detection)")
        r2 = vault_env.svault("--output", "json", "import", "--yes",
                               str(vault_env.source_dir), capture=True)
        (vault_env.results_dir / "import_dedup.stdout").write_text(r2.stdout)
        (vault_env.results_dir / "import_dedup.log").write_text(r2.stderr)
        ok("Import (dedup) complete")

        # Step 7: Dump DB
        log("Dumping DB state")
        db_rows = dump_db(vault_env.vault_dir, vault_env.results_dir / "db_files.json")
        ok(f"DB: {len(db_rows)} rows")

        # Optional chaos scenarios
        if args.chaos:
            log("Running chaos scenarios")
            chaos_source = vault_env.chaos_dir / "chaos"

            # c2: subdir import
            subdir = chaos_source / "moved_subdirectory"
            if subdir.exists():
                log("  c2: import from subdirectory")
                vault_env.svault("import", "--yes", str(subdir))

            # c3: truncated file
            corrupt = chaos_source / "interrupted_copy.jpg"
            if corrupt.exists():
                log("  c3: import truncated JPEG")
                td = ramdisk_path / "chaos_corrupt"
                td.mkdir(exist_ok=True)
                shutil.copy2(corrupt, td / corrupt.name)
                vault_env.svault("import", "--yes", str(td))

            db_rows = dump_db(vault_env.vault_dir,
                              vault_env.results_dir / "db_files_after_chaos.json")
            ok("Chaos scenarios complete")

        # Step 8: Check results
        log("Validating results")
        rules = json.loads((vault_env.results_dir / "test_rules.json").read_text())
        import_log = (vault_env.results_dir / "import_normal.log").read_text()
        all_passed = run_checks(
            rules, db_rows, vault_env.vault_dir, import_log,
            vault_env.results_dir / "check_report.json",
            args.verbose,
        )

    finally:
        if not args.keep:
            ramdisk.umount()
        else:
            log(f"--keep: inspect results at {ramdisk_path}/results/")

    sys.exit(0 if all_passed else 1)


if __name__ == "__main__":
    main()

