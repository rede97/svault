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
        """截断文件修复后重导入：锁定实测行为（2026-08-06 实测）

        现实契约（与原注释的想象不同，svault 无格式校验）：
        1. 截断 JPEG **会被正常导入**（元数据解析失败 → 仅 CRC 指纹路径，
           file-identity.md §Stage 3 fallback）：5 好 + 3 截断 = 8 条记录
        2. 修复后内容变化 → 判为新文件；目标路径已被同名旧文件占用 →
           按 rename_template 改名 .1 另存：再 +3 条 = 11 条
        """
        # 创建正常文件
        for i in range(5):
            f = vault.source_dir / f"good_{i:03d}.jpg"
            create_minimal_jpeg(f, f"GOOD_{i}")

        # 创建截断文件（内容互不相同，避免互相判重）
        corrupted = []
        for i in range(3):
            f = vault.source_dir / f"bad_{i:03d}.jpg"
            f.write_bytes(b'\xff\xd8\xff\xe0' + f'incomplete_{i}'.encode())
            corrupted.append(f)

        # 第一次导入：截断不阻止导入（无格式校验），exit 0，8 条全入库
        result1 = vault.import_dir(vault.source_dir, check=False)
        assert result1.returncode == 0, f"截断文件不应导致失败: {result1.stderr}"
        assert len(vault.db_files()) == 8, "5 好 + 3 截断应全部入库"

        # 修复截断文件（内容改变）
        for i, f in enumerate(corrupted):
            create_minimal_jpeg(f, f"FIXED_{i}")

        # 第二次导入：修复版是新内容 → 判新；同名旧文件在库 → 改名 .1
        result2 = vault.import_dir(vault.source_dir)
        assert result2.returncode == 0

        files = vault.db_files()
        assert len(files) == 11, f"8 旧 + 3 修复版应共 11 条，实际 {len(files)}"
        renamed = [f for f in files if ".1." in f["path"]]
        assert len(renamed) == 3, "3 个修复版应按 rename_template 改名 .1 另存"


class TestConcurrentSourceModification:
    """测试源目录并发修改场景（不使用信号）"""

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


class TestEdgeCases:
    """边界情况测试"""

    def test_reimport_after_vault_file_moved_detects_duplicate(self, vault: VaultEnv) -> None:
        """Vault 文件被移动到 vault 内部新位置后重新导入

        场景：
        1. 导入文件到 vault/archive/2023/
        2. 用户手动移动文件到 vault/archive/2024/ (在 vault 内部)
        3. 重新导入相同文件
        4. 期望：检测到是 vault 内部移动，不应该创建重复记录
        5. 应该提示用户使用 reconcile 命令

        期望结果：
        - 导入时应该检测到 CRC/hash 匹配
        - 但原路径 (2023/) 的文件已不存在
        - 应该识别为 vault-internal move
        - 不应该创建新记录（避免重复）
        - 应该提示用户使用 svault reconcile
        """
        # 创建并导入文件
        for i in range(3):
            f = vault.source_dir / f"photo_{i:03d}.jpg"
            create_minimal_jpeg(f, f"MOVE_TEST_{i}")

        vault.import_dir(vault.source_dir)
        files_before = vault.db_files()
        assert len(files_before) == 3

        # 模拟用户移动：找到导入的文件并移动到新位置
        for f in files_before:
            old_path = vault.vault_dir / f["path"]
            # 移动到新的子目录
            new_dir = vault.vault_dir / "relocated"
            new_dir.mkdir(parents=True, exist_ok=True)
            new_path = new_dir / Path(f["path"]).name
            if old_path.exists():
                old_path.rename(new_path)

        # 重新导入（不使用 --force）
        result = vault.import_dir(vault.source_dir)
        combined = result.stderr + result.stdout

        # 不应该创建新记录（文件已在 vault 中，只是移动了）
        files_after = vault.db_files()
        # 应该仍然是 3 个文件（没有重复）
        assert len(files_after) == 3, \
            f"Expected 3 files (no duplicates), got {len(files_after)}"

        # 应该提示检测到重复或移动
        # 可能显示 "Duplicate" 或 "Already in vault" 或建议使用 reconcile
        assert (
            "duplicate" in combined.lower() or
            "already" in combined.lower() or
            "moved" in combined.lower() or
            "update" in combined.lower()
        ), f"Should detect vault-internal move or duplicate:\n{combined}"

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
