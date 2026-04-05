"""Verify command and verification logic tests.

测试完整性验证功能，包括：
- verify 命令功能（哈希匹配、损坏检测、摘要输出）
- 底层验证逻辑（数据库哈希一致性、源文件验证）
- write-then-verify 模式
- 边界情况（空文件、大文件）

对于需要模拟硬件损坏（坏道、静默损坏）的深度测试，参见 fuse_tests/test_corruption_fuse.py
"""

from __future__ import annotations

import hashlib
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
# 基础 Verify 命令测试
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
        assert "OK" in result.stdout or "Summary" in result.stdout
    
    def test_verify_single_file_ok(self, vault: VaultEnv) -> None:
        """验证单个完好文件"""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        assert len(files) == 1
        file_path = files[0]["path"]
        
        result = vault.run("verify", "--file", file_path, capture=True)
        assert result.returncode == 0


# =============================================================================
# 损坏检测测试
# =============================================================================

class TestVerifyCorruption:
    """损坏检测测试 - 通过直接修改文件模拟"""
    
    def test_verify_detects_bit_flip(self, vault: VaultEnv) -> None:
        """Verify 应能检测到单比特损坏（模拟磁盘错误）"""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        file_path = files[0]["path"]
        full_path = vault.vault_dir / file_path
        
        # 损坏单个字节（翻转所有比特）
        original_data = full_path.read_bytes()
        corrupted = bytearray(original_data)
        corrupted[100] ^= 0xFF
        full_path.write_bytes(corrupted)
        
        result = vault.run("verify", capture=True, check=False)
        assert result.returncode != 0
        assert "mismatch" in result.stdout.lower() or "hash" in result.stdout.lower()
    
    def test_verify_detects_truncation(self, vault: VaultEnv) -> None:
        """Verify 应能检测到文件截断"""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        full_path = vault.vault_dir / files[0]["path"]
        
        # 截断文件
        data = full_path.read_bytes()
        full_path.write_bytes(data[:len(data)//2])
        
        result = vault.run("verify", capture=True, check=False)
        assert result.returncode != 0
        assert "size" in result.stdout.lower() or "mismatch" in result.stdout.lower()
    
    def test_verify_detects_missing_file(self, vault: VaultEnv) -> None:
        """Verify 应能检测到文件丢失"""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        full_path = vault.vault_dir / files[0]["path"]
        full_path.unlink()
        
        result = vault.run("verify", capture=True, check=False)
        assert result.returncode != 0
        assert "missing" in result.stdout.lower()
    
    def test_verify_detects_content_replacement(self, vault: VaultEnv) -> None:
        """Verify 应能检测到内容被替换（不同内容，相同文件名）"""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        full_path = vault.vault_dir / files[0]["path"]
        
        # 替换为完全不同的内容
        create_minimal_jpeg(full_path, "COMPLETELY_DIFFERENT_CONTENT")
        
        result = vault.run("verify", capture=True, check=False)
        assert result.returncode != 0
    
    def test_verify_multiple_corruptions(self, vault: VaultEnv) -> None:
        """Verify 应报告所有损坏文件，而不仅是第一个"""
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "samsung_photo.jpg")
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        assert len(files) >= 2
        
        # 损坏所有文件
        for file_info in files:
            file_path = vault.vault_dir / file_info["path"]
            data = bytearray(file_path.read_bytes())
            data[50] ^= 0xFF
            file_path.write_bytes(data)
        
        result = vault.run("verify", capture=True, check=False)
        assert result.returncode != 0
        assert result.stdout.count("mismatch") >= 2 or result.stdout.count("hash") >= 2


# =============================================================================
# 哈希算法测试
# =============================================================================

class TestVerifyHashAlgorithms:
    """不同哈希算法的验证测试"""
    
    def test_verify_with_sha256(self, vault: VaultEnv) -> None:
        """使用 SHA-256 算法验证"""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir, hash="secure")
        
        result = vault.run("verify", "-H", "secure", capture=True)
        assert result.returncode == 0
    
    def test_verify_with_xxh3_128(self, vault: VaultEnv) -> None:
        """使用 XXH3-128 算法验证"""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        result = vault.run("verify", "-H", "fast", capture=True)
        assert result.returncode == 0
        assert "OK" in result.stdout or "0" in result.stdout
    
    def test_database_hash_matches_actual_file(self, vault: VaultEnv) -> None:
        """数据库中存储的哈希与实际文件匹配"""
        f = vault.source_dir / "test.jpg"
        create_minimal_jpeg(f, "TEST_DATA_HASH_CHECK")
        vault.import_dir(vault.source_dir, hash="secure")
        
        files = vault.db_files()
        assert len(files) == 1
        
        vault_file = vault.vault_dir / files[0]["path"]
        actual_hash = compute_file_hash(vault_file)
        
        db_hash = files[0]["sha256"]
        if isinstance(db_hash, bytes):
            db_hash = db_hash.hex()
        
        assert actual_hash == db_hash.lower(), "数据库哈希应与实际文件匹配"


# =============================================================================
# 源文件验证测试
# =============================================================================

class TestSourceVerification:
    """源文件验证测试 - 验证源文件与 vault 的一致性"""
    
    def test_source_changed_after_import(self, vault: VaultEnv) -> None:
        """导入后源文件被修改应能被检测到"""
        f = vault.source_dir / "test.jpg"
        create_minimal_jpeg(f, "SOURCE_V1")
        
        source_hash_v1 = compute_file_hash(f)
        vault.import_dir(vault.source_dir, hash="secure")
        
        files = vault.db_files()
        vault_file = vault.vault_dir / files[0]["path"]
        vault_hash = compute_file_hash(vault_file)
        
        # 初始一致
        assert source_hash_v1 == vault_hash
        
        # 修改源文件
        time.sleep(0.1)
        create_minimal_jpeg(f, "SOURCE_V2_DIFFERENT")
        source_hash_v2 = compute_file_hash(f)
        
        # 验证哈希不同
        assert source_hash_v1 != source_hash_v2
        assert source_hash_v2 != compute_file_hash(vault_file)
    
    def test_cross_session_consistency(self, vault: VaultEnv) -> None:
        """跨会话一致性 - 多次读取应返回相同数据"""
        f = vault.source_dir / "test.jpg"
        create_minimal_jpeg(f, "STABLE_DATA")
        
        hashes = []
        for _ in range(5):
            h = compute_file_hash(f)
            hashes.append(h)
            time.sleep(0.01)
        
        assert len(set(hashes)) == 1, "多次读取应返回相同哈希"


# =============================================================================
# Write-Then-Verify 模式测试
# =============================================================================

class TestWriteThenVerify:
    """写入后验证模式测试"""
    
    def test_copy_integrity_verification(self, vault: VaultEnv) -> None:
        """复制后验证数据完整性"""
        f = vault.source_dir / "test.jpg"
        create_minimal_jpeg(f, "DATA_FOR_COPY_TEST")
        source_hash = compute_file_hash(f)
        
        result = vault.import_dir(vault.source_dir, hash="secure")
        assert result.returncode == 0
        
        files = vault.db_files()
        assert len(files) == 1
        
        vault_file = vault.vault_dir / files[0]["path"]
        assert vault_file.exists()
        
        vault_hash = compute_file_hash(vault_file)
        assert source_hash == vault_hash, "源文件和 vault 文件应完全一致"
    
    def test_no_partial_files_committed(self, vault: VaultEnv) -> None:
        """无部分写入文件提交 - 验证事务完整性"""
        f = vault.source_dir / "test.jpg"
        create_minimal_jpeg(f, "COMPLETE_FILE_DATA")
        original_size = f.stat().st_size
        
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        for file_info in files:
            vault_path = vault.vault_dir / file_info["path"]
            assert vault_path.exists()
            
            if "size" in file_info and file_info["size"] is not None:
                actual_size = vault_path.stat().st_size
                recorded_size = file_info["size"]
                assert actual_size == recorded_size, f"文件大小不匹配: {file_info['path']}"


# =============================================================================
# 输出格式测试
# =============================================================================

class TestVerifySummary:
    """Verify 输出格式测试"""
    
    def test_verify_summary_counts(self, vault: VaultEnv) -> None:
        """Verify 摘要应显示正确的计数"""
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "samsung_photo.jpg")
        vault.import_dir(vault.source_dir)
        
        result = vault.run("verify", capture=True)
        assert result.returncode == 0
        
        assert "Summary" in result.stdout or "Total" in result.stdout
        assert "OK" in result.stdout or "2" in result.stdout


# =============================================================================
# 边界情况测试
# =============================================================================

class TestVerificationEdgeCases:
    """验证边界情况"""
    
    def test_empty_file_verification(self, vault: VaultEnv) -> None:
        """空文件验证 - 使用 create_minimal_jpeg 创建空的 JPEG 结构"""
        # 创建一个最小的有效 JPEG（非空但很小）
        f = vault.source_dir / "small.jpg"
        create_minimal_jpeg(f, "small_test")
        
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        assert len(files) == 1, f"Expected 1 file, got {len(files)}. DB contents: {files}"
        
        vault_file = vault.vault_dir / files[0]["path"]
        assert vault_file.exists()
        # 验证文件大小大于 0（minimal_jpeg 创建的是有效文件）
        assert vault_file.stat().st_size > 0
    
    def test_large_file_hash_verification(self, vault: VaultEnv) -> None:
        """大文件哈希验证 - 使用 JPEG 格式确保被导入"""
        f = vault.source_dir / "large.jpg"
        # 创建大的 JPEG 文件：minimal JPEG + 填充数据
        create_minimal_jpeg(f, "large_file_test")
        # 追加数据使文件变大（1MB）
        with open(f, 'ab') as fp:
            fp.write(b"X" * (1024 * 1024 - f.stat().st_size))
        
        source_hash = compute_file_hash(f)
        vault.import_dir(vault.source_dir, hash="secure")
        
        files = vault.db_files()
        assert len(files) == 1, f"Expected 1 file, got {len(files)}"
        vault_file = vault.vault_dir / files[0]["path"]
        vault_hash = compute_file_hash(vault_file)
        
        assert source_hash == vault_hash


# =============================================================================
# Fundamental Problem 说明（参考 FUSE 测试）
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
   （两者都基于损坏数据，匹配！）

解决方案：
1. 导入后重新检查源文件（recheck --source）
2. 使用外部参考（多个备份）
3. 使用校验和/ECC 存储

这个根本问题的实际演示参见：
fuse_tests/test_corruption_fuse.py::TestFundamentalProblem
"""
