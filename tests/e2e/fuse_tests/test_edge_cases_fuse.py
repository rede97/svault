"""极端边界 FUSE 测试

判据来源：docs/failure-handling.md §8.3。覆盖故障规则与边界输入的组合：
- 0 字节文件无内容读取 → 按读取范围触发的故障规则永不生效
- 0 字节文件互判 duplicate（§3.1：size=0 且 CRC 相同）
"""

from __future__ import annotations

import errno
import sys
from pathlib import Path

import pytest

# fuse_tests 是包（含 __init__.py），需手动把本目录加入 sys.path
sys.path.insert(0, str(Path(__file__).parent))

from conftest import VaultEnv
from fault_inject_fs import FaultInjectedFS, FaultRule

pytestmark = [pytest.mark.fuse, pytest.mark.slow]


class TestEmptyFileFaultCombinations:
    """空文件与故障规则的组合（§8.3 边界）"""

    def test_empty_file_corrupt_rule_never_fires(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """空文件 + corrupt 规则：无内容读取，规则不触发，正常导入

        判据：
        1. import exit 0，空文件入库
        2. fs.stats.corrupt_count == 0（无读取发生，规则从未触发）
        3. verify 通过
        """
        vault, fuse_mount, fs = vault_with_fuse_source
        (vault.source_dir / "empty.jpg").write_bytes(b"")

        fs.add_rule(
            FaultRule(path="/empty.jpg", offset=0, action="corrupt")
        )

        result = vault.import_dir(fuse_mount)
        assert result.returncode == 0, f"空文件导入应成功: {result.stderr}"
        assert len(vault.db_files()) == 1
        assert fs.get_stats().corrupt_count == 0, (
            "空文件无内容读取，corrupt 规则不应触发"
        )

        verify = vault.run("verify", check=False)
        assert verify.returncode == 0, f"verify 应通过: {verify.stderr}"

    def test_empty_file_eio_rule_never_fires(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """空文件 + EIO 规则：无内容读取，规则不触发，正常导入

        判据：import exit 0，空文件入库，error_count == 0
        """
        vault, fuse_mount, fs = vault_with_fuse_source
        (vault.source_dir / "empty_eio.jpg").write_bytes(b"")

        fs.add_rule(
            FaultRule(
                path="/empty_eio.jpg",
                offset=0,
                action="error",
                error_code=errno.EIO,
            )
        )

        result = vault.import_dir(fuse_mount)
        assert result.returncode == 0, (
            f"空文件无读取可失败，导入应成功: {result.stderr}"
        )
        assert len(vault.db_files()) == 1
        assert fs.get_stats().error_count == 0, (
            "空文件无内容读取，EIO 规则不应触发"
        )

    def test_empty_files_dedup(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """同扩展名空文件互判 duplicate（§3.1：size=0 且 CRC 相同）

        判据：两个空 .jpg 导入后 DB 仅一条记录
        """
        vault, fuse_mount, fs = vault_with_fuse_source
        (vault.source_dir / "e1.jpg").write_bytes(b"")
        (vault.source_dir / "e2.jpg").write_bytes(b"")

        result = vault.import_dir(fuse_mount)
        assert result.returncode == 0, f"导入应成功: {result.stderr}"
        assert len(vault.db_files()) == 1, (
            "同扩展名 0 字节文件 size=0 且 CRC 相同，应互判 duplicate（§3.1）"
        )
