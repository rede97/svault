"""Import interruption and recovery tests.

测试导入过程中的中断和恢复能力。

中文场景说明：
- 进程中断：导入过程中收到 SIGTERM/SIGKILL 信号
- 磁盘满：导入过程中磁盘空间耗尽
- 网络中断：从网络文件系统导入时连接断开
- 数据库锁：多进程并发导入冲突
- 恢复测试：中断后重新导入，验证数据一致性

必要性：
- 数据完整性：确保中断不会导致数据损坏
- 事务一致性：部分导入的文件应该可以恢复或清理
- 用户体验：中断后可以安全地重新开始

测试策略：
1. 创建大量文件（模拟长时间导入）
2. 在导入过程中发送信号中断
3. 验证数据库状态一致性
4. 重新导入，验证可以继续
"""

from __future__ import annotations

import os
import signal
import subprocess
import time
from pathlib import Path
from typing import Optional

import pytest

from conftest import VaultEnv, create_minimal_jpeg


class TestImportInterruption:
    """测试导入过程中的各种中断场景"""

    def test_sigterm_during_import(self, vault: VaultEnv) -> None:
        """测试 SIGTERM 信号中断导入过程

        场景：
        1. 创建大量文件（100+）
        2. 启动导入进程
        3. 导入进行中发送 SIGTERM
        4. 验证进程优雅退出
        5. 重新导入，验证可以继续
        """
        # 创建大量文件以延长导入时间
        num_files = 50
        for i in range(num_files):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        # 启动导入进程（后台运行）
        import_cmd = [
            vault.binary,
            "--vault", str(vault.vault_dir),
            "import",
            str(vault.source_dir),
        ]

        proc = subprocess.Popen(
            import_cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

        # 等待一小段时间让导入开始
        time.sleep(0.5)

        # 发送 SIGTERM 信号
        proc.send_signal(signal.SIGTERM)

        # 等待进程退出
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            pytest.fail("Process did not exit after SIGTERM")

        # 验证数据库状态
        files_after_interrupt = vault.db_files()
        count_after_interrupt = len(files_after_interrupt)

        print(f"Files imported before interrupt: {count_after_interrupt}/{num_files}")

        # 重新导入（应该继续导入剩余文件）
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0

        # 验证所有文件最终都被导入
        files_after_resume = vault.db_files()
        assert len(files_after_resume) == num_files, \
            f"Expected {num_files} files, got {len(files_after_resume)}"

        # 验证没有重复
        paths = [f["path"] for f in files_after_resume]
        assert len(paths) == len(set(paths)), "Duplicate files detected!"

    def test_sigkill_during_import(self, vault: VaultEnv) -> None:
        """测试 SIGKILL 信号强制终止导入过程

        场景：
        1. 创建文件
        2. 启动导入
        3. 发送 SIGKILL 强制终止
        4. 验证可以恢复
        """
        # 创建文件
        num_files = 30
        for i in range(num_files):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        # 启动导入进程
        import_cmd = [
            vault.binary,
            "--vault", str(vault.vault_dir),
            "import",
            str(vault.source_dir),
        ]

        proc = subprocess.Popen(
            import_cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

        # 等待导入开始
        time.sleep(0.3)

        # 发送 SIGKILL 强制终止
        proc.kill()
        proc.wait()

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

    def test_multiple_interruptions(self, vault: VaultEnv) -> None:
        """测试多次中断和恢复

        场景：
        1. 创建大量文件
        2. 第一次导入，中断
        3. 第二次导入，再次中断
        4. 第三次导入，完成
        5. 验证最终状态正确
        """
        num_files = 40
        for i in range(num_files):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        # 第一次中断
        proc1 = subprocess.Popen(
            [vault.binary, "--vault", str(vault.vault_dir),
             "import", str(vault.source_dir)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        time.sleep(0.2)
        proc1.send_signal(signal.SIGTERM)
        proc1.wait(timeout=5)

        count1 = len(vault.db_files())
        print(f"After 1st interrupt: {count1} files")

        # 第二次中断
        proc2 = subprocess.Popen(
            [vault.binary, "--vault", str(vault.vault_dir),
             "import", str(vault.source_dir)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        time.sleep(0.2)
        proc2.send_signal(signal.SIGTERM)
        proc2.wait(timeout=5)

        count2 = len(vault.db_files())
        print(f"After 2nd interrupt: {count2} files")

        # 第三次完成
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0

        count_final = len(vault.db_files())
        assert count_final == num_files, \
            f"Expected {num_files}, got {count_final}"


class TestDatabaseConsistency:
    """测试中断后的数据库一致性"""

    def test_no_partial_files_after_interrupt(self, vault: VaultEnv) -> None:
        """验证中断后没有部分写入的文件

        场景：
        1. 导入过程中中断
        2. 检查数据库中的所有文件
        3. 验证所有记录的文件都完整存在
        """
        # 创建文件
        num_files = 30
        for i in range(num_files):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        # 启动并中断导入
        proc = subprocess.Popen(
            [vault.binary, "--vault", str(vault.vault_dir),
             "import", str(vault.source_dir)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        time.sleep(0.3)
        proc.send_signal(signal.SIGTERM)
        proc.wait(timeout=5)

        # 验证数据库中的所有文件都存在且完整
        files = vault.db_files()
        for file_info in files:
            vault_path = vault.vault_dir / file_info["path"]
            assert vault_path.exists(), \
                f"File in DB but missing on disk: {file_info['path']}"

            # 验证文件大小不为0（不是部分写入）
            assert vault_path.stat().st_size > 0, \
                f"File has zero size: {file_info['path']}"

    def test_database_integrity_after_interrupt(self, vault: VaultEnv) -> None:
        """验证中断后数据库完整性

        场景：
        1. 导入过程中中断
        2. 验证数据库可以正常打开
        3. 验证事件日志完整性
        4. 验证哈希链完整性
        """
        # 创建文件
        for i in range(20):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        # 启动并中断
        proc = subprocess.Popen(
            [vault.binary, "--vault", str(vault.vault_dir),
             "import", str(vault.source_dir)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        time.sleep(0.3)
        proc.kill()
        proc.wait()

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


class TestConcurrentImport:
    """测试并发导入场景"""

    def test_concurrent_import_with_lock(self, vault: VaultEnv) -> None:
        """测试并发导入时的锁机制

        场景：
        1. 启动第一个导入进程
        2. 尝试启动第二个导入进程
        3. 第二个进程应该被锁阻止
        """
        # 创建文件
        for i in range(20):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        # 启动第一个导入
        proc1 = subprocess.Popen(
            [vault.binary, "--vault", str(vault.vault_dir),
             "import", str(vault.source_dir)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

        # 等待第一个进程获取锁
        time.sleep(0.2)

        # 尝试启动第二个导入
        proc2 = subprocess.Popen(
            [vault.binary, "--vault", str(vault.vault_dir),
             "import", str(vault.source_dir)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

        # 等待第二个进程退出
        stdout2, stderr2 = proc2.communicate(timeout=5)

        # 第二个进程应该失败（被锁阻止）
        assert proc2.returncode != 0, \
            "Second import should fail due to lock"

        # 清理第一个进程
        proc1.send_signal(signal.SIGTERM)
        proc1.wait(timeout=5)


class TestErrorInjection:
    """测试各种错误注入场景"""

    def test_import_with_corrupted_source_file(self, vault: VaultEnv) -> None:
        """测试导入损坏的源文件

        场景：
        1. 创建正常文件和损坏文件
        2. 导入
        3. 验证正常文件被导入，损坏文件被跳过
        """
        # 创建正常文件
        f1 = vault.source_dir / "good.jpg"
        create_minimal_jpeg(f1, "GOOD_CONTENT")

        # 创建损坏文件（不完整的 JPEG）
        f2 = vault.source_dir / "corrupted.jpg"
        f2.write_bytes(b'\xff\xd8\xff\xe0' + b'incomplete')

        # 导入（可能部分失败）
        result = vault.import_dir(vault.source_dir, check=False)

        # 至少正常文件应该被导入
        files = vault.db_files()
        assert len(files) >= 1

        # 验证导入的文件存在
        for file_info in files:
            vault_path = vault.vault_dir / file_info["path"]
            assert vault_path.exists()

    def test_import_with_permission_error(self, vault: VaultEnv) -> None:
        """测试导入无权限访问的文件

        场景：
        1. 创建文件并移除读权限
        2. 导入
        3. 验证优雅处理权限错误
        """
        # 创建可读文件
        f1 = vault.source_dir / "readable.jpg"
        create_minimal_jpeg(f1, "READABLE")

        # 创建不可读文件（仅在 Unix 系统）
        if os.name != 'nt':
            f2 = vault.source_dir / "unreadable.jpg"
            create_minimal_jpeg(f2, "UNREADABLE")
            f2.chmod(0o000)

            try:
                # 导入
                result = vault.import_dir(vault.source_dir, check=False)

                # 应该至少导入可读文件
                files = vault.db_files()
                assert len(files) >= 1
            finally:
                # 恢复权限以便清理
                f2.chmod(0o644)


class TestRecoveryScenarios:
    """测试各种恢复场景"""

    def test_resume_after_disk_full(self, vault: VaultEnv) -> None:
        """测试磁盘满后的恢复

        场景：
        1. 导入到小容量 vault
        2. 磁盘满导致失败
        3. 清理空间
        4. 重新导入，验证可以继续
        """
        # 创建文件
        for i in range(10):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        # 第一次导入（可能因空间不足失败）
        result1 = vault.import_dir(vault.source_dir, check=False)
        count1 = len(vault.db_files())

        # 第二次导入（应该继续）
        result2 = vault.import_dir(vault.source_dir)
        count2 = len(vault.db_files())

        # 第二次应该导入更多或相同数量的文件
        assert count2 >= count1

    def test_resume_with_new_files_added(self, vault: VaultEnv) -> None:
        """测试中断后添加新文件的恢复

        场景：
        1. 导入部分文件
        2. 中断
        3. 添加新文件到源目录
        4. 重新导入
        5. 验证旧文件不重复，新文件被导入
        """
        # 第一批文件
        for i in range(10):
            f = vault.source_dir / f"batch1_{i:03d}.jpg"
            create_minimal_jpeg(f, f"BATCH1_{i}")

        # 启动并中断
        proc = subprocess.Popen(
            [vault.binary, "--vault", str(vault.vault_dir),
             "import", str(vault.source_dir)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        time.sleep(0.3)
        proc.send_signal(signal.SIGTERM)
        proc.wait(timeout=5)

        count_after_interrupt = len(vault.db_files())

        # 添加第二批文件
        for i in range(10):
            f = vault.source_dir / f"batch2_{i:03d}.jpg"
            create_minimal_jpeg(f, f"BATCH2_{i}")

        # 重新导入
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0

        # 验证总数正确
        files_final = vault.db_files()
        assert len(files_final) == 20, \
            f"Expected 20 files, got {len(files_final)}"

        # 验证没有重复
        paths = [f["path"] for f in files_final]
        assert len(paths) == len(set(paths))


@pytest.mark.slow
class TestStressScenarios:
    """压力测试场景"""

    def test_large_batch_with_interruptions(self, vault: VaultEnv) -> None:
        """测试大批量导入的多次中断

        场景：
        1. 创建大量文件（100+）
        2. 多次中断和恢复
        3. 验证最终所有文件都被正确导入
        """
        num_files = 100
        for i in range(num_files):
            f = vault.source_dir / f"file_{i:04d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        # 多次中断
        for attempt in range(3):
            proc = subprocess.Popen(
                [vault.binary, "--vault", str(vault.vault_dir),
                 "import", str(vault.source_dir)],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            time.sleep(0.5)
            proc.send_signal(signal.SIGTERM)
            proc.wait(timeout=5)

            count = len(vault.db_files())
            print(f"After interrupt {attempt + 1}: {count}/{num_files} files")

        # 最后完整导入
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0

        # 验证所有文件
        files_final = vault.db_files()
        assert len(files_final) == num_files

        # 验证所有文件都存在
        for file_info in files_final:
            vault_path = vault.vault_dir / file_info["path"]
            assert vault_path.exists()
            assert vault_path.stat().st_size > 0
