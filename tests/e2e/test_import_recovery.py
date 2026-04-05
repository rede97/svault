"""Import recovery and idempotency tests.

测试导入的恢复能力和幂等性（不使用信号中断）。

中文场景说明：
- 幂等性测试：多次导入相同文件不会产生重复
- 增量导入：添加新文件后继续导入
- 部分失败恢复：部分文件失败后可以重新导入
- 混合场景：新旧文件混合导入

必要性：
- 数据一致性：确保导入操作是幂等的
- 用户体验：用户可以安全地重复执行导入
- 错误恢复：部分失败不影响整体恢复

这些测试不依赖信号中断，更稳定可靠。
"""

from __future__ import annotations

import shutil
import time
from pathlib import Path

import pytest

from conftest import VaultEnv, create_minimal_jpeg


class TestImportIdempotency:
    """测试导入的幂等性"""

    def test_reimport_same_files_no_duplicates(self, vault: VaultEnv) -> None:
        """多次导入相同文件不产生重复

        场景：
        1. 创建文件并导入
        2. 再次导入相同文件
        3. 验证数据库中只有一份
        """
        # 创建文件
        for i in range(10):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        # 第一次导入
        result1 = vault.import_dir(vault.source_dir)
        assert result1.returncode == 0
        count1 = len(vault.db_files())
        assert count1 == 10

        # 第二次导入（相同文件）
        result2 = vault.import_dir(vault.source_dir)
        assert result2.returncode == 0
        count2 = len(vault.db_files())

        # 应该还是10个文件（没有重复）
        assert count2 == 10, f"Expected 10 files, got {count2}"

        # 验证路径唯一性
        files = vault.db_files()
        paths = [f["path"] for f in files]
        assert len(paths) == len(set(paths)), "Duplicate paths found!"

    def test_reimport_after_source_cleanup(self, vault: VaultEnv) -> None:
        """清理源目录后重新导入

        场景：
        1. 导入文件
        2. 清空源目录
        3. 重新复制文件到源目录
        4. 再次导入
        5. 验证识别为重复
        """
        # 第一次导入
        for i in range(5):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        vault.import_dir(vault.source_dir)
        assert len(vault.db_files()) == 5

        # 清空源目录
        for f in vault.source_dir.iterdir():
            if f.is_file():
                f.unlink()

        # 重新创建相同内容的文件
        for i in range(5):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        # 再次导入
        vault.import_dir(vault.source_dir)

        # 应该还是5个文件
        assert len(vault.db_files()) == 5

    def test_multiple_reimports(self, vault: VaultEnv) -> None:
        """多次重复导入

        场景：
        1. 创建文件
        2. 导入5次
        3. 验证始终只有一份
        """
        # 创建文件
        for i in range(8):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"UNIQUE_{i}")

        # 导入多次
        for attempt in range(5):
            result = vault.import_dir(vault.source_dir)
            assert result.returncode == 0

            count = len(vault.db_files())
            assert count == 8, f"Attempt {attempt + 1}: expected 8, got {count}"


class TestIncrementalImport:
    """测试增量导入"""

    def test_add_new_files_between_imports(self, vault: VaultEnv) -> None:
        """在两次导入之间添加新文件

        场景：
        1. 导入第一批文件
        2. 添加第二批文件
        3. 再次导入
        4. 验证两批文件都在
        """
        # 第一批
        for i in range(5):
            f = vault.source_dir / f"batch1_{i:03d}.jpg"
            create_minimal_jpeg(f, f"BATCH1_{i}")

        vault.import_dir(vault.source_dir)
        assert len(vault.db_files()) == 5

        # 第二批
        for i in range(5):
            f = vault.source_dir / f"batch2_{i:03d}.jpg"
            create_minimal_jpeg(f, f"BATCH2_{i}")

        vault.import_dir(vault.source_dir)
        files = vault.db_files()
        assert len(files) == 10

        # 验证两批文件都存在
        paths = [f["path"] for f in files]
        batch1_count = sum(1 for p in paths if "batch1" in p)
        batch2_count = sum(1 for p in paths if "batch2" in p)
        assert batch1_count == 5
        assert batch2_count == 5

    def test_incremental_import_large_batches(self, vault: VaultEnv) -> None:
        """大批量增量导入

        场景：
        1. 分3批导入，每批20个文件
        2. 验证最终有60个文件
        """
        for batch in range(3):
            # 添加新文件
            for i in range(20):
                f = vault.source_dir / f"batch{batch}_{i:03d}.jpg"
                create_minimal_jpeg(f, f"BATCH{batch}_{i}")

            # 导入
            result = vault.import_dir(vault.source_dir)
            assert result.returncode == 0

            expected_count = (batch + 1) * 20
            actual_count = len(vault.db_files())
            assert actual_count == expected_count, \
                f"Batch {batch}: expected {expected_count}, got {actual_count}"

    def test_mixed_new_and_existing_files(self, vault: VaultEnv) -> None:
        """混合新旧文件导入

        场景：
        1. 导入5个文件
        2. 删除源目录中的2个文件（但vault中保留）
        3. 添加5个新文件到源目录
        4. 再次导入
        5. 验证总共10个文件（5个旧 + 5个新）
        """
        # 第一批：5个文件
        for i in range(5):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        vault.import_dir(vault.source_dir)
        assert len(vault.db_files()) == 5

        # 从源目录删除2个文件（vault中仍保留）
        (vault.source_dir / "file_003.jpg").unlink()
        (vault.source_dir / "file_004.jpg").unlink()

        # 添加5个新文件
        for i in range(5, 10):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        # 再次导入
        vault.import_dir(vault.source_dir)

        # 应该有10个文件（5个旧的已在vault + 5个新导入）
        files = vault.db_files()
        assert len(files) == 10


class TestPartialFailureRecovery:
    """测试部分失败后的恢复"""

    def test_recover_after_some_files_deleted(self, vault: VaultEnv) -> None:
        """部分文件被删除后恢复

        场景：
        1. 创建10个文件
        2. 删除其中3个
        3. 导入（7个成功）
        4. 恢复被删除的文件
        5. 再次导入
        6. 验证最终10个文件都在
        """
        # 创建文件
        for i in range(10):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        # 删除部分文件
        deleted_files = []
        for i in [2, 5, 8]:
            f = vault.source_dir / f"file_{i:03d}.jpg"
            deleted_files.append((f, f.read_bytes()))
            f.unlink()

        # 第一次导入（部分成功）
        result1 = vault.import_dir(vault.source_dir, check=False)
        count1 = len(vault.db_files())
        assert count1 == 7

        # 恢复被删除的文件
        for f, content in deleted_files:
            f.write_bytes(content)

        # 第二次导入
        result2 = vault.import_dir(vault.source_dir)
        assert result2.returncode == 0

        # 应该有10个文件
        files = vault.db_files()
        assert len(files) == 10

    def test_recover_after_corrupted_files(self, vault: VaultEnv) -> None:
        """损坏文件修复后恢复

        场景：
        1. 创建正常文件和损坏文件
        2. 导入（正常文件成功，损坏文件可能被导入）
        3. 修复损坏文件
        4. 再次导入
        5. 验证所有文件都在
        """
        # 创建正常文件
        for i in range(5):
            f = vault.source_dir / f"good_{i:03d}.jpg"
            create_minimal_jpeg(f, f"GOOD_{i}")

        # 创建损坏文件
        corrupted = []
        for i in range(3):
            f = vault.source_dir / f"bad_{i:03d}.jpg"
            f.write_bytes(b'\xff\xd8\xff\xe0' + b'incomplete')
            corrupted.append(f)

        # 第一次导入
        result1 = vault.import_dir(vault.source_dir, check=False)
        count1 = len(vault.db_files())
        assert count1 >= 5  # 至少正常文件被导入

        # 修复损坏文件
        for i, f in enumerate(corrupted):
            create_minimal_jpeg(f, f"FIXED_{i}")

        # 第二次导入
        result2 = vault.import_dir(vault.source_dir)
        assert result2.returncode == 0

        # 验证至少有8个文件（5个正常 + 3个修复后的）
        # 注意：损坏文件可能在第一次导入时也被导入了，所以可能有重复
        files = vault.db_files()
        assert len(files) >= 8, f"Expected at least 8 files, got {len(files)}"


class TestConcurrentSourceModification:
    """测试源目录并发修改场景（不使用信号）"""

    def test_files_added_during_import_window(self, vault: VaultEnv) -> None:
        """模拟导入窗口期间添加文件

        场景：
        1. 创建第一批文件
        2. 开始导入
        3. 在导入完成前添加第二批文件（模拟相机持续拍摄）
        4. 第二批文件在下次导入时被处理
        """
        # 第一批
        for i in range(10):
            f = vault.source_dir / f"first_{i:03d}.jpg"
            create_minimal_jpeg(f, f"FIRST_{i}")

        # 导入第一批
        vault.import_dir(vault.source_dir)
        assert len(vault.db_files()) == 10

        # 添加第二批（模拟导入期间新增）
        for i in range(5):
            f = vault.source_dir / f"second_{i:03d}.jpg"
            create_minimal_jpeg(f, f"SECOND_{i}")

        # 导入第二批
        vault.import_dir(vault.source_dir)
        assert len(vault.db_files()) == 15

    def test_files_modified_between_imports(self, vault: VaultEnv) -> None:
        """文件在两次导入之间被修改

        场景：
        1. 导入文件
        2. 修改源文件内容
        3. 再次导入
        4. 验证修改后的文件被识别为新文件
        """
        # 创建并导入
        f = vault.source_dir / "test.jpg"
        create_minimal_jpeg(f, "ORIGINAL")

        vault.import_dir(vault.source_dir)
        assert len(vault.db_files()) == 1

        # 修改文件内容
        time.sleep(0.1)  # 确保 mtime 变化
        create_minimal_jpeg(f, "MODIFIED_DIFFERENT_CONTENT")

        # 再次导入
        vault.import_dir(vault.source_dir)

        # 应该有2个文件（内容不同）
        files = vault.db_files()
        assert len(files) == 2


class TestLargeScaleRecovery:
    """大规模恢复测试"""

    def test_large_batch_incremental_import(self, vault: VaultEnv) -> None:
        """大批量增量导入

        场景：
        1. 分多批导入大量文件
        2. 验证每批都正确累加
        """
        total_files = 0
        batch_size = 20
        num_batches = 5

        for batch in range(num_batches):
            # 添加新文件
            for i in range(batch_size):
                f = vault.source_dir / f"batch{batch:02d}_file{i:03d}.jpg"
                create_minimal_jpeg(f, f"BATCH{batch}_FILE{i}")

            # 导入
            result = vault.import_dir(vault.source_dir)
            assert result.returncode == 0

            total_files += batch_size
            actual_count = len(vault.db_files())
            assert actual_count == total_files, \
                f"Batch {batch}: expected {total_files}, got {actual_count}"

        # 最终验证
        assert len(vault.db_files()) == num_batches * batch_size

    def test_reimport_after_vault_file_deletion(self, vault: VaultEnv) -> None:
        """Vault 文件被删除后重新导入

        场景：
        1. 导入文件
        2. 删除 vault 中的某些文件
        3. 使用 --force 重新导入
        4. 验证文件被恢复
        """
        # 创建并导入
        for i in range(10):
            f = vault.source_dir / f"file_{i:03d}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")

        vault.import_dir(vault.source_dir)
        files_before = vault.db_files()
        assert len(files_before) == 10

        # 删除 vault 中的某些文件
        for i in [2, 5, 8]:
            file_info = [f for f in files_before if f"file_{i:03d}" in f["path"]][0]
            vault_path = vault.vault_dir / file_info["path"]
            if vault_path.exists():
                vault_path.unlink()

        # 使用 --force 重新导入
        result = vault.import_dir(vault.source_dir, force=True)
        assert result.returncode == 0

        # 验证文件数量（可能有重复，因为 --force）
        files_after = vault.db_files()
        assert len(files_after) >= 10


class TestEdgeCases:
    """边界情况测试"""

    def test_empty_source_reimport(self, vault: VaultEnv) -> None:
        """空源目录重复导入

        场景：
        1. 导入空目录
        2. 再次导入
        3. 验证不会出错
        """
        # 第一次导入空目录
        result1 = vault.import_dir(vault.source_dir)
        assert result1.returncode == 0
        assert len(vault.db_files()) == 0

        # 第二次导入空目录
        result2 = vault.import_dir(vault.source_dir)
        assert result2.returncode == 0
        assert len(vault.db_files()) == 0

    def test_single_file_multiple_imports(self, vault: VaultEnv) -> None:
        """单个文件多次导入

        场景：
        1. 创建单个文件
        2. 导入10次
        3. 验证只有一份
        """
        f = vault.source_dir / "single.jpg"
        create_minimal_jpeg(f, "SINGLE_FILE")

        # 导入多次
        for _ in range(10):
            result = vault.import_dir(vault.source_dir)
            assert result.returncode == 0

        # 应该只有1个文件
        files = vault.db_files()
        assert len(files) == 1

    def test_import_with_subdirectories(self, vault: VaultEnv) -> None:
        """包含子目录的导入和重新导入

        场景：
        1. 创建多层目录结构
        2. 导入
        3. 添加更多子目录
        4. 再次导入
        5. 验证所有文件都被找到
        """
        # 第一批：多层目录
        for i in range(3):
            subdir = vault.source_dir / f"dir{i}"
            subdir.mkdir()
            for j in range(3):
                f = subdir / f"file_{j:03d}.jpg"
                create_minimal_jpeg(f, f"DIR{i}_FILE{j}")

        vault.import_dir(vault.source_dir)
        assert len(vault.db_files()) == 9

        # 第二批：更多子目录
        for i in range(3, 5):
            subdir = vault.source_dir / f"dir{i}"
            subdir.mkdir()
            for j in range(3):
                f = subdir / f"file_{j:03d}.jpg"
                create_minimal_jpeg(f, f"DIR{i}_FILE{j}")

        vault.import_dir(vault.source_dir)
        assert len(vault.db_files()) == 15
