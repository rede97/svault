"""Verify command tests.

Tests the integrity verification functionality that checks if vault files
match their stored hashes.

中文场景说明：
- 完整性验证：定期检查 vault 中的文件是否损坏或被篡改
- 损坏检测：模拟文件损坏（位翻转、截断、替换），验证工具能否发现
- 丢失检测：删除文件后运行 verify，确认能报告缺失

必要性：
- 数据完整性：长期存储中文件可能因磁盘错误而损坏
- 安全审计：检测未经授权的文件修改
- 用户信心：让用户确信他们的档案是完好的
"""

from __future__ import annotations

import shutil
from pathlib import Path

import pytest

from conftest import VaultEnv, copy_fixture, create_minimal_jpeg


class TestVerifyBasic:
    """Basic verification tests."""
    
    def test_verify_all_ok(self, vault: VaultEnv) -> None:
        """Verify should pass for all intact files.
        
        Scenario: Vault has been imported and nothing has changed.
        Expected: All files verify successfully.
        """
        # Import some files with SHA-256 (so default verify works)
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "samsung_photo.jpg")
        vault.import_dir(vault.source_dir, hash="sha256")
        
        # Verify all
        result = vault.run("verify", capture=True)
        assert result.returncode == 0
        assert "OK" in result.stdout or "Summary" in result.stdout
    
    def test_verify_single_file_ok(self, vault: VaultEnv) -> None:
        """Verify single file that is intact.
        
        Scenario: User wants to check a specific file.
        Expected: Single file verifies successfully.
        """
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir, hash="sha256")
        
        # Get the imported file path from DB
        files = vault.db_files()
        assert len(files) == 1
        file_path = files[0]["path"]
        
        # Verify single file
        result = vault.run("verify", "--file", file_path, capture=True)
        assert result.returncode == 0


class TestVerifyCorruption:
    """Corruption detection tests."""
    
    def test_verify_detects_bit_flip(self, vault: VaultEnv) -> None:
        """Verify should detect single bit corruption.
        
        Scenario: File corrupted by single bit flip (simulating disk error).
        Expected: Hash mismatch reported.
        
        Necessity: Even a single bit change should be detected to ensure
        data integrity.
        """
        # Import a file with SHA-256
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir, hash="sha256")
        
        # Find the imported file
        files = vault.db_files()
        file_path = files[0]["path"]
        full_path = vault.vault_dir / file_path
        
        # Corrupt single byte (flip bits)
        original_data = full_path.read_bytes()
        corrupted = bytearray(original_data)
        corrupted[100] ^= 0xFF  # Flip all bits in byte at offset 100
        full_path.write_bytes(corrupted)
        
        # Verify should detect corruption
        result = vault.run("verify", capture=True, check=False)
        assert result.returncode != 0
        assert "mismatch" in result.stdout.lower() or "hash" in result.stdout.lower()
    
    def test_verify_detects_truncation(self, vault: VaultEnv) -> None:
        """Verify should detect truncated file.
        
        Scenario: File was truncated (incomplete copy or disk full).
        Expected: Size mismatch reported.
        """
        # Import a file with SHA-256
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir, hash="sha256")
        
        # Find and truncate the file
        files = vault.db_files()
        file_path = files[0]["path"]
        full_path = vault.vault_dir / file_path
        
        original_size = full_path.stat().st_size
        truncated_size = original_size // 2
        
        data = full_path.read_bytes()
        full_path.write_bytes(data[:truncated_size])
        
        # Verify should detect size mismatch
        result = vault.run("verify", capture=True, check=False)
        assert result.returncode != 0
        assert "size" in result.stdout.lower() or "mismatch" in result.stdout.lower()
    
    def test_verify_detects_missing_file(self, vault: VaultEnv) -> None:
        """Verify should detect missing file.
        
        Scenario: File was deleted but still in database.
        Expected: Missing file reported.
        """
        # Import a file with SHA-256
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir, hash="sha256")
        
        # Delete the file
        files = vault.db_files()
        file_path = files[0]["path"]
        full_path = vault.vault_dir / file_path
        full_path.unlink()
        
        # Verify should detect missing file
        result = vault.run("verify", capture=True, check=False)
        assert result.returncode != 0
        assert "missing" in result.stdout.lower()
    
    def test_verify_detects_content_replacement(self, vault: VaultEnv) -> None:
        """Verify should detect content replacement.
        
        Scenario: File replaced with different content but same name.
        Expected: Hash mismatch reported.
        """
        # Import first file with SHA-256
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir, hash="sha256")
        
        # Find the file
        files = vault.db_files()
        file_path = files[0]["path"]
        full_path = vault.vault_dir / file_path
        
        # Replace with different content (but same size approximately)
        create_minimal_jpeg(full_path, "COMPLETELY_DIFFERENT_CONTENT_MARKER")
        
        # Verify should detect mismatch
        result = vault.run("verify", capture=True, check=False)
        assert result.returncode != 0
    
    def test_verify_multiple_corruptions(self, vault: VaultEnv) -> None:
        """Verify should report all corrupted files, not just first.
        
        Scenario: Multiple files corrupted.
        Expected: All corruptions reported in summary.
        """
        # Import multiple files with SHA-256
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "samsung_photo.jpg")
        vault.import_dir(vault.source_dir, hash="sha256")
        
        files = vault.db_files()
        assert len(files) >= 2
        
        # Corrupt both files
        for file_info in files:
            file_path = vault.vault_dir / file_info["path"]
            data = bytearray(file_path.read_bytes())
            data[50] ^= 0xFF  # Corrupt byte at offset 50
            file_path.write_bytes(data)
        
        # Verify should report all failures
        result = vault.run("verify", capture=True, check=False)
        assert result.returncode != 0
        # Should mention hash or mismatch for each file
        assert result.stdout.count("mismatch") >= 2 or result.stdout.count("hash") >= 2


class TestVerifyHashAlgorithms:
    """Test verification with different hash algorithms."""
    
    def test_verify_with_sha256(self, vault: VaultEnv) -> None:
        """Verify using SHA-256 algorithm.
        
        Expected: Verification works with SHA-256 (cryptographic strength).
        """
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        result = vault.run("verify", "-H", "sha256", capture=True)
        assert result.returncode == 0
    
    def test_verify_with_xxh3_128(self, vault: VaultEnv) -> None:
        """Verify using XXH3-128 algorithm.
        
        Note: Files imported with default settings have XXH3-128 computed.
        Expected: OK (hash available and matches).
        """
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        result = vault.run("verify", "-H", "xxh3-128", capture=True)
        # Should succeed - default import uses xxh3-128
        assert result.returncode == 0
        assert "OK" in result.stdout or "0" in result.stdout


class TestVerifySummary:
    """Test verify output formatting."""
    
    def test_verify_summary_counts(self, vault: VaultEnv) -> None:
        """Verify summary should show correct counts.
        
        Expected: Summary shows total, ok, and any failures.
        """
        # Import files with SHA-256
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "samsung_photo.jpg")
        vault.import_dir(vault.source_dir, hash="sha256")
        
        result = vault.run("verify", capture=True)
        assert result.returncode == 0
        
        # Should have summary with counts
        assert "Summary" in result.stdout or "Total" in result.stdout
        assert "OK" in result.stdout or "2" in result.stdout
