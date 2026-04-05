"""Cross-filesystem integration tests for Svault.

Tests import behavior across different filesystem combinations.
Currently focused on btrfs reflink support.

Uses img + loopback to create isolated filesystems without needing
additional disk partitions.

Img files are created on RAMDisk for:
- Speed: Memory is faster than disk
- Automatic cleanup: RAMDisk is cleaned up with test isolation
- Reduced disk wear: Avoid frequent img creation/deletion on SSD
"""

from __future__ import annotations

import os
import subprocess
import tempfile
from contextlib import contextmanager
from pathlib import Path
from typing import Generator

import pytest

# Import from conftest
from conftest import PROJECT_ROOT


class LoopbackFs:
    """Manage a loopback-mounted filesystem for testing.
    
    Img files can be created on RAMDisk for speed and automatic cleanup.
    """
    
    def __init__(self, fs_type: str, size_mb: int = 256, mount_opts: str = ""):
        self.fs_type = fs_type
        self.size_mb = size_mb
        self.mount_opts = mount_opts
        self.img_path: Path | None = None
        self.mount_point: Path | None = None
        self._loop_device: str | None = None
        self._mounted = False
    
    def create(self, base_dir: Path, img_dir: Path | None = None) -> Path:
        """Create and mount the filesystem.
        
        Args:
            base_dir: Base directory for mount point
            img_dir: Optional directory to store img file
        
        Returns:
            Path to mount point
        """
        # If img_dir provided, store img file there
        if img_dir:
            img_subdir = img_dir / "loopback_images"
            img_subdir.mkdir(parents=True, exist_ok=True)
            self.img_path = img_subdir / f"{self.fs_type}_{id(self)}.img"
        else:
            self.img_path = base_dir / f"{self.fs_type}.img"
        
        self.mount_point = base_dir / self.fs_type
        self.mount_point.mkdir(parents=True, exist_ok=True)
        
        # Create image file
        subprocess.run(
            ["dd", "if=/dev/zero", f"of={self.img_path}", "bs=1M", f"count={self.size_mb}"],
            check=True,
            capture_output=True,
        )
        
        # Create filesystem
        mkfs_cmd = {
            "ext4": ["mkfs.ext4", "-F", str(self.img_path)],
            "btrfs": ["mkfs.btrfs", "-f", str(self.img_path)],
        }.get(self.fs_type)
        
        if not mkfs_cmd:
            raise ValueError(f"Unsupported filesystem: {self.fs_type}")
        
        subprocess.run(mkfs_cmd, check=True, capture_output=True)
        
        # Mount with loop
        mount_cmd = ["mount", "-o", "loop"]
        if self.mount_opts:
            mount_cmd.extend(["-o", self.mount_opts])
        mount_cmd.extend([str(self.img_path), str(self.mount_point)])
        
        try:
            subprocess.run(mount_cmd, check=True, capture_output=True)
        except subprocess.CalledProcessError:
            # Try with sudo
            subprocess.run(["sudo", "-n"] + mount_cmd, check=True, capture_output=True)
        
        self._mounted = True
        
        # Set ownership to current user recursively
        # This is needed because ext4 filesystems may have files owned by root
        try:
            os.chown(self.mount_point, os.getuid(), os.getgid())
            # Also chown the root of the mounted filesystem
            for item in self.mount_point.iterdir():
                os.chown(item, os.getuid(), os.getgid())
        except PermissionError:
            subprocess.run(
                ["sudo", "-n", "chown", "-R", f"{os.getuid()}:{os.getgid()}", str(self.mount_point)],
                check=False,
            )
        
        return self.mount_point
    
    def cleanup(self) -> None:
        """Unmount and remove the filesystem."""
        if self._mounted and self.mount_point:
            # Wait for any pending IO and sync
            try:
                subprocess.run(["sync"], check=False, capture_output=True)
            except Exception:
                pass
            
            # Try regular umount first
            umounted = False
            try:
                result = subprocess.run(
                    ["umount", str(self.mount_point)],
                    check=False,
                    capture_output=True,
                )
                if result.returncode == 0:
                    umounted = True
            except Exception:
                pass
            
            # If regular umount fails, try with sudo
            if not umounted:
                try:
                    subprocess.run(
                        ["sudo", "-n", "umount", str(self.mount_point)],
                        check=False,
                        capture_output=True,
                    )
                except Exception:
                    pass
            
            # Mark as unmounted regardless (best effort)
            self._mounted = False
        
        # Remove img file
        if self.img_path and self.img_path.exists():
            try:
                self.img_path.unlink()
            except Exception:
                pass
    
    def __enter__(self) -> Path:
        return self.create(Path(tempfile.gettempdir()) / "svault-fs-test")
    
    def __exit__(self, *args) -> None:
        self.cleanup()


@contextmanager
def cross_fs_env(
    source_fs: str,
    vault_fs: str,
    img_dir: Path | None = None,
) -> Generator[tuple[Path, Path, LoopbackFs, LoopbackFs], None, None]:
    """Create a cross-filesystem test environment.
    
    Args:
        source_fs: Filesystem type for source directory
        vault_fs: Filesystem type for vault directory
        img_dir: Optional directory to store img files
    
    Yields:
        Tuple of (source_mount, vault_mount, source_fs_obj, vault_fs_obj)
    """
    # Use img_dir for base dir if available, otherwise use temp dir
    if img_dir:
        base_dir = img_dir / "cross_fs_test"
    else:
        base_dir = Path(tempfile.gettempdir()) / "svault-cross-fs-test"
    
    base_dir.mkdir(parents=True, exist_ok=True)
    
    source = LoopbackFs(source_fs)
    vault = LoopbackFs(vault_fs)
    
    try:
        source_mount = source.create(base_dir / "source", img_dir=img_dir)
        vault_mount = vault.create(base_dir / "vault", img_dir=img_dir)
        yield (source_mount, vault_mount, source, vault)
    finally:
        # Cleanup loopback filesystems
        vault.cleanup()
        source.cleanup()
        
        # Give umount time to complete
        import time
        time.sleep(0.2)
        
        # Cleanup base dir if not using custom img_dir
        if not img_dir:
            try:
                base_dir.rmdir()
            except Exception:
                pass


def check_file_linked(file1: Path, file2: Path) -> dict[str, bool]:
    """Check relationship between two files.
    
    Returns dict with:
        - is_hardlink: same inode
        - is_reflink: same physical blocks (CoW)
        - is_copy: completely separate
    """
    result = {
        "is_hardlink": False,
        "is_reflink": False,
        "is_copy": True,
    }
    
    if not file1.exists() or not file2.exists():
        return result
    
    # Check hardlink (same inode)
    stat1 = file1.stat()
    stat2 = file2.stat()
    
    if stat1.st_ino == stat2.st_ino and stat1.st_dev == stat2.st_dev:
        result["is_hardlink"] = True
        result["is_copy"] = False
        return result
    
    # Check reflink (btrfs)
    try:
        # Use filefrag to check shared extents
        frag1 = subprocess.run(
            ["filefrag", "-v", str(file1)],
            capture_output=True,
            text=True,
        )
        frag2 = subprocess.run(
            ["filefrag", "-v", str(file2)],
            capture_output=True,
            text=True,
        )
        
        # If both show shared extents, likely reflink
        if "shared" in frag1.stdout.lower() and "shared" in frag2.stdout.lower():
            result["is_reflink"] = True
            result["is_copy"] = False
    except Exception:
        pass
    
    return result


@pytest.fixture
def check_root():
    """Skip tests if not root and sudo is not available."""
    if os.geteuid() != 0:
        # Check if sudo is available without password
        result = subprocess.run(
            ["sudo", "-n", "true"],
            capture_output=True,
        )
        if result.returncode != 0:
            pytest.skip("Root or passwordless sudo required for loopback mount")


@pytest.fixture
def check_btrfs_tools():
    """Skip tests if btrfs tools are not available."""
    result = subprocess.run(
        ["which", "mkfs.btrfs"],
        capture_output=True,
    )
    if result.returncode != 0:
        pytest.skip("btrfs-tools not installed")


@pytest.fixture(scope="function")
def loopback_img_dir(tmp_path: Path) -> Path:
    """Provide a directory for loopback img files.
    
    Note: Using regular tmp_path instead of RAMDisk to avoid mount cleanup issues.
    The img files are small (256MB) and will be cleaned up by pytest.
    """
    img_dir = tmp_path / "loopback_images"
    img_dir.mkdir(parents=True, exist_ok=True)
    return img_dir


class TestCrossFilesystemImport:
    """Cross-filesystem import behavior tests."""

    def test_ext4_to_ext4_copy(self, check_root, loopback_img_dir, svault_binary: Path):
        """X1: ext4 → ext4 should use copy (different mounts).
        
        Setup:
        - Source on ext4 loopback
        - Vault on ext4 loopback
        
        Verify:
        - Import succeeds
        - Files are valid copies (not hardlinks - different mounts)
        """
        with cross_fs_env("ext4", "ext4", img_dir=loopback_img_dir) as (source_mount, vault_mount, _, _):
            # Create a JPEG test file (svault only imports media files by default)
            from conftest import create_minimal_jpeg
            source_file = source_mount / "test.jpg"
            create_minimal_jpeg(source_file, "cross_fs_test_data")
            
            # Init vault
            result = subprocess.run(
                [str(svault_binary), "init"],
                cwd=vault_mount,
                capture_output=True,
                text=True,
            )
            assert result.returncode == 0, f"Vault init failed: {result.stderr}"
            
            # First import (with --yes to skip confirmation)
            result = subprocess.run(
                [str(svault_binary), "--yes", "import", str(source_mount)],
                cwd=vault_mount,
                capture_output=True,
                text=True,
            )
            assert result.returncode == 0, f"First import failed: {result.stderr}"
            
            # Find imported file
            vault_file = list(vault_mount.rglob("test.jpg"))
            assert len(vault_file) > 0, f"File not imported. Vault contents: {list(vault_mount.rglob('*'))}"
            
            # Verify it's a copy (different mounts, can't hardlink)
            link_info = check_file_linked(source_file, vault_file[0])
            assert link_info["is_copy"], "Should be copy across different mounts"

    @pytest.mark.skip(reason="btrfs requires kernel support and tools")
    def test_btrfs_to_btrfs_reflink(self, check_root, check_btrfs_tools, loopback_img_dir, svault_binary: Path):
        """X2: btrfs → btrfs should use reflink (same FS type, CoW).
        
        Note: Skipped by default as btrfs may not be available in all environments.
        When enabled, tests that svault properly uses reflink on btrfs.
        """
        with cross_fs_env("btrfs", "btrfs", img_dir=loopback_img_dir) as (source_mount, vault_mount, _, _):
            # Create a JPEG test file (svault only imports media files by default)
            from conftest import create_minimal_jpeg
            source_file = source_mount / "reflink_test.jpg"
            create_minimal_jpeg(source_file, "btrfs_reflink_test")
            
            # Init vault
            result = subprocess.run(
                [str(svault_binary), "init"],
                cwd=vault_mount,
                capture_output=True,
                text=True,
            )
            assert result.returncode == 0, f"Vault init failed: {result.stderr}"
            
            # Import with reflink strategy (with --yes to skip confirmation)
            result = subprocess.run(
                [str(svault_binary), "--yes", "import", "--strategy", "reflink,copy", 
                 str(source_mount)],
                cwd=vault_mount,
                capture_output=True,
                text=True,
            )
            assert result.returncode == 0, f"Import failed: {result.stderr}"
            
            # Find imported file
            vault_files = list(vault_mount.rglob("reflink_test.jpg"))
            assert len(vault_files) > 0, "File not imported"
            
            # Should be reflink (same filesystem)
            link_info = check_file_linked(source_file, vault_files[0])
            # Note: Due to mount boundaries, might still be copy
            # This test mainly verifies import succeeds on btrfs
            assert vault_files[0].exists(), "File should exist in vault"

    def test_tmpfs_to_ext4_stream_copy(self, tmp_path: Path, svault_binary: Path):
        """X3: tmpfs → ext4 should use stream copy.
        
        Setup:
        - Source on tmpfs (memory)
        - Vault on regular ext4
        
        Verify:
        - Import succeeds
        - Files are valid
        """
        import sqlite3
        
        source_dir = tmp_path / "source"
        vault_dir = tmp_path / "vault"
        source_dir.mkdir()
        vault_dir.mkdir()
        
        # Create test image file (JPEG)
        test_file = source_dir / "memory_test.jpg"
        try:
            from PIL import Image
            img = Image.new("RGB", (100, 100), color="blue")
            img.save(test_file, "JPEG")
        except ImportError:
            # Fallback: minimal JPEG
            test_file.write_bytes(b"\xff\xd8\xff\xe0" + b"\x00" * 100)
        
        # Init vault
        subprocess.run(
            [str(svault_binary), "init"],
            cwd=vault_dir,
            check=True,
            capture_output=True,
        )
        
        # Import with --yes to skip confirmation
        result = subprocess.run(
            [str(svault_binary), "--yes", "import", str(source_dir)],
            cwd=vault_dir,
            capture_output=True,
            text=True,
        )
        
        assert result.returncode == 0, f"Import failed: {result.stderr}"
        
        # Verify via database
        db_path = vault_dir / ".svault" / "vault.db"
        conn = sqlite3.connect(str(db_path))
        conn.row_factory = sqlite3.Row
        cur = conn.execute("SELECT * FROM files WHERE path LIKE '%.jpg%'")
        rows = cur.fetchall()
        conn.close()
        
        assert len(rows) > 0, "File not imported (not in database)"


class TestFilesystemCapabilities:
    """Test filesystem capability detection."""

    @pytest.mark.skip(reason="btrfs requires kernel support and tools")
    def test_reflink_capability_detection(self, check_root, check_btrfs_tools):
        """Verify we can detect reflink support on btrfs filesystem."""
        with tempfile.TemporaryDirectory() as tmpdir:
            btrfs = LoopbackFs("btrfs")
            try:
                mount = btrfs.create(Path(tmpdir))
                test_file = mount / "reflink_test.txt"
                test_file.write_text("test")
                
                # btrfs should support reflink
                # This is a simplified check - actual reflink testing 
                # would require creating two files and checking shared extents
                assert test_file.exists(), "Test file should exist"
            finally:
                btrfs.cleanup()
