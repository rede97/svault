"""损坏场景 FUSE 测试 - 模拟硬件故障和静默损坏

本文件使用 FUSE 模拟以下场景：
- 硬盘坏道（特定偏移量返回错误数据）
- 静默数据损坏（随机位翻转）
- 不稳定读取（多次读取返回不同数据）
- "Fundamental Problem"：哈希基于损坏数据计算的不可检测性

这些测试需要 FUSE 支持，因为它们需要精确控制内核返回的数据。
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

# fuse_tests 是包（含 __init__.py），需手动把本目录加入 sys.path
sys.path.insert(0, str(Path(__file__).parent))

from conftest import VaultEnv, create_minimal_jpeg
from fault_inject_fs import FaultRule

pytestmark = [pytest.mark.fuse, pytest.mark.slow, pytest.mark.corruption]


def _create_padded_jpeg(source_dir: Path, name: str, size: int) -> Path:
    """创建填充到指定字节的 JPEG"""
    path = source_dir / name
    create_minimal_jpeg(path, name)
    current = path.stat().st_size
    if current < size:
        with open(path, "ab") as f:
            f.write(b"\xff" * (size - current))
    return path


def _vault_data_files(vault: VaultEnv) -> list[Path]:
    """vault 中的数据文件（排除 svault.toml 等配置文件）"""
    return [f for f in vault.get_vault_files() if f.name != "svault.toml"]



class TestFundamentalProblem:
    """演示哈希验证的根本限制

    核心问题：如果哈希是基于损坏数据计算的，verify 无法发现问题。
    这些测试使用 FUSE 实际演示这个问题。

    注意：本类测试是**局限性确认测试**（failure-handling.md §4）——
    断言的是现行检测能力边界，不是缺陷。
    """

    def test_corrupted_hash_undetectable_by_verify(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """局限性确认（§8.1 P0-5）：导入时源损坏 → verify 与 fast 重跑均不可检出，
        `-c mid` 重跑可检出

        场景与判据（failure-handling.md §4 检测能力边界）：
        1. FUSE 在 CRC 盲区（offset=100KB，头尾各 64KB 之外）注入 16 字节 0x00
        2. import 读取损坏数据 → 哈希基于损坏数据入库（H_bad），exit 0
        3. verify：vault 副本哈希 == DB H_bad → **exit 0（无法检测，锁定局限）**
        4. 清除故障 + 失效页缓存后：fast 重跑 → 指纹命中（盲区）判 duplicate，
           仍不可检出；`-c mid` 重跑 → 源端 XXH3 与 DB H_bad 不符 → 推翻指纹
           判定，按新文件导入（第二条记录 = 检出证据）
        """
        vault, fuse_mount, fs = vault_with_fuse_source
        victim = _create_padded_jpeg(vault.source_dir, "victim.jpg", 200 * 1024)
        original = victim.read_bytes()

        fs.add_rule(
            FaultRule(
                path="/victim.jpg",
                offset=100 * 1024,  # CRC 头尾读取盲区
                action="corrupt",
                # 填充区是 0xFF，用 0x00 载荷保证损坏可区分
                corrupt_data=b"\x00" * 16,
            )
        )

        # 1-2. 导入损坏数据：import 正常完成（无源-目标比对，§4）
        result = vault.import_dir(fuse_mount)
        assert result.returncode == 0, f"导入应完成: {result.stderr}"
        assert len(vault.db_files()) == 1

        # vault 副本确实是损坏数据（与原始源不同）
        vault_files = _vault_data_files(vault)
        assert len(vault_files) == 1
        assert vault_files[0].read_bytes() != original, (
            "vault 副本应包含被注入的损坏数据"
        )

        # 3. verify 无法检出（H_bad == H_bad）——局限性确认，非缺陷
        verify = vault.run("verify", check=False)
        assert verify.returncode == 0, (
            f"局限性确认：verify 对导入期损坏不可检出，应 exit 0: {verify.stderr}"
        )

        # 4. 清除故障并失效页缓存（重写真实源文件使缓存页作废）
        fs.clear_rules()
        victim.write_bytes(original)

        # fast 重跑：损坏在 CRC 盲区 → 指纹命中判 duplicate，不可检出（锁定局限）
        fast = vault.import_dir(fuse_mount)
        assert fast.returncode == 0
        assert len(vault.db_files()) == 1, (
            "fast：盲区损坏不改变指纹，应仍判 duplicate"
        )

        # mid 重跑：源端 XXH3 与 DB H_bad 不符 → 按新文件导入 = 检出
        mid = vault.run(
            "--yes", "import", str(fuse_mount), "-c", "mid", check=False
        )
        assert mid.returncode == 0, f"mid 重跑应完成: {mid.stderr}"
        assert len(vault.db_files()) == 2, (
            "mid：源（原始数据）与 DB H_bad 不符，应推翻指纹判定重新入库"
        )


class TestUnstableStorage:
    """不稳定存储测试
    
    模拟存储介质不稳定，多次读取返回不同数据。
    """
    
    def test_unstable_read_during_import(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """局限性确认（§8.2 P1）：多次读取返回不同数据，导入期无法检测

        机制：corrupt_sequence=[A, B]——首次内容读取（Stage B CRC）得到 A
        （0x00 注入），其后所有读取（EXIF/复制）得到 B（0x11 注入）。
        vault 副本 = B 损坏版，DB 强哈希 = H(B)（Stage D 哈希 vault 副本），
        导入路径无源-目标哈希比对（§4）。

        判据：
        1. import exit 0——不稳定读取**无法检测**（被锁定的局限）
        2. vault 副本含 B 载荷；verify exit 0（自洽）
        3. 清除故障 + 失效页缓存后重跑 import：源（原始数据）CRC 与
           DB 记录的 CRC(A) 不符 → 按新文件导入（第二条记录 = 检出证据）
        """
        vault, fuse_mount, fs = vault_with_fuse_source
        victim = _create_padded_jpeg(vault.source_dir, "unstable.jpg", 8 * 1024)
        original = victim.read_bytes()

        payload_b = b"\x11" * 16
        fs.add_rule(
            FaultRule(
                path="/unstable.jpg",
                offset=1024,
                action="corrupt",
                corrupt_sequence=[b"\x00" * 16, payload_b],
            )
        )

        result = vault.import_dir(fuse_mount)
        assert result.returncode == 0, (
            f"局限性确认：不稳定读取在导入期无法检测，import 应 exit 0: {result.stderr}"
        )
        assert len(vault.db_files()) == 1

        vault_files = _vault_data_files(vault)
        assert len(vault_files) == 1
        content = vault_files[0].read_bytes()
        assert content != original, "vault 副本应与原始源不同"
        assert content[1024:1040] == payload_b, "vault 副本应含序列载荷 B"

        verify = vault.run("verify", check=False)
        assert verify.returncode == 0, (
            f"verify 只校验 vault 副本自洽，应 exit 0: {verify.stderr}"
        )

        fs.clear_rules()
        victim.write_bytes(original)  # 失效页缓存

        # 8KB 文件 CRC 全覆盖：恢复后的源与 DB 的 CRC(A) 不符 → 按新文件导入
        rerun = vault.import_dir(fuse_mount)
        assert rerun.returncode == 0
        assert len(vault.db_files()) == 2, (
            "重跑应检出源变化（CRC 不符），按新文件入库"
        )


class TestCorruptionDuringCopy:
    """复制过程中的损坏
    
    源文件正常，但在复制到 vault 的过程中发生损坏。
    """
    
    def test_corruption_during_copy_to_vault(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """复制到 vault 时损坏
        
        场景：
        1. 源文件正常（直接读取）
        2. FUSE 在 vault 路径的写入操作注入错误
        3. 或：FUSE 在读取源文件时（如果是 FUSE 挂载的源）注入损坏
        4. 验证写入后校验能检测到
        """
        pytest.skip("待实现（P2，vault 侧注入：dm-flakey，见 failure-handling.md §8.4 INFRA-3）")
