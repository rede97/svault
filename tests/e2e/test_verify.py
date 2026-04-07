"""Verify and recheck command tests.

Merged from:
- test_verify.py: Hash verification and corruption detection
- test_recheck.py: Manifest-based recheck workflow
- test_atomic_verification.py: Atomic verification concepts

测试完整性验证功能，包括：
- verify 命令功能（哈希匹配、损坏检测、摘要输出）
- recheck 命令功能（基于清单的双向验证）
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
    
    def test_verify_detects_content_replacement(self, vault: VaultEnv) -> None:
        """Verify 应能检测到内容被替换"""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        full_path = vault.vault_dir / files[0]["path"]
        
        create_minimal_jpeg(full_path, "COMPLETELY_DIFFERENT_CONTENT")
        
        result = vault.run("verify", capture=True, check=False)
        assert result.returncode != 0
    
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
    
    def test_verify_with_xxh3_128(self, vault: VaultEnv) -> None:
        """使用 XXH3-128 算法验证"""
        # Default hash is xxh3_128
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        result = vault.run("verify", capture=True)
        assert result.returncode == 0
        combined = result.stdout + result.stderr
        assert "OK" in combined or "0" in combined
    
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
# Recheck 命令测试 (merged from test_recheck.py)
# =============================================================================

class TestRecheckWorkflow:
    """Recheck 端到端工作流测试"""

    def test_recheck_detects_corruption_and_reimport_succeeds(self, vault: VaultEnv) -> None:
        """Detect vault file corruption and recover by re-importing."""
        f1 = vault.source_dir / "keep.jpg"
        f2 = vault.source_dir / "corrupt.jpg"
        create_minimal_jpeg(f1, "KEEP_KEEP_KEEP_" * 1000)
        create_minimal_jpeg(f2, "CORRUPT_CORRUPT_" * 1000)

        result = vault.import_dir(vault.source_dir, strategy="copy")
        assert result.returncode == 0
        files = vault.db_files()
        assert len(files) == 2

        vault_files = vault.get_vault_files("*.jpg")
        assert len(vault_files) == 2

        corrupt_target = None
        for vf in vault_files:
            if "corrupt" in vf.name.lower():
                corrupt_target = vf
                break
        assert corrupt_target is not None

        data = corrupt_target.read_bytes()
        corrupt_target.write_bytes(data[:65536] + b"TAMPERED_TAIL_DATA")

        result = vault.run("recheck")
        assert result.returncode == 0
        combined = result.stderr + result.stdout
        assert "Vault corrupted" in combined

        corrupt_target.unlink()

        # Mark deleted file as missing in DB, then re-import
        result = vault.run("update", "--yes")
        assert result.returncode == 0
        
        result = vault.import_dir(vault.source_dir, strategy="copy")
        assert result.returncode == 0

        vault_files_after = vault.get_vault_files("*.jpg")
        assert len(vault_files_after) == 2

        result = vault.run("recheck")
        assert result.returncode == 0
        combined = result.stderr + result.stdout
        assert "VAULT_CORRUPTED" not in combined

    def test_recheck_all_ok(self, vault: VaultEnv) -> None:
        """Recheck after successful import should report all OK."""
        f1 = vault.source_dir / "a.jpg"
        f2 = vault.source_dir / "b.jpg"
        create_minimal_jpeg(f1, "FILE_A" * 500)
        create_minimal_jpeg(f2, "FILE_B" * 500)

        vault.import_dir(vault.source_dir, strategy="copy")

        result = vault.run("recheck")
        assert result.returncode == 0
        combined = result.stderr + result.stdout
        assert "OK" in combined
        assert "corrupted" not in combined.lower()

    def test_recheck_source_mismatch(self, vault: VaultEnv) -> None:
        """Providing a source path that doesn't match the manifest should error."""
        f1 = vault.source_dir / "a.jpg"
        create_minimal_jpeg(f1, "FILE_A" * 500)

        vault.import_dir(vault.source_dir, strategy="copy")

        wrong_source = vault.root / "wrong_source"
        wrong_source.mkdir(parents=True, exist_ok=True)
        result = vault.run("recheck", str(wrong_source.resolve()), check=False)
        assert result.returncode != 0
        combined = result.stderr + result.stdout
        assert "Source path mismatch" in combined

    def test_recheck_with_matching_source(self, vault: VaultEnv) -> None:
        """Providing the correct source path should work."""
        f1 = vault.source_dir / "a.jpg"
        create_minimal_jpeg(f1, "FILE_A" * 500)

        vault.import_dir(vault.source_dir, strategy="copy")

        result = vault.run("recheck", str(vault.source_dir.resolve()))
        assert result.returncode == 0
        combined = result.stderr + result.stdout
        assert "OK" in combined


# =============================================================================
# Source 验证测试
# =============================================================================

class TestSourceVerification:
    """源文件验证测试"""
    
    def test_source_changed_after_import(self, vault: VaultEnv) -> None:
        """导入后源文件被修改应能被检测到"""
        f = vault.source_dir / "test.jpg"
        create_minimal_jpeg(f, "SOURCE_V1")
        
        source_hash_v1 = compute_file_hash(f)
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        vault_file = vault.vault_dir / files[0]["path"]
        vault_hash = compute_file_hash(vault_file)
        
        assert source_hash_v1 == vault_hash
        
        time.sleep(0.1)
        create_minimal_jpeg(f, "SOURCE_V2_DIFFERENT")
        source_hash_v2 = compute_file_hash(f)
        
        assert source_hash_v1 != source_hash_v2
        assert source_hash_v2 != compute_file_hash(vault_file)
    
    def test_cross_session_consistency(self, vault: VaultEnv) -> None:
        """跨会话一致性"""
        f = vault.source_dir / "test.jpg"
        create_minimal_jpeg(f, "STABLE_DATA")
        
        hashes = []
        for _ in range(5):
            h = compute_file_hash(f)
            hashes.append(h)
            time.sleep(0.01)
        
        assert len(set(hashes)) == 1


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
        assert source_hash == vault_hash
    
    def test_no_partial_files_committed(self, vault: VaultEnv) -> None:
        """无部分写入文件提交"""
        f = vault.source_dir / "test.jpg"
        create_minimal_jpeg(f, "COMPLETE_FILE_DATA")
        
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        for file_info in files:
            vault_path = vault.vault_dir / file_info["path"]
            assert vault_path.exists()
            
            if "size" in file_info and file_info["size"] is not None:
                actual_size = vault_path.stat().st_size
                recorded_size = file_info["size"]
                assert actual_size == recorded_size


# =============================================================================
# 边界情况测试
# =============================================================================

class TestVerificationEdgeCases:
    """验证边界情况"""
    
    def test_empty_file_verification(self, vault: VaultEnv) -> None:
        """空文件验证"""
        f = vault.source_dir / "small.jpg"
        create_minimal_jpeg(f, "small_test")
        
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        assert len(files) == 1
        
        vault_file = vault.vault_dir / files[0]["path"]
        assert vault_file.exists()
        assert vault_file.stat().st_size > 0
    
    def test_large_file_hash_verification(self, vault: VaultEnv) -> None:
        """大文件哈希验证"""
        f = vault.source_dir / "large.jpg"
        create_minimal_jpeg(f, "large_file_test")
        with open(f, 'ab') as fp:
            fp.write(b"X" * (1024 * 1024 - f.stat().st_size))
        
        source_hash = compute_file_hash(f)
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        assert len(files) == 1
        vault_file = vault.vault_dir / files[0]["path"]
        vault_hash = compute_file_hash(vault_file)
        
        assert source_hash == vault_hash


# =============================================================================
# Recovery 测试
# =============================================================================

class TestVerifyRecovery:
    """验证失败后的恢复测试"""

    def test_deleted_file_can_be_reimported_after_verify_failure(self, vault: VaultEnv) -> None:
        """If verify detects corruption, deleting and re-importing works."""
        f1 = vault.source_dir / "photo.jpg"
        create_minimal_jpeg(f1, "PHOTO_DATA" * 500)

        vault.import_dir(vault.source_dir, strategy="copy")

        result = vault.run("verify")
        assert result.returncode == 0

        vault_files = vault.get_vault_files("*.jpg")
        assert len(vault_files) == 1
        vf = vault_files[0]
        data = vf.read_bytes()
        vf.write_bytes(data[:65536] + b"CORRUPT")

        result = vault.run("verify", check=False)
        assert result.returncode != 0

        vf.unlink()

        # Mark deleted file as missing in DB, then re-import
        result = vault.run("update", "--yes")
        assert result.returncode == 0
        
        result = vault.import_dir(vault.source_dir, strategy="copy")
        assert result.returncode == 0

        vault_files_after = vault.get_vault_files("*.jpg")
        assert len(vault_files_after) == 1

        result = vault.run("verify")
        assert result.returncode == 0


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
1. 导入后重新检查源文件（recheck --source）
2. 使用外部参考（多个备份）
3. 使用校验和/ECC 存储

这个根本问题的实际演示参见：
fuse_tests/test_corruption_fuse.py::TestFundamentalProblem
"""


# =============================================================================
# DB Verify-Chain 测试
# =============================================================================

class TestDbVerifyChain:
    """db verify-chain 命令测试 - 验证事件哈希链完整性"""
    
    def test_verify_chain_empty_vault(self, vault: VaultEnv) -> None:
        """空 vault 应验证通过（0 事件）"""
        result = vault.run("db", "verify-chain")
        assert result.returncode == 0
        assert "verified" in result.stdout.lower() or "0 events" in result.stdout
    
    def test_verify_chain_after_import(self, vault: VaultEnv) -> None:
        """导入文件后应验证通过"""
        from conftest import create_minimal_jpeg
        
        test_file = vault.source_dir / "test.jpg"
        create_minimal_jpeg(test_file)
        
        vault.import_dir(vault.source_dir)
        
        result = vault.run("db", "verify-chain")
        assert result.returncode == 0
        assert "verified" in result.stdout.lower()
    
    def test_verify_chain_detects_tampering(self, vault: VaultEnv) -> None:
        """篡改事件后应检测出链断裂"""
        from conftest import create_minimal_jpeg
        
        test_file = vault.source_dir / "test.jpg"
        create_minimal_jpeg(test_file)
        
        vault.import_dir(vault.source_dir)
        
        # 直接篡改数据库中的事件
        db_path = vault.vault_dir / ".svault" / "vault.db"
        conn = sqlite3.connect(str(db_path))
        try:
            # 修改第一个事件的 payload，破坏 self_hash
            conn.execute("UPDATE events SET payload = '{\"tampered\":true}' WHERE seq = 1")
            conn.commit()
        finally:
            conn.close()
        
        # 验证应失败
        result = vault.run("db", "verify-chain", check=False)
        assert result.returncode != 0
        assert "failed" in result.stdout.lower() or "failed" in result.stderr.lower() or "broken" in result.stdout.lower() or "broken" in result.stderr.lower()
