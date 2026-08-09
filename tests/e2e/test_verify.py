"""Verify command tests.

Merged from:
- test_verify.py: Hash verification and corruption detection
- test_atomic_verification.py: Atomic verification concepts

测试完整性验证功能，包括：
- verify 命令功能（哈希匹配、损坏检测、摘要输出）
- 底层验证逻辑（数据库哈希一致性）
- write-then-verify 模式
- 边界情况（空文件、大文件）

对于需要模拟硬件损坏（坏道、静默损坏）的深度测试，参见 fuse_tests/test_corruption_fuse.py
"""

from __future__ import annotations

import hashlib
import json
import sqlite3
import time
from pathlib import Path

import pytest

from conftest import VaultEnv, copy_fixture, create_minimal_jpeg


def compute_file_hash(path: Path) -> str:
    """计算文件的 SHA-256 哈希"""
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        while chunk := f.read(8192):
            h.update(chunk)
    return h.hexdigest()


# =============================================================================
# Verify 命令测试
# =============================================================================

class TestVerifyBasic:
    """基础 verify 命令测试"""
    
    def test_verify_all_ok(self, vault: VaultEnv) -> None:
        """所有文件完好的情况应通过验证"""
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "samsung_photo.jpg")
        vault.import_dir(vault.source_dir)
        
        result = vault.run("verify", capture=True)
        assert result.returncode == 0
        combined = result.stdout + result.stderr
        assert "OK" in combined or "Summary" in combined
    
    def test_verify_single_file_ok(self, vault: VaultEnv) -> None:
        """验证单个完好文件"""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        assert len(files) == 1
        file_path = files[0]["path"]
        
        result = vault.run("verify", "--file", file_path, capture=True)
        assert result.returncode == 0
        combined = result.stdout + result.stderr
        assert "✓" in combined or "OK" in combined


class TestVerifyCorruption:
    """损坏检测测试"""
    
    def test_verify_detects_bit_flip(self, vault: VaultEnv) -> None:
        """Verify 应能检测到单比特损坏"""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        file_path = files[0]["path"]
        full_path = vault.vault_dir / file_path
        
        original_data = full_path.read_bytes()
        corrupted = bytearray(original_data)
        corrupted[100] ^= 0xFF
        full_path.write_bytes(corrupted)
        
        result = vault.run("verify", capture=True, check=False)
        assert result.returncode != 0
        combined = result.stdout + result.stderr
        assert "mismatch" in combined.lower() or "hash" in combined.lower()
    
    def test_verify_detects_truncation(self, vault: VaultEnv) -> None:
        """Verify 应能检测到文件截断"""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        full_path = vault.vault_dir / files[0]["path"]
        
        data = full_path.read_bytes()
        full_path.write_bytes(data[:len(data)//2])
        
        result = vault.run("verify", capture=True, check=False)
        assert result.returncode != 0
        combined = result.stdout + result.stderr
        assert "size" in combined.lower() or "mismatch" in combined.lower()
    
    def test_verify_detects_missing_file(self, vault: VaultEnv) -> None:
        """Verify 应能检测到文件丢失"""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        full_path = vault.vault_dir / files[0]["path"]
        full_path.unlink()
        
        result = vault.run("verify", capture=True, check=False)
        assert result.returncode != 0
        combined = result.stdout + result.stderr
        assert "missing" in combined.lower()
    
    def test_verify_multiple_corruptions(self, vault: VaultEnv) -> None:
        """Verify 应报告所有损坏文件"""
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "samsung_photo.jpg")
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        assert len(files) >= 2
        
        for file_info in files:
            file_path = vault.vault_dir / file_info["path"]
            data = bytearray(file_path.read_bytes())
            data[50] ^= 0xFF
            file_path.write_bytes(data)
        
        result = vault.run("verify", capture=True, check=False)
        assert result.returncode != 0
        combined = result.stdout + result.stderr
        assert combined.count("mismatch") >= 2 or combined.count("hash") >= 2


class TestVerifyHashAlgorithms:
    """不同哈希算法的验证测试"""
    
    def test_verify_with_sha256(self, vault: VaultEnv) -> None:
        """使用 SHA-256 算法验证"""
        # Configure vault to use sha256
        vault.set_hash_algorithm("sha256")
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        result = vault.run("verify", capture=True)
        assert result.returncode == 0
    
    def test_database_hash_matches_actual_file(self, vault: VaultEnv) -> None:
        """数据库中存储的哈希与实际文件匹配"""
        # Use --full-id to compute SHA-256 for this test
        f = vault.source_dir / "test.jpg"
        create_minimal_jpeg(f, "TEST_DATA_HASH_CHECK")
        vault.import_dir(vault.source_dir, full_id=True)
        
        files = vault.db_files()
        assert len(files) == 1
        
        vault_file = vault.vault_dir / files[0]["path"]
        actual_hash = compute_file_hash(vault_file)
        
        db_hash = files[0]["sha256"]
        if isinstance(db_hash, bytes):
            db_hash = db_hash.hex()
        
        assert db_hash is not None, "SHA-256 hash should be computed"
        assert actual_hash == db_hash.lower()


class TestVerifySummary:
    """Verify 输出格式测试"""
    
    def test_verify_summary_counts(self, vault: VaultEnv) -> None:
        """Verify 摘要应显示正确的计数"""
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "samsung_photo.jpg")
        vault.import_dir(vault.source_dir)
        
        result = vault.run("verify", capture=True)
        assert result.returncode == 0
        combined = result.stdout + result.stderr
        assert "Summary" in combined or "Total" in combined
        assert "OK" in combined or "2" in combined


# =============================================================================
# Source 验证测试
# =============================================================================


# =============================================================================
# Write-Then-Verify 模式测试
# =============================================================================


# =============================================================================
# 边界情况测试
# =============================================================================


# =============================================================================
# Recovery 测试
# =============================================================================


# =============================================================================
# 说明
# =============================================================================

"""
【哈希验证的根本限制】

本文件测试的是 "正常情况下的验证"。有一个根本限制无法通过常规测试验证：

问题：如果哈希是基于损坏数据计算的，verify 无法发现！

场景：
1. 源文件在坏道上
2. 读取时得到损坏数据
3. 计算哈希 H_bad（基于损坏数据）
4. 存储 H_bad 到数据库
5. 复制损坏数据到 vault
6. verify 比较：vault_hash == db_hash → PASS！

解决方案：
1. 导入时用 --compare-level mid/high 重新导入一次（源哈希比对）
2. 使用外部参考（多个备份）
3. 使用校验和/ECC 存储

这个根本问题的实际演示参见：
fuse_tests/test_corruption_fuse.py::TestFundamentalProblem
"""


class TestBackgroundHash:
    """background-hash：为快速哈希导入的文件补齐 SHA-256（合并自 test_background_hash.py）。"""

    def test_background_hash_computes_missing_sha256(self, vault: VaultEnv) -> None:
        """background-hash 应为仅有 XXH3-128 的文件计算 SHA-256。"""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        files = vault.db_files()
        assert len(files) == 1
        assert files[0]["sha256"] is None

        result = vault.run("verify", "--background-hash", capture=True)
        assert result.returncode == 0

        files = vault.db_files()
        assert files[0]["sha256"] is not None
        assert len(files[0]["sha256"]) > 0

    def test_background_hash_no_pending_files(self, vault: VaultEnv) -> None:
        """无待补文件时 background-hash 应成功空转。"""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.run("import", "--yes", "--full-id", str(vault.source_dir))

        result = vault.run("verify", "--background-hash", capture=True)
        assert result.returncode == 0

    def test_background_hash_limit(self, vault: VaultEnv) -> None:
        """--background-hash-limit 应限制单次处理数量。"""
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "samsung_photo.jpg")
        vault.import_dir(vault.source_dir)

        pending_before = [f for f in vault.db_files() if f["sha256"] is None]
        assert len(pending_before) == 2

        result = vault.run("verify", "--background-hash", "--background-hash-limit", "1", capture=True)
        assert result.returncode == 0

        pending_after = [f for f in vault.db_files() if f["sha256"] is None]
        assert len(pending_after) == 1
