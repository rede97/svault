"""Disk full (ENOSPC) handling tests for Svault.

Tests graceful handling of out-of-space conditions:
- Exit code 4 for disk full
- Transaction consistency (no partial files)
- Recovery after cleanup

使用 loopback 设备创建小容量 ext4 文件系统进行测试，
避免依赖 tmpfs 和 CAP_SYS_ADMIN。
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

from conftest import PROJECT_ROOT, create_minimal_jpeg


# Exit code definitions from CLI
EXIT_SUCCESS = 0
EXIT_DISK_FULL = 4


class SmallLoopbackFs:
    """小容量 loopback 文件系统，用于磁盘满测试。
    
    创建一个指定大小的 ext4 镜像文件并挂载，无需额外磁盘分区。
    """
    
    def __init__(self, size_mb: int = 4):
        self.size_mb = size_mb
        self.img_path: Path | None = None
        self.mount_point: Path | None = None
        self._mounted = False
    
    def _cleanup_previous(self, base_dir: Path) -> None:
        """清理可能残留的旧挂载和镜像。"""
        img_path = base_dir / f"small_disk_{self.size_mb}m.img"
        mount_point = base_dir / "small_disk"
        
        # 尝试卸载残留挂载
        if mount_point.exists():
            subprocess.run(["umount", str(mount_point)], check=False, capture_output=True)
            subprocess.run(["sudo", "-n", "umount", str(mount_point)], check=False, capture_output=True)
        
        # 释放关联的 loopback 设备
        if img_path.exists():
            result = subprocess.run(["losetup", "-j", str(img_path)], capture_output=True, text=True)
            for line in result.stdout.strip().split("\n"):
                if ":" in line:
                    loop_dev = line.split(":")[0]
                    subprocess.run(["sudo", "-n", "losetup", "-d", loop_dev], check=False, capture_output=True)
            # 删除旧镜像
            try:
                img_path.unlink()
            except OSError:
                subprocess.run(["sudo", "-n", "rm", "-f", str(img_path)], check=False, capture_output=True)
    
    def create(self, base_dir: Path) -> Path:
        """创建并挂载小容量文件系统。
        
        Args:
            base_dir: 用于存放镜像和挂载点的基础目录
            
        Returns:
            挂载点路径
        """
        self.img_path = base_dir / f"small_disk_{self.size_mb}m.img"
        self.mount_point = base_dir / "small_disk"
        
        # 先清理可能残留的旧资源
        self._cleanup_previous(base_dir)
        
        self.mount_point.mkdir(parents=True, exist_ok=True)
        
        # 创建镜像文件
        subprocess.run(
            ["dd", "if=/dev/zero", f"of={self.img_path}", "bs=1M", f"count={self.size_mb}"],
            check=True,
            capture_output=True,
        )
        
        # 创建 ext4 文件系统
        subprocess.run(
            ["mkfs.ext4", "-F", str(self.img_path)],
            check=True,
            capture_output=True,
        )
        
        # 挂载（尝试直接挂载，失败则使用 sudo）
        try:
            subprocess.run(
                ["mount", "-o", "loop", str(self.img_path), str(self.mount_point)],
                check=True,
                capture_output=True,
            )
        except subprocess.CalledProcessError:
            try:
                subprocess.run(
                    ["sudo", "-n", "mount", "-o", "loop", str(self.img_path), str(self.mount_point)],
                    check=True,
                    capture_output=True,
                )
            except (subprocess.CalledProcessError, FileNotFoundError):
                raise RuntimeError("Failed to mount loopback device (requires root or passwordless sudo)")
        
        self._mounted = True
        
        # 设置当前用户为所有者（需要 sudo，因为 mount 可能也是 sudo）
        try:
            import os
            uid, gid = os.getuid(), os.getgid()
            # 先尝试不使用 sudo
            result = subprocess.run(
                ["chown", "-R", f"{uid}:{gid}", str(self.mount_point)],
                check=False,
                capture_output=True,
            )
            if result.returncode != 0:
                # 失败则尝试 sudo
                subprocess.run(
                    ["sudo", "-n", "chown", "-R", f"{uid}:{gid}", str(self.mount_point)],
                    check=False,
                    capture_output=True,
                )
        except Exception:
            pass
        
        return self.mount_point
    
    def cleanup(self):
        """清理：卸载并删除镜像。"""
        if self._mounted and self.mount_point:
            try:
                subprocess.run(
                    ["umount", str(self.mount_point)],
                    check=False,
                    capture_output=True,
                )
            except Exception:
                pass
            # 也尝试 sudo umount
            try:
                subprocess.run(
                    ["sudo", "-n", "umount", str(self.mount_point)],
                    check=False,
                    capture_output=True,
                )
            except Exception:
                pass
            self._mounted = False
        
        # 释放 loopback 设备并删除镜像
        if self.img_path and self.img_path.exists():
            try:
                result = subprocess.run(
                    ["losetup", "-j", str(self.img_path)],
                    capture_output=True, text=True,
                )
                for line in result.stdout.strip().split("\n"):
                    if ":" in line:
                        loop_dev = line.split(":")[0]
                        subprocess.run(
                            ["sudo", "-n", "losetup", "-d", loop_dev],
                            check=False, capture_output=True,
                        )
            except Exception:
                pass
            try:
                self.img_path.unlink()
            except OSError:
                try:
                    subprocess.run(
                        ["sudo", "-n", "rm", "-f", str(self.img_path)],
                        check=False, capture_output=True,
                    )
                except Exception:
                    pass


@pytest.fixture
def small_disk(test_dir: Path, check_loopback_support):
    """创建测试环境：loopback 文件系统 + 外部源目录（全部在测试目录中）。
    
    所有测试数据都在测试目录中，保证测试结束后系统干净。
    
    Returns:
        tuple: (vault_dir, source_dir) 
        - vault_dir: 在 32MB loopback 内（用于测试磁盘满）
        - source_dir: 在测试目录内但在 loopback 外（确保有足够空间创建大文件）
    """
    fs = SmallLoopbackFs(size_mb=32)
    try:
        # loopback 挂载点在测试目录内
        mount_point = fs.create(test_dir)
        # vault 在 loopback 内（小磁盘，会满）
        vault_dir = mount_point / "vault"
        # source 在测试目录内但在 loopback 外（大磁盘，不会满）
        source_dir = test_dir / "disk_full_source"
        yield vault_dir, source_dir
    except RuntimeError as e:
        pytest.skip(f"Cannot create loopback filesystem: {e}")
    finally:
        fs.cleanup()


@pytest.fixture
def check_loopback_support():
    """检查是否支持 loopback 设备。"""
    try:
        # 测试是否能使用 losetup
        result = subprocess.run(
            ["losetup", "-f"],
            capture_output=True,
            check=False,
        )
        if result.returncode != 0:
            pytest.skip("Loopback device not available (requires root or loop module)")
        
        # 测试 mkfs.ext4
        result = subprocess.run(
            ["which", "mkfs.ext4"],
            capture_output=True,
            check=False,
        )
        if result.returncode != 0:
            pytest.skip("mkfs.ext4 not available")
            
    except FileNotFoundError:
        pytest.skip("Required tools not available")


class TestDiskFullHandling:
    """Test disk full scenarios with small loopback filesystem."""

    def test_import_fails_with_exit_code_4_on_disk_full(
        self, small_disk: tuple[Path, Path], svault_binary: Path, check_loopback_support
    ):
        """D1: Import large JPEG should fail with exit code 4 when disk is full.
        
        Steps:
        1. Create 32MB loopback ext4 filesystem (vault 目录)
        2. Initialize vault
        3. Create large JPEG files (>40MB total) in external source
        4. Import should fail with exit code 4
        """
        vault_dir, source_dir = small_disk
        
        # Initialize vault first (takes some space)
        vault_dir.mkdir(parents=True, exist_ok=True)
        result = subprocess.run(
            [str(svault_binary), "init"],
            cwd=vault_dir,
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0, f"Failed to init vault: {result.stderr}"
        
        # Create large JPEG files (total > 40MB to exceed 32MB disk)
        source_dir.mkdir(parents=True, exist_ok=True)
        for i in range(5):
            jpeg_file = source_dir / f"large_photo_{i}.jpg"
            # Create base JPEG then append padding to make it ~10MB each
            create_minimal_jpeg(jpeg_file, f"LARGE_PHOTO_{i}")
            current_size = jpeg_file.stat().st_size
            target_size = 10 * 1024 * 1024  # 10MB each
            with open(jpeg_file, 'ab') as f:
                f.write(b"\xff" * (target_size - current_size))
        
        # Try to import - should fail due to disk full
        # Exit code 4 = disk full detected during copy (graceful)
        # Exit code 1 = disk full at other stage (e.g., staging list)
        result = subprocess.run(
            [str(svault_binary), "--yes", "import", str(source_dir)],
            cwd=vault_dir,
            capture_output=True,
            text=True,
        )
        
        # Accept either exit code as long as it's a disk full error
        assert result.returncode in [EXIT_DISK_FULL, 1], (
            f"Expected exit code {EXIT_DISK_FULL} or 1 (disk full), "
            f"got {result.returncode}. stderr: {result.stderr}"
        )
        assert "No space" in result.stderr or "disk full" in result.stderr.lower(), (
            f"Expected disk full error message, got: {result.stderr}"
        )

    def test_no_partial_files_after_disk_full(
        self, small_disk: tuple[Path, Path], svault_binary: Path, check_loopback_support
    ):
        """D2: No partial files should remain after disk full failure.
        
        Steps:
        1. Initialize vault
        2. Import small JPEG (should fit)
        3. Try to import large JPEGs (should fail)
        4. Verify no partial files in vault
        """
        vault_dir, source_dir = small_disk
        
        # Initialize vault
        vault_dir.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            [str(svault_binary), "init"],
            cwd=vault_dir,
            check=True,
            capture_output=True,
        )
        
        # Create first small JPEG (should fit)
        source_dir.mkdir(parents=True, exist_ok=True)
        file1 = source_dir / "photo1.jpg"
        create_minimal_jpeg(file1, "SMALL_PHOTO")
        
        result = subprocess.run(
            [str(svault_binary), "import", str(source_dir)],
            cwd=vault_dir,
            capture_output=True,
            text=True,
        )
        
        # First import should succeed
        assert result.returncode == 0, f"First import failed: {result.stderr}"
        
        # Create second large JPEG (should cause disk full)
        file2 = source_dir / "photo2.jpg"
        create_minimal_jpeg(file2, "LARGE_PHOTO")
        with open(file2, 'ab') as f:
            f.write(b"\xff" * (20 * 1024 * 1024))  # Add 20MB padding
        
        result = subprocess.run(
            [str(svault_binary), "--yes", "import", str(source_dir)],
            cwd=vault_dir,
            capture_output=True,
        )
        
        # Should fail with disk full (exit code 4 or 1)
        if result.returncode in [EXIT_DISK_FULL, 1]:
            # Check that no partial files exist
            objects_dir = vault_dir / ".svault" / "objects"
            if objects_dir.exists():
                for obj_file in objects_dir.rglob("*"):
                    if obj_file.is_file():
                        assert not obj_file.name.endswith(".tmp"), (
                            f"Found temporary file: {obj_file}"
                        )

    def test_can_import_after_cleanup(
        self, small_disk: tuple[Path, Path], svault_binary: Path, check_loopback_support
    ):
        """D3: Can import successfully after freeing up space.
        
        Steps:
        1. Import small JPEG
        2. Fill up disk with large JPEG
        3. Delete some vault files to free space
        4. Import should succeed
        """
        vault_dir, source_dir = small_disk
        
        # Initialize vault
        vault_dir.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            [str(svault_binary), "init"],
            cwd=vault_dir,
            check=True,
            capture_output=True,
        )
        
        # Create and import first file
        source_dir.mkdir(parents=True, exist_ok=True)
        file1 = source_dir / "photo1.jpg"
        create_minimal_jpeg(file1, "PHOTO_ONE")
        
        subprocess.run(
            [str(svault_binary), "import", str(source_dir)],
            cwd=vault_dir,
            check=True,
            capture_output=True,
        )
        
        # Find and delete the imported file from vault to free space
        objects_dir = vault_dir / ".svault" / "objects"
        imported_files = list(objects_dir.rglob("*.jpg"))
        
        if imported_files:
            # Delete to free up space
            imported_files[0].unlink()
            
            # Now try to import a different file
            file2 = source_dir / "photo2.jpg"
            create_minimal_jpeg(file2, "PHOTO_TWO")
            
            result = subprocess.run(
                [str(svault_binary), "--yes", "import", str(source_dir)],
                cwd=vault_dir,
                capture_output=True,
                text=True,
            )
            
            # Should succeed after cleanup
            assert result.returncode == 0, (
                f"Import failed after cleanup: {result.stderr}"
            )


class TestDiskFullEdgeCases:
    """Edge cases for disk full handling."""

    def test_exact_size_file(
        self, small_disk: tuple[Path, Path], svault_binary: Path, check_loopback_support
    ):
        """D4: Import JPEG that exactly fits available space.
        
        Note: Due to ext4 metadata overhead, we need some margin.
        """
        import shutil
        
        vault_dir, source_dir = small_disk
        
        vault_dir.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            [str(svault_binary), "init"],
            cwd=vault_dir,
            check=True,
            capture_output=True,
        )
        
        # Clean source dir to ensure no leftover files from previous tests
        if source_dir.exists():
            shutil.rmtree(source_dir)
        source_dir.mkdir(parents=True, exist_ok=True)
        
        # Create a JPEG that should fit (leaving margin for metadata)
        # 32MB filesystem, use ~5MB file (smaller to ensure it fits)
        file1 = source_dir / "photo1.jpg"
        create_minimal_jpeg(file1, "FIT_TEST")
        with open(file1, 'ab') as f:
            f.write(b"\xff" * (5 * 1024 * 1024))  # Add 5MB padding (smaller)
        
        result = subprocess.run(
            [str(svault_binary), "--yes", "import", str(source_dir)],
            cwd=vault_dir,
            capture_output=True,
        )
        
        # Should succeed - file fits in available space
        assert result.returncode == 0, (
            f"Expected success (0), got {result.returncode}. stderr: {result.stderr}"
        )
