"""损坏场景 FUSE 测试 - 模拟硬件故障和静默损坏

本文件使用 FUSE 模拟以下场景：
- 硬盘坏道（特定偏移量返回错误数据）
- 静默数据损坏（随机位翻转）
- 不稳定读取（多次读取返回不同数据）
- "Fundamental Problem"：哈希基于损坏数据计算的不可检测性

这些测试需要 FUSE 支持，因为它们需要精确控制内核返回的数据。
"""

from __future__ import annotations

import errno
import hashlib
import json
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest

# fuse_tests 是包（含 __init__.py），需手动把本目录加入 sys.path
sys.path.insert(0, str(Path(__file__).parent))

from conftest import VaultEnv, create_minimal_jpeg
from fault_inject_fs import FaultInjectedFS, FaultRule

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


def _latest_recheck_report(vault: VaultEnv) -> dict:
    """读取最新的 recheck 报告（JSON）"""
    staging = vault.vault_dir / ".svault" / "staging"
    reports = sorted(staging.glob("recheck_*.json"))
    assert reports, "未找到 recheck 报告"
    return json.loads(reports[-1].read_text())


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
        """局限性确认（§8.1 P0-5）：导入时源损坏 → verify 不可检出，recheck 可检出

        场景与判据（failure-handling.md §4 检测能力边界）：
        1. FUSE 在 offset=1024 注入 16 字节 0x00（模拟坏道返回错误数据）
        2. import 读取损坏数据 → 哈希基于损坏数据入库（H_bad），exit 0
        3. verify：vault 副本哈希 == DB H_bad → **exit 0（无法检测，这是被锁定的局限）**
        4. 清除故障 + 失效页缓存后 recheck：源为原始数据 ≠ manifest H_bad
           → 报告 SourceModified（recheck 是唯一兜底手段）
        """
        vault, fuse_mount, fs = vault_with_fuse_source
        victim = _create_padded_jpeg(vault.source_dir, "victim.jpg", 8 * 1024)
        original = victim.read_bytes()

        fs.add_rule(
            FaultRule(
                path="/victim.jpg",
                offset=1024,
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

        # 4. 清除故障并失效页缓存（重写真实源文件使缓存页作废），
        #    recheck 重读到的将是原始数据
        fs.clear_rules()
        victim.write_bytes(original)

        recheck = vault.run("recheck", str(fuse_mount), check=False)
        assert recheck.returncode == 0, (
            f"recheck 恒 exit 0（不一致只写报告）: {recheck.stderr}"
        )

        report = _latest_recheck_report(vault)
        victim_entries = [
            f for f in report["files"] if Path(f["src_path"]).name == "victim.jpg"
        ]
        assert len(victim_entries) == 1, "报告应包含 victim.jpg"
        assert victim_entries[0]["status"] == "SourceModified", (
            f"源（原始数据）应与 manifest 的 H_bad 不符，实际: {victim_entries[0]['status']}"
        )

    def test_bad_sector_during_import(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """导入过程中遇到坏道

        场景：
        1. FUSE 在特定偏移返回 EIO（模拟坏道）
        2. svault import 读取时遇到错误
        3. 验证：错误被报告，部分文件不导入

        与静默损坏不同，这里显式返回错误。
        """
        pytest.skip("待实现（P1）")
    
    def test_silent_corruption_at_specific_offset(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """局限性确认（§8.2 P1）：单字节静默位翻转同样不可被 verify 检出

        与 P0-5 互补：走 corrupt 的默认 XOR 0xFF 位翻转路径（无 corrupt_data）。
        判据（failure-handling.md §4）：
        1. import 读取被翻转的数据 → H_bad 入库，exit 0
        2. verify exit 0（无法检测，被锁定的局限）
        3. 清除故障 + 失效页缓存后 recheck → SourceModified
        """
        vault, fuse_mount, fs = vault_with_fuse_source
        victim = _create_padded_jpeg(vault.source_dir, "silent.jpg", 8 * 1024)
        original = victim.read_bytes()

        # 默认行为：offset=8 处单字节 XOR 0xFF（静默位翻转）
        fs.add_rule(
            FaultRule(path="/silent.jpg", offset=8, action="corrupt")
        )

        result = vault.import_dir(fuse_mount)
        assert result.returncode == 0, f"导入应完成: {result.stderr}"
        assert len(vault.db_files()) == 1

        vault_files = _vault_data_files(vault)
        assert len(vault_files) == 1
        assert vault_files[0].read_bytes() != original, "vault 副本应含被翻转的字节"

        verify = vault.run("verify", check=False)
        assert verify.returncode == 0, (
            f"局限性确认：verify 对静默损坏不可检出，应 exit 0: {verify.stderr}"
        )

        fs.clear_rules()
        victim.write_bytes(original)  # 失效页缓存，recheck 读到原始数据

        recheck = vault.run("recheck", str(fuse_mount), check=False)
        assert recheck.returncode == 0

        report = _latest_recheck_report(vault)
        entries = [f for f in report["files"] if Path(f["src_path"]).name == "silent.jpg"]
        assert len(entries) == 1
        assert entries[0]["status"] == "SourceModified", (
            f"recheck 应检出源与 H_bad 不符，实际: {entries[0]['status']}"
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
        3. 清除故障 + 失效页缓存后 recheck → SourceModified（唯一兜底）
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

        recheck = vault.run("recheck", str(fuse_mount), check=False)
        assert recheck.returncode == 0

        report = _latest_recheck_report(vault)
        entries = [f for f in report["files"] if Path(f["src_path"]).name == "unstable.jpg"]
        assert len(entries) == 1
        assert entries[0]["status"] == "SourceModified", (
            f"recheck 应检出源与 H(B) 不符，实际: {entries[0]['status']}"
        )
    
    def test_bit_rot_detection(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """位翻转检测（Bit Rot）
        
        场景：
        1. 正常导入文件
        2. 时间推移（模拟），FUSE 返回略微不同的数据（1 bit 翻转）
        3. recheck/verify 应检测到哈希不匹配
        
        验证 svault 能检测到随时间推移的数据衰减。
        """
        pytest.skip("待实现")


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
        pytest.skip("待实现")
    
    def test_intermittent_corruption(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """间歇性损坏
        
        场景：
        1. FUSE 配置：随机 1% 概率返回损坏数据
        2. 大量文件导入
        3. 验证：损坏被检测到并报告
        """
        pytest.skip("待实现")


class TestCrossDeviceVerification:
    """跨设备验证
    
    验证数据在不同存储设备间的一致性。
    """
    
    def test_verify_across_different_storage(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """跨不同存储设备的验证
        
        场景：
        1. 源在 FUSE 挂载点（模拟慢/不可靠存储）
        2. vault 在常规存储
        3. FUSE 配置延迟和偶尔错误
        4. 验证导入仍能完成（带重试）
        """
        pytest.skip("待实现")


class TestDetectionStrategies:
    """损坏检测策略验证
    
    验证各种检测策略的有效性。
    """
    
    def test_post_import_source_recheck_detects_corruption(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """导入后重新检查源文件能发现损坏
        
        解决方案验证：
        1. FUSE 第一次读取（导入）：返回正常数据
        2. FUSE 后续读取（recheck）：返回损坏数据
        3. recheck --source 对比发现不匹配
        4. 报告潜在损坏
        
        这说明为什么需要导入后源验证。
        """
        pytest.skip("待实现")
    
    def test_parity_verification_detects_corruption(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """奇偶校验检测损坏（如果 svault 支持）
        
        如果 svault 实现了奇偶校验或 ECC：
        1. FUSE 注入单 bit 错误
        2. 奇偶校验应能检测并纠正
        """
        pytest.skip("待实现：需要 svault 支持 parity")
    
    def test_multiple_hash_algorithms_detect_corruption(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """多种哈希算法提高检测率
        
        某些损坏可能逃过一种哈希但被另一种捕获。
        验证使用多种哈希（CRC32C + XXH3 + SHA256）提高检测率。
        """
        pytest.skip("待实现")


class TestRealWorldScenarios:
    """真实世界场景模拟"""
    
    def test_aging_hard_drive_simulation(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """老化硬盘模拟
        
        模拟老化硬盘的行为：
        - 读取延迟逐渐增加
        - 偶尔返回错误（需要重试）
        - 特定区域（老化区域）返回损坏数据
        
        验证 svault 能优雅处理并在可能时恢复。
        """
        pytest.skip("待实现")
    
    def test_network_storage_interruption(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """网络存储中断
        
        模拟 NFS/SMB 中断：
        - 读取时返回 EIO
        - 超时后恢复
        - 验证重试和恢复机制
        """
        pytest.skip("待实现")
