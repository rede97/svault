"""FUSE 故障注入基础设施 smoke 测试

验证三项基础设施行为（判据见 docs/failure-handling.md §8.4）：

- INFRA-1: corrupt action 在 post-read 阶段修改返回缓冲区
- INFRA-2: 运行时规则管理（set_rules/clear_rules/enable_rule/disable_rule）
- INFRA-4: per-read 内容序列（corrupt_sequence）

同时回归验证既有 error/delay 行为不受影响。
"""

from __future__ import annotations

import errno
import os
import sys
import time
from pathlib import Path

import pytest

# 与 conftest 相同的 sys.path 处理：fuse_tests 是包（含 __init__.py），
# pytest 不会把本目录加入 sys.path，需手动加入以便导入 fault_inject_fs
# （同时让 conftest 中 fault_inject_fs_class fixture 的延迟导入可用）
sys.path.insert(0, str(Path(__file__).parent))

from fault_inject_fs import FaultInjectedFS, FaultRule

pytestmark = [pytest.mark.fuse, pytest.mark.slow]


def _write_source(source_dir: Path, name: str, data: bytes) -> Path:
    """在 FUSE 源目录中写入测试文件"""
    path = source_dir / name
    path.write_bytes(data)
    return path


def _read_mounted(mount_point: Path, name: str) -> bytes:
    """经 FUSE 挂载点读取整个文件

    读取前用 posix_fadvise(DONTNEED) 丢弃该文件的内核页缓存，
    确保每次读取都真正触发 FUSE read 回调（规避页缓存干扰）。
    """
    fd = os.open(mount_point / name, os.O_RDONLY)
    try:
        if hasattr(os, "posix_fadvise"):
            os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)
        chunks: list[bytes] = []
        while chunk := os.read(fd, 4096):
            chunks.append(chunk)
        return b"".join(chunks)
    finally:
        os.close(fd)


class TestCorruptAction:
    """INFRA-1: corrupt action 落地"""

    def test_corrupt_data_injection(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """corrupt_data 注入后 read 返回被修改的字节（越界部分截断）"""
        vault, fuse_mount, fs = vault_with_fuse_source
        assert isinstance(fs, FaultInjectedFS)

        original = b"A" * 4096
        _write_source(vault.source_dir, "corrupt_data.bin", original)
        fs.add_rule(FaultRule(
            path="/corrupt_data.bin",
            offset=100,
            action="corrupt",
            corrupt_data=b"BADSECTOR!",
        ))

        data = _read_mounted(fuse_mount, "corrupt_data.bin")

        assert data[100:110] == b"BADSECTOR!"
        assert data[:100] == original[:100]
        assert data[110:] == original[110:]
        assert fs.get_stats().corrupt_count >= 1

    def test_corrupt_xor_flip(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """corrupt_data=None 时触发位置字节被 XOR 0xFF（单字节位翻转，模拟坏道）"""
        vault, fuse_mount, fs = vault_with_fuse_source

        original = bytes(range(256)) * 16
        _write_source(vault.source_dir, "corrupt_xor.bin", original)
        fs.add_rule(FaultRule(
            path="/corrupt_xor.bin",
            offset=200,
            action="corrupt",
            corrupt_data=None,
        ))

        data = _read_mounted(fuse_mount, "corrupt_xor.bin")

        assert len(data) == len(original)
        assert data[200] == (original[200] ^ 0xFF)
        assert data[:200] == original[:200]
        assert data[201:] == original[201:]


class TestCorruptSequence:
    """INFRA-4: per-read 内容序列"""

    def test_corrupt_sequence(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """两次读返回序列中不同内容，第三次读得到序列最后一个元素"""
        vault, fuse_mount, fs = vault_with_fuse_source

        original = b"C" * 1024
        _write_source(vault.source_dir, "corrupt_seq.bin", original)
        fs.add_rule(FaultRule(
            path="/corrupt_seq.bin",
            offset=0,
            action="corrupt",
            corrupt_sequence=[b"AAAA", b"BBBB"],
        ))

        first = _read_mounted(fuse_mount, "corrupt_seq.bin")
        second = _read_mounted(fuse_mount, "corrupt_seq.bin")
        third = _read_mounted(fuse_mount, "corrupt_seq.bin")

        assert first[:4] == b"AAAA"
        assert second[:4] == b"BBBB"
        # 触发次数超出序列长度后固定使用最后一个元素
        assert third[:4] == b"BBBB"
        # 序列覆盖范围之外的字节未被修改
        assert first[4:] == original[4:]
        assert second[4:] == original[4:]
        assert third[4:] == original[4:]


class TestRuleManagement:
    """INFRA-2: 运行时规则管理"""

    def test_clear_rules_restores_data(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """clear_rules 后读取恢复正常数据"""
        vault, fuse_mount, fs = vault_with_fuse_source

        original = b"D" * 2048
        _write_source(vault.source_dir, "clear_rules.bin", original)
        fs.add_rule(FaultRule(
            path="/clear_rules.bin",
            offset=0,
            action="corrupt",
            corrupt_data=b"XXXX",
        ))

        corrupted = _read_mounted(fuse_mount, "clear_rules.bin")
        assert corrupted[:4] == b"XXXX"

        fs.clear_rules()
        restored = _read_mounted(fuse_mount, "clear_rules.bin")
        assert restored == original

    def test_disable_enable_rule(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """disable_rule 后规则不再触发，enable_rule 后恢复触发"""
        vault, fuse_mount, fs = vault_with_fuse_source

        original = b"E" * 1024
        _write_source(vault.source_dir, "toggle_rule.bin", original)
        fs.add_rule(FaultRule(
            path="/toggle_rule.bin",
            offset=0,
            action="corrupt",
            corrupt_data=b"YYYY",
        ))

        fs.disable_rule(0)
        assert _read_mounted(fuse_mount, "toggle_rule.bin") == original

        fs.enable_rule(0)
        corrupted = _read_mounted(fuse_mount, "toggle_rule.bin")
        assert corrupted[:4] == b"YYYY"


class TestPreReadRegression:
    """回归：既有 error/delay 行为不受影响"""

    def test_error_and_delay_regression(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """error(EIO) 规则仍抛出错误，delay 规则仍增加读取耗时"""
        vault, fuse_mount, fs = vault_with_fuse_source

        # error 规则：EIO
        original_err = b"F" * 1024
        _write_source(vault.source_dir, "reg_error.bin", original_err)
        fs.add_rule(FaultRule(
            path="/reg_error.bin",
            offset=0,
            action="error",
            error_code=errno.EIO,
        ))
        with pytest.raises(OSError) as exc_info:
            _read_mounted(fuse_mount, "reg_error.bin")
        assert exc_info.value.errno == errno.EIO
        assert fs.get_stats().error_count >= 1

        # delay 规则：读取耗时显著增加（不同文件名规避页缓存）
        original_delay = b"G" * 1024
        _write_source(vault.source_dir, "reg_delay.bin", original_delay)
        fs.add_rule(FaultRule(
            path="/reg_delay.bin",
            offset=0,
            action="delay",
            delay_ms=500,
        ))
        start = time.monotonic()
        data = _read_mounted(fuse_mount, "reg_delay.bin")
        elapsed = time.monotonic() - start

        assert data == original_delay
        assert elapsed >= 0.4
