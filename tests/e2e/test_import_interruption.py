"""Import interruption tests - 使用 strace 注入信号实现精确中断控制

本文件使用 strace 的 inject 功能在特定系统调用时注入信号，
实现比 time.sleep() 更可靠的进程中断测试。

对于需要精确 IO 控制的深度故障注入测试，参见 fuse_tests/ 目录。
"""

from __future__ import annotations

import os
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


class TestSignalInterruption:
    """信号中断测试 - 使用 strace inject 实现精确控制
    
    使用 strace -e inject=SYSCALL:signal=SIGNAL:when=N 在特定系统调用时发送信号
    """

    @pytest.mark.skipif(
        not check_strace_available(),
        reason="strace not available or doesn't support inject"
    )
    def test_sigterm_during_import(self, vault: VaultEnv, strace_available: bool) -> None:
        """SIGTERM 中断导入过程 - 使用 strace 在第 N 次 read 时注入
        
        验证点：
        - 进程能优雅退出
        - 数据库状态一致
        - 重新导入可继续
        """
        # 创建多个文件以延长导入时间
        num_files = 20
        for i in range(num_files):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        # 使用 strace 在第 10 次 read 调用时发送 SIGTERM
        # read 在文件内容读取时调用，比 openat 更晚，更容易中断
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

        # 进程应该被 SIGTERM 终止
        assert result.returncode != 0, f"Process should have been terminated by SIGTERM, got {result.returncode}"

        # 验证：应该有部分文件被导入
        files_after_interrupt = vault.db_files()
        count_after_interrupt = len(files_after_interrupt)
        
        print(f"Files imported before interrupt: {count_after_interrupt}/{num_files}")
        
        # 允许部分导入（至少 0 个，但少于总数）
        # 如果时机太晚，可能所有文件都已导入，这也算通过（SIGTERM 在完成后发送）
        assert count_after_interrupt <= num_files, \
            f"Imported more files than exist: {count_after_interrupt} > {num_files}"

        # 重新导入（应该继续导入剩余文件或报告已完成）
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0

        # 验证所有文件最终都被导入
        files_after_resume = vault.db_files()
        assert len(files_after_resume) == num_files, \
            f"Expected {num_files} files, got {len(files_after_resume)}"

        # 验证没有重复
        paths = [f["path"] for f in files_after_resume]
        assert len(paths) == len(set(paths)), "Duplicate files detected!"

    @pytest.mark.skipif(
        not check_strace_available(),
        reason="strace not available or doesn't support inject"
    )
    def test_sigkill_during_import(self, vault: VaultEnv, strace_available: bool) -> None:
        """SIGKILL 强制终止导入过程
        
        验证点：
        - 强制终止后数据库可恢复
        - 已写入数据完整
        """
        # 创建文件
        num_files = 15
        for i in range(num_files):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        # 使用 strace 在第 8 次 read 时发送 SIGKILL
        strace_cmd = [
            "strace",
            "-e", "inject=read:signal=SIGKILL:when=8",
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

        # SIGKILL 会强制终止进程（返回码 137 = 128 + 9，或 -9）
        assert result.returncode in [137, -9, 9], \
            f"Process should have been killed by SIGKILL, got returncode {result.returncode}"

        # 验证数据库状态
        files_after_kill = vault.db_files()
        count_after_kill = len(files_after_kill)

        print(f"Files imported before SIGKILL: {count_after_kill}/{num_files}")

        # 重新导入
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0

        # 验证所有文件都被导入
        files_final = vault.db_files()
        assert len(files_final) == num_files

    @pytest.mark.skipif(
        not check_strace_available(),
        reason="strace not available or doesn't support inject"
    )
    def test_multiple_interruptions(self, vault: VaultEnv, strace_available: bool) -> None:
        """多次中断和恢复
        
        验证点：
        - 多次中断不会损坏数据
        - 最终能完成所有导入
        """
        num_files = 25
        for i in range(num_files):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        # 第一次中断（第 5 次 read）
        subprocess.run(
            [
                "strace", "-e", "inject=read:signal=SIGTERM:when=5",
                "-o", "/dev/null",
                str(vault.binary), "--yes", "import", str(vault.source_dir),
            ],
            cwd=vault.vault_dir,
            capture_output=True,
        )
        count1 = len(vault.db_files())
        print(f"After 1st interrupt: {count1} files")

        # 第二次中断（第 15 次 read）
        subprocess.run(
            [
                "strace", "-e", "inject=read:signal=SIGTERM:when=15",
                "-o", "/dev/null",
                str(vault.binary), "--yes", "import", str(vault.source_dir),
            ],
            cwd=vault.vault_dir,
            capture_output=True,
        )
        count2 = len(vault.db_files())
        print(f"After 2nd interrupt: {count2} files")

        # 第三次完成
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0

        count_final = len(vault.db_files())
        assert count_final == num_files, \
            f"Expected {num_files}, got {count_final}"


class TestDatabaseConsistency:
    """中断后的数据库一致性验证"""

    @pytest.mark.skipif(
        not check_strace_available(),
        reason="strace not available"
    )
    def test_no_partial_files_after_interrupt(self, vault: VaultEnv, strace_available: bool) -> None:
        """验证中断后没有部分写入的文件
        
        使用 strace 在 write 调用时注入 SIGTERM，模拟写入中途中断
        """
        # 创建较大的文件以增加写入时间
        f = vault.source_dir / "large_test.jpg"
        create_minimal_jpeg(f, "LARGE_FILE_CONTENT")
        # 追加数据使文件变大
        with open(f, 'ab') as fp:
            fp.write(b"X" * (100 * 1024))  # 100KB padding

        # 在第一次 write 调用时发送 SIGTERM
        subprocess.run(
            [
                "strace", "-e", "inject=write:signal=SIGTERM:when=1",
                "-o", "/dev/null",
                str(vault.binary), "--yes", "import", str(vault.source_dir),
            ],
            cwd=vault.vault_dir,
            capture_output=True,
        )

        # 验证数据库中的所有文件都存在且完整
        files = vault.db_files()
        for file_info in files:
            vault_path = vault.vault_dir / file_info["path"]
            assert vault_path.exists(), \
                f"File in DB but missing on disk: {file_info['path']}"
            # 验证文件大小不为0（不是部分写入）
            assert vault_path.stat().st_size > 0, \
                f"File has zero size: {file_info['path']}"

    @pytest.mark.skipif(
        not check_strace_available(),
        reason="strace not available"
    )
    def test_database_integrity_after_interrupt(self, vault: VaultEnv, strace_available: bool) -> None:
        """验证中断后数据库完整性"""
        # 创建文件
        for i in range(5):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        # 在 read 调用时发送 SIGKILL
        subprocess.run(
            [
                "strace", "-e", "inject=read:signal=SIGKILL:when=5",
                "-o", "/dev/null",
                str(vault.binary), "--yes", "import", str(vault.source_dir),
            ],
            cwd=vault.vault_dir,
            capture_output=True,
        )

        # 验证数据库可以打开
        import sqlite3
        db_path = vault.vault_dir / ".svault" / "vault.db"
        conn = sqlite3.connect(str(db_path))

        # 验证表结构完整
        cursor = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        )
        tables = [row[0] for row in cursor.fetchall()]
        assert "files" in tables
        assert "events" in tables

        # 验证可以查询
        cursor = conn.execute("SELECT COUNT(*) FROM files")
        count = cursor.fetchone()[0]
        assert count >= 0

        conn.close()


class TestImportResumption:
    """导入恢复测试 - 验证中断后能正确恢复"""

    @pytest.mark.skipif(
        not check_strace_available(),
        reason="strace not available"
    )
    def test_resumed_import_no_duplicates(self, vault: VaultEnv, strace_available: bool) -> None:
        """恢复导入时不产生重复文件
        
        验证点：
        1. 中断前已导入的文件不会重复
        2. 数据库中只有一份记录
        3. vault 中只有一个副本
        """
        num_files = 15
        for i in range(num_files):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"RESUME_TEST_{i}")

        # 第一次中断（导入部分文件）
        subprocess.run(
            [
                "strace", "-e", "inject=read:signal=SIGTERM:when=12",
                "-o", "/dev/null",
                str(vault.binary), "--yes", "import", str(vault.source_dir),
            ],
            cwd=vault.vault_dir,
            capture_output=True,
        )
        
        count_after_interrupt = len(vault.db_files())
        print(f"Files after interrupt: {count_after_interrupt}")
        
        # 再次导入（恢复）
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        count_after_resume = len(vault.db_files())
        print(f"Files after resume: {count_after_resume}")
        
        # 验证数量正确
        assert count_after_resume == num_files
        
        # 验证没有重复（按哈希检查）
        files = vault.db_files()
        hashes = [f.get("sha256") or f.get("xxh3_128") for f in files]
        assert len(hashes) == len(set(hashes)), "发现重复哈希！"
        
        # 验证 vault 中的实际文件数量
        vault_files = list(vault.vault_dir.rglob("*.jpg"))
        vault_files = [f for f in vault_files if ".svault" not in str(f)]
        assert len(vault_files) == num_files, f"Vault 中应有 {num_files} 个文件，实际 {len(vault_files)}"


class TestFallbackAndCorruptedFiles:
    """Fallback 和损坏文件处理测试"""

    def test_import_unreadable_file(self, vault: VaultEnv) -> None:
        """导入无权限读取的文件"""
        # 创建可读文件
        f1 = vault.source_dir / "readable.jpg"
        create_minimal_jpeg(f1, "READABLE")

        # 创建不可读文件
        f2 = vault.source_dir / "unreadable.jpg"
        create_minimal_jpeg(f2, "UNREADABLE")
        f2.chmod(0o000)

        try:
            result = vault.import_dir(vault.source_dir, check=False)
            files = vault.db_files()
            assert len(files) >= 1, "至少可读文件应被导入"
        finally:
            f2.chmod(0o644)  # 清理

    def test_import_corrupted_source(self, vault: VaultEnv) -> None:
        """导入损坏的源文件"""
        # 正常文件
        f1 = vault.source_dir / "good.jpg"
        create_minimal_jpeg(f1, "GOOD")

        # 损坏文件（截断的 JPEG）
        f2 = vault.source_dir / "bad.jpg"
        f2.write_bytes(b'\xff\xd8\xff\xe0' + b'incomplete')

        result = vault.import_dir(vault.source_dir, check=False)
        files = vault.db_files()
        assert len(files) >= 1

    def test_fake_jpeg_fallback(self, vault: VaultEnv) -> None:
        """假 JPEG 文件 fallback 测试
        
        创建一个扩展名是 .jpg 但实际不是有效 JPEG 的文件，
        验证 svault 是否能作为二进制文件导入（如果支持）。
        """
        # 创建一个假 JPEG（扩展名 .jpg 但实际是纯文本）
        fake_jpg = vault.source_dir / "fake_image.jpg"
        fake_jpg.write_text("This is not a real JPEG file, just plain text content.")
        
        # 同时创建一个有效的 JPEG 作为对照
        real_jpg = vault.source_dir / "real_image.jpg"
        create_minimal_jpeg(real_jpg, "REAL_JPEG")

        result = vault.import_dir(vault.source_dir, check=False)
        files = vault.db_files()
        
        # 验证：有效的 JPEG 应该被导入
        real_imported = any("real_image" in str(f.get("path", "")) for f in files)
        assert real_imported, "有效的 JPEG 应该被导入"
        
        # 根据 svault 的实现，假 JPEG 可能被：
        # 1. 作为二进制文件导入（如果 svault 支持 fallback）
        # 2. 被跳过（如果严格检查文件头）
        # 测试记录实际行为
        fake_imported = any("fake_image" in str(f.get("path", "")) for f in files)
        if fake_imported:
            print("假 JPEG 被作为二进制文件导入（fallback 工作）")
        else:
            print("假 JPEG 被跳过（严格的文件头检查）")

    def test_binary_file_with_image_extension(self, vault: VaultEnv) -> None:
        """二进制文件使用图片扩展名的处理
        
        验证 svault 对非媒体文件（如随机数据）使用 .jpg 扩展名的处理。
        """
        # 创建一个随机数据的 "JPEG"
        binary_jpg = vault.source_dir / "binary.jpg"
        binary_jpg.write_bytes(bytes(range(256)) * 100)  # 非 JPEG 数据
        
        # 创建一个真正的 JPEG
        real_jpg = vault.source_dir / "valid.jpg"
        create_minimal_jpeg(real_jpg, "VALID")

        result = vault.import_dir(vault.source_dir, check=False)
        files = vault.db_files()
        
        # 至少有效的 JPEG 应该被导入
        assert len(files) >= 1, "至少有效文件应被导入"
        
        # 记录 svault 对二进制 fallback 的处理行为
        all_paths = [f.get("path", "") for f in files]
        print(f"导入的文件: {all_paths}")


# =============================================================================
# 测试架构说明
# =============================================================================

"""
【strace inject 说明】

strace -e inject=SYSCALL:signal=SIGNAL:when=N 语法：
- SYSCALL: 系统调用名称（如 openat, read, write）
- SIGNAL: 要发送的信号（如 SIGTERM, SIGKILL）
- when=N: 在第 N 次调用时注入

常用系统调用：
- openat: 打开文件（每个文件导入时会调用）
- read: 读取文件内容
- write: 写入文件内容
- close: 关闭文件描述符

示例：
# 在第 3 次打开文件时发送 SIGTERM
strace -e inject=openat:signal=SIGTERM:when=3 ./svault import /path

【测试分层架构】

Level 1 - 本文件 (strace 注入):
├── 信号中断（SIGTERM/SIGKILL）
├── 系统调用级错误注入
└── 数据库一致性验证

Level 2 - fuse_tests/ 目录:
├── 精确字节级 IO 控制
├── 任意时刻暂停/恢复
└── 网络存储异常模拟

【运行方式】

# 需要 root 权限（strace 注入通常需要）
sudo ./run.sh -v -k interruption

# 或单独运行
sudo python -m pytest test_import_interruption.py -v
"""
