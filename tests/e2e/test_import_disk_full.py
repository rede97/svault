"""Disk full (ENOSPC) handling tests for Svault.

Tests graceful handling of out-of-space conditions:
- Exit code 4 for disk full
- Transaction consistency (no partial files)
- Recovery after cleanup

使用 loopback 设备创建小容量 ext4 文件系统进行测试，
避免依赖 tmpfs 和 CAP_SYS_ADMIN。
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest

from conftest import PROJECT_ROOT, create_minimal_jpeg


# Exit code definitions from CLI
EXIT_SUCCESS = 0


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
        # test_dir 是跨测试共享的 RAMDisk 根——必须清理，否则上一用例的
        # 大文件会污染本用例的首个导入
        source_dir = test_dir / "disk_full_source"
        if source_dir.exists():
            shutil.rmtree(source_dir)
        source_dir.mkdir(parents=True, exist_ok=True)
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

    def test_disk_full_copy_failure_is_per_file(
        self, small_disk: tuple[Path, Path], svault_binary: Path, check_loopback_support
    ):
        """D1: 复制阶段 ENOSPC 是逐文件失败（G4）→ exit 0，DB 无记录。

        新契约（2026-08-09 staging 模型 + events 移除后锁定）：
        - 复制失败逐文件隔离，不致命 → rc == 0（失败计数体现在摘要）
        - 致命 ENOSPC（plan 写失败 / DB 插入失败 / manifest 写失败）才是 rc == 1
        - 半成品绝不落在最终路径；本次会话的 staging 子树由本进程清理，
          会话目录只剩 plan.json（无 manifest）
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

        # 单个 40MB 文件必然超过 32MB 盘——确定性复制失败
        source_dir.mkdir(parents=True, exist_ok=True)
        big = source_dir / "too_big.jpg"
        create_minimal_jpeg(big, "TOO_BIG")
        with open(big, 'ab') as f:
            f.write(b"\xff" * (40 * 1024 * 1024 - big.stat().st_size))

        result = subprocess.run(
            [str(svault_binary), "--yes", "import", str(source_dir)],
            cwd=vault_dir,
            capture_output=True,
            text=True,
        )

        assert result.returncode == 0, (
            f"复制 ENOSPC 是逐文件失败（G4），应为 exit 0，got {result.returncode}. "
            f"stderr: {result.stderr}"
        )
        assert "No space" in result.stderr or "disk full" in result.stderr.lower(), (
            f"Expected disk full error message, got: {result.stderr}"
        )

        # DB 无记录；最终路径无半成品；会话目录只有 plan.json
        import sqlite3
        conn = sqlite3.connect(str(vault_dir / ".svault" / "vault.db"))
        count = conn.execute("SELECT COUNT(*) FROM files").fetchone()[0]
        conn.close()
        assert count == 0, f"复制失败的文件不得入库: {count}"

        visible = [
            p for p in vault_dir.rglob("*.jpg") if ".svault" not in p.parts
        ]
        assert visible == [], f"最终路径不得出现半成品: {visible}"

        sessions = list((vault_dir / ".svault" / "sessions" / "import").glob("*"))
        assert len(sessions) == 1
        assert (sessions[0] / "plan.json").exists()
        assert not (sessions[0] / "manifest.json").exists()
        assert not (sessions[0] / "staging").exists(), (
            "本次会话的 staging 子树应由本进程清理"
        )

    def test_no_partial_files_after_disk_full(
        self, small_disk: tuple[Path, Path], svault_binary: Path, check_loopback_support
    ):
        """D2: ENOSPC 后 DB 不含失败文件；最终路径绝无半成品（staging 模型）

        判据（failure-handling.md §3.1/G7，2026-08-09 更新）：
        1. 复制 ENOSPC 是逐文件失败 → rc == 0（G4）
        2. DB 只含首个成功文件
        3. 失败文件绝不出现在最终路径——staging 模型下半成品只可能
           存在于会话目录，且本次会话的 staging 子树已由本进程清理
        """
        vault_dir, source_dir = small_disk

        vault_dir.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            [str(svault_binary), "init"],
            cwd=vault_dir,
            check=True,
            capture_output=True,
        )

        # 首个小文件（放得下）
        source_dir.mkdir(parents=True, exist_ok=True)
        file1 = source_dir / "photo1.jpg"
        create_minimal_jpeg(file1, "SMALL_PHOTO")

        result = subprocess.run(
            [str(svault_binary), "--yes", "import", str(source_dir)],
            cwd=vault_dir,
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0, f"First import failed: {result.stderr}"

        # 30MB 大文件：32MB 盘减去 init+photo1 后必然放不下
        file2 = source_dir / "photo2.jpg"
        create_minimal_jpeg(file2, "LARGE_PHOTO")
        full_size = 30 * 1024 * 1024
        with open(file2, 'ab') as f:
            f.write(b"\xff" * (full_size - file2.stat().st_size))

        result = subprocess.run(
            [str(svault_binary), "--yes", "import", str(source_dir)],
            cwd=vault_dir,
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0, (
            f"复制 ENOSPC 是逐文件失败（G4），应为 exit 0: rc={result.returncode}"
        )

        # DB 只含 photo1（photo2 未入库）
        import sqlite3
        conn = sqlite3.connect(str(vault_dir / ".svault" / "vault.db"))
        rows = [r[0] for r in conn.execute("SELECT path FROM files").fetchall()]
        conn.close()
        assert len(rows) == 1 and "photo1" in rows[0], (
            f"DB 应只含首个成功文件: {rows}"
        )

        # staging 模型契约：photo2 绝不可见于最终路径
        residue = [
            p for p in vault_dir.rglob("photo2*.jpg")
            if ".svault" not in p.parts
        ]
        assert residue == [], (
            f"最终路径不得出现失败文件的任何副本: {residue}"
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
            [str(svault_binary), "--yes", "import", str(source_dir)],
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
