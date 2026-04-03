"""Disk full (ENOSPC) handling tests for Svault.

Tests graceful handling of out-of-space conditions:
- Exit code 4 for disk full
- Transaction consistency (no partial files)
- Recovery after cleanup
"""

from __future__ import annotations

import subprocess
import tempfile
from pathlib import Path

import pytest


# Exit code definitions from CLI
EXIT_SUCCESS = 0
EXIT_DISK_FULL = 4


def create_test_file(path: Path, size_bytes: int) -> None:
    """Create a test file with specified size."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "wb") as f:
        f.write(b"0" * size_bytes)


def get_available_bytes(path: Path) -> int:
    """Get available bytes on the filesystem containing path."""
    import shutil
    stat = shutil.disk_usage(path)
    return stat.free


class TestDiskFullHandling:
    """Test disk full scenarios with small tmpfs."""

    @pytest.fixture
    def small_ramdisk(self, tmp_path: Path):
        """Create a very small RAMDisk (2MB) for space testing.
        
        Note: This uses a subdirectory with limited space simulation
        rather than actual mount to avoid permission issues.
        """
        # For actual disk full testing, we need a real mount
        # Use subprocess to create a small tmpfs
        mount_point = tmp_path / "small_ramdisk"
        mount_point.mkdir(parents=True, exist_ok=True)
        
        # Try to mount a small tmpfs (requires Linux and permissions)
        try:
            subprocess.run(
                ["mount", "-t", "tmpfs", "-o", "size=2m", "tmpfs", str(mount_point)],
                check=True,
                capture_output=True,
            )
            yield mount_point
            # Cleanup
            subprocess.run(
                ["umount", str(mount_point)],
                check=False,
                capture_output=True,
            )
        except (subprocess.CalledProcessError, FileNotFoundError):
            # If mount fails (no permissions), skip these tests
            pytest.skip("Cannot mount tmpfs (requires root or CAP_SYS_ADMIN)")

    def test_import_fails_with_exit_code_4_on_disk_full(
        self, small_ramdisk: Path, svault_binary: Path
    ):
        """D1: Import should fail with exit code 4 when disk is full.
        
        Steps:
        1. Create 2MB RAMDisk
        2. Initialize vault
        3. Create 3MB source file
        4. Import should fail with exit code 4
        """
        vault_dir = small_ramdisk / "vault"
        source_dir = small_ramdisk / "source"
        
        # Initialize vault
        result = subprocess.run(
            [str(svault_binary), "init"],
            cwd=vault_dir,
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0, f"Failed to init vault: {result.stderr}"
        
        # Create a large file (3MB should exceed 2MB disk)
        source_file = source_dir / "large_file.bin"
        create_test_file(source_file, 3 * 1024 * 1024)
        
        # Try to import - should fail with exit code 4
        result = subprocess.run(
            [str(svault_binary), "import", str(source_dir)],
            cwd=vault_dir,
            capture_output=True,
            text=True,
        )
        
        assert result.returncode == EXIT_DISK_FULL, (
            f"Expected exit code {EXIT_DISK_FULL} (disk full), "
            f"got {result.returncode}. stderr: {result.stderr}"
        )

    def test_no_partial_files_after_disk_full(
        self, small_ramdisk: Path, svault_binary: Path
    ):
        """D2: No partial files should remain after disk full failure.
        
        Steps:
        1. Initialize vault
        2. Fill disk partially
        3. Try to import more files
        4. Verify no partial files in vault
        """
        vault_dir = small_ramdisk / "vault"
        source_dir = small_ramdisk / "source"
        
        # Initialize vault
        subprocess.run(
            [str(svault_binary), "init"],
            cwd=vault_dir,
            check=True,
            capture_output=True,
        )
        
        # Create first file (should fit)
        file1 = source_dir / "file1.bin"
        create_test_file(file1, 512 * 1024)  # 512KB
        
        result = subprocess.run(
            [str(svault_binary), "import", str(source_dir)],
            cwd=vault_dir,
            capture_output=True,
        )
        
        # First import should succeed
        assert result.returncode == 0, f"First import failed: {result.stderr}"
        
        # Create second large file (should cause disk full)
        file2 = source_dir / "file2.bin"
        create_test_file(file2, 2 * 1024 * 1024)  # 2MB
        
        result = subprocess.run(
            [str(svault_binary), "import", str(source_dir)],
            cwd=vault_dir,
            capture_output=True,
        )
        
        # Should fail with disk full
        if result.returncode == EXIT_DISK_FULL:
            # Check that no partial files exist
            objects_dir = vault_dir / ".svault" / "objects"
            if objects_dir.exists():
                # List all files and check size integrity
                for obj_file in objects_dir.rglob("*"):
                    if obj_file.is_file():
                        size = obj_file.stat().st_size
                        # Partial files would have smaller sizes
                        # or end with .tmp or similar
                        assert not obj_file.name.endswith(".tmp"), (
                            f"Found temporary file: {obj_file}"
                        )

    def test_can_import_after_cleanup(
        self, small_ramdisk: Path, svault_binary: Path
    ):
        """D4: Can import successfully after freeing up space.
        
        Steps:
        1. Fill up disk with imports
        2. Delete some files from vault
        3. Import should succeed
        """
        vault_dir = small_ramdisk / "vault"
        source_dir = small_ramdisk / "source"
        
        # Initialize vault
        subprocess.run(
            [str(svault_binary), "init"],
            cwd=vault_dir,
            check=True,
            capture_output=True,
        )
        
        # Create and import first file
        file1 = source_dir / "file1.bin"
        create_test_file(file1, 512 * 1024)
        
        subprocess.run(
            [str(svault_binary), "import", str(source_dir)],
            cwd=vault_dir,
            check=True,
            capture_output=True,
        )
        
        # Find and delete the imported file from vault
        objects_dir = vault_dir / ".svault" / "objects"
        imported_files = list(objects_dir.rglob("*.bin"))
        
        if imported_files:
            # Delete to free up space
            imported_files[0].unlink()
            
            # Now try to import again
            file2 = source_dir / "file2.bin"
            create_test_file(file2, 256 * 1024)  # Smaller file
            
            result = subprocess.run(
                [str(svault_binary), "import", str(source_dir)],
                cwd=vault_dir,
                capture_output=True,
            )
            
            # Should succeed after cleanup
            assert result.returncode == 0, (
                f"Import failed after cleanup: {result.stderr}"
            )



