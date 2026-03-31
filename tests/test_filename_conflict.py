#!/usr/bin/env python3
"""
Test filename conflict handling - same filename, different content.

This tests the scenario where two cameras with the same model import files
with the same filename (e.g., DSC0001.jpg) on the same day. The second file
should be automatically renamed to DSC0001.1.jpg.
"""

import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path


def create_test_jpeg(path: Path, content_marker: str):
    """Create a minimal JPEG file with unique content."""
    # JPEG header + unique content
    header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
    path.write_bytes(header + content_marker.encode())


def run_svault(args: list, cwd: Path = None) -> subprocess.CompletedProcess:
    """Run svault command."""
    svault_bin = Path(__file__).parent.parent / "target" / "release" / "svault"
    cmd = [str(svault_bin)] + args
    return subprocess.run(cmd, capture_output=True, text=True, cwd=cwd)


def test_filename_conflict():
    """Test that files with same name but different content get renamed."""
    # Setup test directories
    test_dir = Path("/tmp/svault_conflict_pytest")
    if test_dir.exists():
        shutil.rmtree(test_dir)
    
    source_dir = test_dir / "source"
    vault_dir = test_dir / "vault"
    
    # Create source files with same name but different content
    (source_dir / "camera_a").mkdir(parents=True)
    (source_dir / "camera_b").mkdir(parents=True)
    
    create_test_jpeg(source_dir / "camera_a" / "DSC0001.jpg", "CAMERA_A_TOKYO_GPS")
    create_test_jpeg(source_dir / "camera_b" / "DSC0001.jpg", "CAMERA_B_LONDON_GPS")
    
    print("Created test files:")
    for f in sorted(source_dir.rglob("*.jpg")):
        print(f"  {f.relative_to(source_dir)}: {f.stat().st_size} bytes")
    
    # Init vault
    vault_dir.mkdir(parents=True)
    result = run_svault(["init"], cwd=vault_dir)
    if result.returncode != 0:
        print(f"ERROR: vault init failed: {result.stderr}")
        return False
    
    # Import files
    print("\nImporting files...")
    result = run_svault(["import", "--yes", str(source_dir)], cwd=vault_dir)
    if result.returncode != 0:
        print(f"ERROR: import failed: {result.stderr}")
        return False
    
    print(result.stderr)  # Show import output
    
    # Check results
    vault_files = list(vault_dir.rglob("*.jpg"))
    db_path = vault_dir / ".svault" / "vault.db"
    
    conn = sqlite3.connect(db_path)
    cursor = conn.execute("SELECT COUNT(*) FROM files")
    db_count = cursor.fetchone()[0]
    conn.close()
    
    print(f"\nResults:")
    print(f"  Files in vault: {len(vault_files)}")
    print(f"  Files in DB: {db_count}")
    
    for f in sorted(vault_files):
        print(f"    - {f.name}")
    
    # Cleanup
    shutil.rmtree(test_dir)
    
    # Verify
    if len(vault_files) == 2 and db_count == 2:
        print("\n✓ Test PASSED: Both files imported with unique names")
        return True
    else:
        print(f"\n✗ Test FAILED: Expected 2 files, got {len(vault_files)} (vault) and {db_count} (db)")
        return False


def test_mtp_storage_listing():
    """Test that MTP storage listing works correctly."""
    # This requires an actual MTP device, so we just verify the command exists
    result = run_svault(["mtp", "ls"])
    # Should either list devices or show error about no device
    if result.returncode == 0 or "No MTP devices found" in result.stderr or "MTP" in result.stderr:
        print("\n✓ MTP command available")
        return True
    print(f"\n✗ MTP command failed: {result.stderr}")
    return False


if __name__ == "__main__":
    print("=" * 50)
    print("Filename Conflict Test")
    print("=" * 50)
    
    success = test_filename_conflict()
    
    print("\n" + "=" * 50)
    print("MTP Storage Listing Test")
    print("=" * 50)
    
    # MTP test is optional (requires device)
    test_mtp_storage_listing()
    
    sys.exit(0 if success else 1)
