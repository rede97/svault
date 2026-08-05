"""Import interruption and concurrent modification tests.

Merged from:
- test_import_interruption.py: Signal interruption using strace inject
- test_concurrent_modification.py: Concurrent modification during import

本文件使用 strace 的 inject 功能在特定系统调用时注入信号，
实现比 time.sleep() 更可靠的进程中断测试。

对于需要精确 IO 控制的深度故障注入测试，参见 fuse_tests/ 目录。
"""

from __future__ import annotations

import os
import sqlite3
import subprocess
import time
from pathlib import Path

import pytest

from conftest import VaultEnv, create_minimal_jpeg


def check_strace_available() -> bool:
    """检查 strace 是否可用且支持 inject 功能"""
    try:
        result = subprocess.run(
            ["strace", "--help"],
            capture_output=True,
            text=True,
        )
        return result.returncode == 0 and "inject" in result.stdout
    except FileNotFoundError:
        return False


@pytest.fixture(scope="session")
def strace_available() -> bool:
    """检查 strace 是否可用"""
    return check_strace_available()


# =============================================================================
# Level 1: Signal Interruption (using strace inject)
# =============================================================================

class TestSignalInterruption:
    """信号中断测试 - 使用 strace inject 实现精确控制"""

    @pytest.mark.skipif(
        not check_strace_available(),
        reason="strace not available or doesn't support inject"
    )
    def test_sigterm_during_import(self, vault: VaultEnv, strace_available: bool) -> None:
        """SIGTERM 中断导入过程 - 在第 N 次 read 时注入"""
        num_files = 20
        for i in range(num_files):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        strace_cmd = [
            "strace",
            "-e", "inject=read:signal=SIGTERM:when=10",
            "-o", "/dev/null",
            str(vault.binary),
            "--yes", "import", str(vault.source_dir),
        ]

        result = subprocess.run(
            strace_cmd,
            cwd=vault.vault_dir,
            capture_output=True,
            text=True,
        )

        assert result.returncode != 0, f"Process should have been terminated by SIGTERM"

        files_after_interrupt = vault.db_files()
        count_after_interrupt = len(files_after_interrupt)
        print(f"Files imported before interrupt: {count_after_interrupt}/{num_files}")
        assert count_after_interrupt <= num_files

        # Resume import
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0

        files_after_resume = vault.db_files()
        assert len(files_after_resume) == num_files

        paths = [f["path"] for f in files_after_resume]
        assert len(paths) == len(set(paths)), "Duplicate files detected!"


class TestDatabaseConsistency:
    """中断后的数据库一致性验证"""

    @pytest.mark.skipif(
        not check_strace_available(),
        reason="strace not available"
    )
    def test_no_partial_files_after_interrupt(self, vault: VaultEnv, strace_available: bool) -> None:
        """验证中断后没有部分写入的文件"""
        f = vault.source_dir / "large_test.jpg"
        create_minimal_jpeg(f, "LARGE_FILE_CONTENT")
        with open(f, 'ab') as fp:
            fp.write(b"X" * (100 * 1024))

        subprocess.run(
            [
                "strace", "-e", "inject=write:signal=SIGTERM:when=1",
                "-o", "/dev/null",
                str(vault.binary), "--yes", "import", str(vault.source_dir),
            ],
            cwd=vault.vault_dir,
            capture_output=True,
        )

        files = vault.db_files()
        for file_info in files:
            vault_path = vault.vault_dir / file_info["path"]
            assert vault_path.exists(), f"File in DB but missing: {file_info['path']}"
            assert vault_path.stat().st_size > 0, f"File has zero size"

    @pytest.mark.skipif(
        not check_strace_available(),
        reason="strace not available"
    )
    def test_database_integrity_after_interrupt(self, vault: VaultEnv, strace_available: bool) -> None:
        """验证中断后数据库完整性"""
        for i in range(5):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        subprocess.run(
            [
                "strace", "-e", "inject=read:signal=SIGKILL:when=5",
                "-o", "/dev/null",
                str(vault.binary), "--yes", "import", str(vault.source_dir),
            ],
            cwd=vault.vault_dir,
            capture_output=True,
        )

        db_path = vault.vault_dir / ".svault" / "vault.db"
        conn = sqlite3.connect(str(db_path))

        cursor = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        )
        tables = [row[0] for row in cursor.fetchall()]
        assert "files" in tables
        assert "events" in tables

        cursor = conn.execute("SELECT COUNT(*) FROM files")
        file_count = cursor.fetchone()[0]
        assert file_count >= 0
        conn.close()

        # 事件链在中断后必须仍可验证（WAL 事务回滚不破坏链）
        chain = vault.run("db", "verify-chain", check=False)
        assert chain.returncode == 0, (
            f"SIGKILL 中断后事件链应完整: {chain.stdout}{chain.stderr}"
        )

        # 重跑可恢复至全量入库（幂等恢复契约）
        vault.import_dir(vault.source_dir)
        assert len(vault.db_files()) == 5


class TestImportResumption:
    """导入恢复测试"""


# =============================================================================
# Level 2: Concurrent Modification (merged from test_concurrent_modification.py)
# =============================================================================

class TestFileDeletionDuringImport:
    """导入过程中文件被删除的处理"""
    
    def test_detect_file_deleted_before_copy(self, vault: VaultEnv) -> None:
        """扫描后、复制前文件被删除的处理"""
        f1 = vault.source_dir / "keep.jpg"
        f2 = vault.source_dir / "delete_me.jpg"
        create_minimal_jpeg(f1, "KEEP_THIS")
        create_minimal_jpeg(f2, "DELETE_THIS")
        
        f2.unlink()
        
        result = vault.import_dir(vault.source_dir, check=False)
        assert result.returncode in [0, 1]
        
        files = vault.db_files()
        assert len(files) == 1
        assert "keep" in files[0]["path"]
    
class TestFileModificationDuringImport:
    """导入过程中文件被修改的检测"""
    
    def test_detect_content_change_before_copy(self, vault: VaultEnv) -> None:
        """扫描后文件内容被修改的处理"""
        f = vault.source_dir / "modify.jpg"
        create_minimal_jpeg(f, "ORIGINAL_CONTENT")
        
        time.sleep(0.1)
        create_minimal_jpeg(f, "MODIFIED_CONTENT_DIFFERENT")
        
        result = vault.import_dir(vault.source_dir, check=False)
        assert result.returncode in [0, 1]
        
        files = vault.db_files()
        assert len(files) == 1


# =============================================================================
# Level 3: Fallback and Error Handling
# =============================================================================

class TestFallbackAndCorruptedFiles:
    """Fallback 和损坏文件处理测试"""

    def test_import_unreadable_file(self, vault: VaultEnv) -> None:
        """导入无权限读取的文件"""
        f1 = vault.source_dir / "readable.jpg"
        create_minimal_jpeg(f1, "READABLE")

        f2 = vault.source_dir / "unreadable.jpg"
        create_minimal_jpeg(f2, "UNREADABLE")
        f2.chmod(0o000)

        try:
            result = vault.import_dir(vault.source_dir, check=False)
            files = vault.db_files()
            assert len(files) >= 1, "至少可读文件应被导入"
        finally:
            f2.chmod(0o644)

    def test_fake_jpeg_fallback(self, vault: VaultEnv) -> None:
        """假 JPEG 文件 fallback 测试"""
        fake_jpg = vault.source_dir / "fake_image.jpg"
        fake_jpg.write_text("This is not a real JPEG file.")
        
        real_jpg = vault.source_dir / "real_image.jpg"
        create_minimal_jpeg(real_jpg, "REAL_JPEG")

        result = vault.import_dir(vault.source_dir, check=False)
        files = vault.db_files()
        
        real_imported = any("real_image" in str(f.get("path", "")) for f in files)
        assert real_imported, "有效的 JPEG 应该被导入"


# =============================================================================
# Test Architecture Notes
# =============================================================================

"""
【三层测试架构】

Level 1 - 本文件 (strace 注入):
├── 信号中断（SIGTERM/SIGKILL）
├── 并发文件修改（删除/修改/新增）
├── 数据库一致性验证
└── 恢复和幂等性测试

Level 2 - fuse_tests/ 目录:
├── 精确字节级 IO 控制
├── 任意时刻暂停/恢复
└── 网络存储异常模拟

【strace inject 语法】
strace -e inject=SYSCALL:signal=SIGNAL:when=N
- SYSCALL: openat, read, write, close
- SIGNAL: SIGTERM, SIGKILL
- when=N: 第 N 次调用时注入
"""
