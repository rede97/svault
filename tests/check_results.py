#!/usr/bin/env python3
"""check_results.py — Validate svault import results against test_rules.json.

Called by run_tests.sh after import. Reads:
  --rules   test_rules.json (expected outcomes)
  --db      db_files.json   (sqlite3 -json dump of files table)
  --vault-root  path to vault dir (to inspect actual file layout)
  --log     import_normal.log
  --output  check_report.json (written by this script)

Exit code 0 = all PASS, 1 = any FAIL.
"""
import argparse
import json
import os
import sys
from pathlib import Path


def parse_args():
    p = argparse.ArgumentParser()
    p.add_argument("--rules", required=True)
    p.add_argument("--db", required=True)
    p.add_argument("--vault-root", required=True)
    p.add_argument("--log", required=True)
    p.add_argument("--output", required=True)
    return p.parse_args()


def load_json(path):
    with open(path) as f:
        return json.load(f)


def load_text(path):
    try:
        with open(path) as f:
            return f.read()
    except FileNotFoundError:
        return ""


def find_db_row_by_filename(db_rows, filename):
    """Find DB rows whose path ends with the given filename."""
    return [r for r in db_rows if Path(r["path"]).name == filename]


def check_scenario(scenario, db_rows, vault_root, import_log):
    """Validate one scenario. Returns a result dict."""
    sid = scenario["id"]
    src = scenario["src"]
    expected_status = scenario.get("expected_status")
    checks = []
    passed = True

    rows = find_db_row_by_filename(db_rows, src)

    # --- Check: DB row presence ---
    expect_db = scenario.get("expected_db_row", True)
    has_db = len(rows) > 0
    if expect_db:
        ok = has_db
        checks.append({"check": "db_row_exists", "pass": ok,
                        "detail": f"Found {len(rows)} row(s) in DB"})
        passed = passed and ok
    else:
        ok = not has_db
        checks.append({"check": "db_row_absent", "pass": ok,
                        "detail": f"Expected no DB row; found {len(rows)}"})
        passed = passed and ok

    # --- Check: status in DB ---
    if expected_status == "imported" and rows:
        for row in rows:
            ok = row.get("status") == "imported"
            checks.append({"check": "db_status", "pass": ok,
                            "detail": f"DB status={row.get('status')!r}"})
            passed = passed and ok

    # --- Check: crc32c_val non-null ---
    if scenario.get("expected_crc32c_nonnull") and rows:
        for row in rows:
            val = row.get("crc32c_val")
            ok = val is not None and val != 0
            checks.append({"check": "crc32c_nonnull", "pass": ok,
                            "detail": f"crc32c_val={val!r}"})
            passed = passed and ok

    # --- Check: dest path contains expected substrings ---
    if rows and scenario.get("expected_dest_contains"):
        for row in rows:
            dest = row.get("path", "")
            for fragment in scenario["expected_dest_contains"]:
                ok = fragment in dest
                checks.append({"check": f"dest_contains:{fragment!r}", "pass": ok,
                                "detail": f"dest={dest!r}"})
                passed = passed and ok

    # --- Check: dest path does NOT contain forbidden substrings ---
    if rows and scenario.get("expected_dest_not_contains"):
        for row in rows:
            dest = row.get("path", "")
            for fragment in scenario["expected_dest_not_contains"]:
                ok = fragment not in dest
                checks.append({"check": f"dest_not_contains:{fragment!r}", "pass": ok,
                                "detail": f"dest={dest!r}"})
                passed = passed and ok

    # --- Check: duplicate logged in import log ---
    if expected_status == "duplicate":
        dup_reasons = scenario.get("expected_dup_reason", [])
        # Just verify the file was NOT copied to vault (no DB row)
        if not rows:
            checks.append({"check": "duplicate_not_in_db", "pass": True,
                            "detail": "Correctly absent from files table"})
        else:
            checks.append({"check": "duplicate_not_in_db", "pass": False,
                            "detail": f"Unexpectedly found {len(rows)} row(s) in DB"})
            passed = False

    # --- Check: actual file exists in vault (for imported) ---
    if expected_status == "imported" and rows:
        for row in rows:
            dest_path = Path(vault_root) / row["path"]
            ok = dest_path.exists()
            checks.append({"check": "dest_file_exists", "pass": ok,
                            "detail": f"path={dest_path}"})
            passed = passed and ok

    return {
        "id": sid,
        "src": src,
        "scenario": scenario.get("scenario", ""),
        "pass": passed,
        "checks": checks,
    }


def main():
    args = parse_args()
    rules = load_json(args.rules)
    db_rows = load_json(args.db)
    import_log = load_text(args.log)
    vault_root = args.vault_root

    results = []
    for scenario in rules.get("scenarios", []):
        result = check_scenario(scenario, db_rows, vault_root, import_log)
        results.append(result)
        status = "PASS" if result["pass"] else "FAIL"
        color = "\033[1;32m" if result["pass"] else "\033[1;31m"
        reset = "\033[0m"
        print(f"  {color}{status}{reset}  [{result['id']}] {result['scenario']}")
        if not result["pass"]:
            for c in result["checks"]:
                if not c["pass"]:
                    print(f"         - {c['check']}: {c['detail']}")

    total = len(results)
    passed = sum(1 for r in results if r["pass"])
    failed = total - passed

    print()
    print(f"Results: {passed}/{total} passed", end="")
    if failed:
        print(f", {failed} FAILED")
    else:
        print(" — all OK")

    report = {
        "total": total,
        "passed": passed,
        "failed": failed,
        "results": results,
    }
    with open(args.output, "w") as f:
        json.dump(report, f, indent=2)
    print(f"Report written to {args.output}")

    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()

