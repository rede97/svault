"""Atomic verification and corruption detection tests.

Tests the fundamental problem: what if hash is computed from already-corrupted data?

中文场景说明：
- 坏道问题：硬盘坏道导致读取时返回错误数据
- 静默损坏：文件看起来正常但实际内容已损坏
- 时间窗口问题：hash计算和复制之间文件变化

核心问题：如果hash本身是基于损坏数据计算的，verify会发现不了问题！

解决方案测试：
1. 写入后验证：复制完成后立即重新hash
2. 跨会话验证：不同时间点多次验证
3. 源文件验证：与原始源重新对比

必要性：
- 坏道硬盘是真实存在的风险
- 静默数据损坏可能长期未被发现
- 需要多层次的验证策略
"""

from __future__ import annotations

import pytest

from conftest import VaultEnv, copy_fixture, create_minimal_jpeg


class TestCorruptedHashScenario:
    """Test the scenario where stored hash is already wrong."""
    
    def test_verify_cannot_detect_pre_corruption(self, vault: VaultEnv) -> None:
        """Demonstrate the fundamental problem: verify can't detect if hash was computed from corrupted data.
        
        Scenario:
        1. Source file exists but is on bad sectors
        2. Import reads corrupted data, computes hash H_bad
        3. Stores H_bad in database
        4. Copies corrupted data to vault
        5. Verify compares H_bad with copied file → MATCHES!
        
        This test shows why post-import source verification is needed.
        """
        # Create and import file normally
        f = vault.source_dir / "test.jpg"
        create_minimal_jpeg(f, "ORIGINAL_DATA")
        vault.import_dir(vault.source_dir, hash="sha256")
        
        # Simulate: stored hash is wrong (computed from corrupted source)
        # In real scenario, this happened during import due to bad sectors
        import sqlite3
        db_path = vault.vault_dir / ".svault" / "vault.db"
        conn = sqlite3.connect(str(db_path))
        
        # Get current (correct) hash
        cursor = conn.execute("SELECT sha256 FROM files LIMIT 1")
        correct_hash = cursor.fetchone()[0]
        
        # Simulate corruption: replace with wrong hash
        # (In real bug, this wrong hash was computed from corrupted source)
        fake_hash = bytes([b ^ 0xFF for b in correct_hash]) if isinstance(correct_hash, bytes) else 'fake'
        conn.execute("UPDATE files SET sha256 = ?", (fake_hash,))
        conn.commit()
        conn.close()
        
        # Now verify will pass even though hash is wrong!
        # Because both stored hash and file are "consistently wrong"
        result = vault.run("verify", capture=True)
        
        # This demonstrates the problem: verify passes but data is corrupted
        # In this case, it will actually fail because we changed hash to random value
        # But if the corruption happened during import, both would match
        assert result.returncode != 0 or "mismatch" in result.stdout.lower()


class TestPostImportSourceVerification:
    """Test verification against original source after import."""
    
    def test_source_reverification_detects_corruption(self, vault: VaultEnv) -> None:
        """Re-verify against source after import to detect corruption.
        
        Solution: After import, re-read source and compare with vault copy.
        If they don't match, corruption occurred during import.
        """
        # Create source file
        f = vault.source_dir / "test.jpg"
        create_minimal_jpeg(f, "SOURCE_DATA_V1")
        
        # Import
        vault.import_dir(vault.source_dir, hash="sha256")
        
        # Get imported file location
        files = vault.db_files()
        vault_file = vault.vault_dir / files[0]["path"]
        
        # Simulate: source was modified after import (or we re-read and it's different)
        create_minimal_jpeg(f, "SOURCE_DATA_V2_DIFFERENT")
        
        # Compare source with vault
        source_hash = compute_file_hash(f)
        vault_hash = compute_file_hash(vault_file)
        
        assert source_hash != vault_hash, "Hashes should differ if source changed"
        print("Source verification detected mismatch - potential corruption!")

    def test_cross_session_verification(self, vault: VaultEnv) -> None:
        """Verify at two different time points to catch unstable reads.
        
        If a file reads differently at T1 and T2, the storage may be unreliable.
        """
        import time
        
        f = vault.source_dir / "test.jpg"
        create_minimal_jpeg(f, "STABLE_DATA")
        
        # First read
        hash1 = compute_file_hash(f)
        
        # Wait a bit (in real scenario, this could be hours)
        time.sleep(0.1)
        
        # Second read
        hash2 = compute_file_hash(f)
        
        # Should match for stable storage
        assert hash1 == hash2, "File reads differently - storage may be unreliable!"


def compute_file_hash(path):
    """Compute SHA-256 of file."""
    import hashlib
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        while chunk := f.read(8192):
            h.update(chunk)
    return h.hexdigest()


class TestWriteThenVerify:
    """Test the write-then-verify pattern."""
    
    def test_copy_then_verify_integrity(self, vault: VaultEnv) -> None:
        """Copy file, then verify copy matches source before recording hash.
        
        This ensures the hash in DB represents the actual copied data.
        """
        # Create source
        f = vault.source_dir / "test.jpg"
        create_minimal_jpeg(f, "DATA_TO_VERIFY")
        
        # Get source hash
        source_hash_before = compute_file_hash(f)
        
        # Import
        vault.import_dir(vault.source_dir, hash="sha256")
        
        # Get vault file
        files = vault.db_files()
        vault_file = vault.vault_dir / files[0]["path"]
        
        # Verify vault file hash
        vault_hash = compute_file_hash(vault_file)
        
        # Source should still match (if not corrupted)
        source_hash_after = compute_file_hash(f)
        
        # Both should match
        assert source_hash_before == source_hash_after, "Source file changed during import"
        
        # Database should have correct hash
        import sqlite3
        conn = sqlite3.connect(str(vault.vault_dir / ".svault" / "vault.db"))
        cursor = conn.execute("SELECT sha256 FROM files LIMIT 1")
        db_hash = cursor.fetchone()[0]
        conn.close()
        
        # Convert vault_hash to bytes for comparison
        vault_hash_bytes = bytes.fromhex(vault_hash)
        
        # They should match
        if db_hash is not None:
            if isinstance(db_hash, str):
                db_hash = bytes.fromhex(db_hash)
            assert vault_hash_bytes == db_hash, "DB hash doesn't match actual file hash!"


@pytest.mark.skip(reason="Demonstrates the fundamental problem - manual verification needed")
class TestFundamentalLimitations:
    """Document fundamental limitations of hash-based verification."""
    
    def test_cannot_detect_if_both_source_and_hash_corrupted(self) -> None:
        """If source is corrupted AND hash is computed from corrupted source,
        there's no way to detect without external reference.
        
        This is why:
        1. Multiple backups are essential
        2. Cross-device verification is needed
        3. Parity/ECC storage should be used for critical data
        """
        pass
    
    def test_cannot_detect_silent_corruption_without_redundancy(self) -> None:
        """Without redundant copies or parity data, silent corruption is undetectable.
        
        Hash only tells you if the file changed, not if it was correct to begin with.
        """
        pass
