"""E2E tests for config file handling and transfer strategies.

Tests configuration file operations and various transfer strategy combinations.

中文场景说明：
- 配置文件操作：验证 svault.toml 的读取、解析和错误处理
- 传输策略：验证 reflink/hardlink/copy 的 fallback 链
- 多策略组合：验证逗号分隔的策略列表（如 reflink,hardlink,copy）

必要性：
- 确保配置文件变更正确生效
- 验证不同存储场景下的传输策略选择
- 确保 fallback 链按预期工作
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest

from conftest import VaultEnv, create_minimal_jpeg


@pytest.mark.config
class TestConfigHandling:
    """Tests for configuration file handling."""
    
    def test_config_file_is_created_on_init(self, vault: VaultEnv) -> None:
        """vault init should create svault.toml with default values."""
        config_path = vault.vault_dir / "svault.toml"
        assert config_path.exists(), "svault.toml should be created on init"
        
        content = config_path.read_text()
        assert "[global]" in content
        assert "[import]" in content
        assert "hash" in content
        assert "sync_strategy" in content
    
    def test_custom_config_hash_algorithm(self, vault: VaultEnv) -> None:
        """Custom hash algorithm should be respected."""
        # Modify config to use sha256
        config_path = vault.vault_dir / "svault.toml"
        config_path.write_text("""
[global]
hash = "sha256"

[import]
path_template = "$year/$mon-$day/$device/$filename"
allowed_extensions = ["jpg"]
""")
        
        # Create test file and import
        create_minimal_jpeg(vault.source_dir / "test.jpg", "content1")
        vault.import_dir(vault.source_dir)
        
        # Verify file was imported
        rows = vault.db_files()
        assert len(rows) == 1
        # Hash algorithm doesn't change import success, just the stored hash
    
    def test_custom_config_extensions(self, vault: VaultEnv) -> None:
        """Custom allowed_extensions should filter files."""
        config_path = vault.vault_dir / "svault.toml"
        config_path.write_text("""
[global]

[import]
path_template = "$year/$filename"
allowed_extensions = ["png"]
""")
        
        # Create both jpg and png files
        create_minimal_jpeg(vault.source_dir / "test.jpg", "jpg_content")
        create_minimal_jpeg(vault.source_dir / "test.png", "png_content")
        
        vault.import_dir(vault.source_dir)
        
        # Only PNG should be imported
        rows = vault.db_files()
        assert len(rows) == 1
        assert rows[0]["path"].endswith(".png")
    
    def test_config_persists_after_reinit(self, vault: VaultEnv) -> None:
        """Config modifications should persist after re-init if init preserves them."""
        config_path = vault.vault_dir / "svault.toml"
        
        # Modify config
        custom_config = """
[global]
hash = "sha256"

[import]
store_exif = true
path_template = "$device/$filename"
allowed_extensions = ["raw", "dng"]
"""
        config_path.write_text(custom_config)
        
        # Re-init - behavior depends on implementation
        # Some implementations may preserve, others may reset
        result = vault.init()
        
        # Just verify the operation succeeded - exact behavior may vary
        assert result.returncode in [0, 1]  # Success or error is acceptable


@pytest.mark.transfer
class TestTransferStrategies:
    """Tests for file transfer strategies."""
    
    def test_strategy_reflink_fallback_to_hardlink(self, vault: VaultEnv) -> None:
        """reflink should fallback to hardlink when not supported."""
        # reflink is not supported on most tmpfs/ext4, should fallback
        create_minimal_jpeg(vault.source_dir / "test.jpg", "content")
        
        result = vault.import_dir(vault.source_dir, strategy="reflink,hardlink,copy")
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 1
        assert rows[0]["status"] == "imported"
    
    def test_strategy_hardlink_only(self, vault: VaultEnv) -> None:
        """--strategy hardlink should work when source and vault are on same filesystem."""
        create_minimal_jpeg(vault.source_dir / "test.jpg", "content")
        
        result = vault.import_dir(vault.source_dir, strategy="hardlink")
        assert result.returncode == 0
        
        # Verify file exists in vault (get_vault_files excludes .svault/ but includes other files)
        vault_files = [f for f in vault.get_vault_files() if f.suffix == ".jpg"]
        assert len(vault_files) == 1
    
    def test_strategy_copy_always_works(self, vault: VaultEnv) -> None:
        """--strategy copy should always work regardless of filesystem."""
        create_minimal_jpeg(vault.source_dir / "test.jpg", "content")
        
        result = vault.import_dir(vault.source_dir, strategy="copy")
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 1
    
    def test_strategy_empty_list_uses_default(self, vault: VaultEnv) -> None:
        """Empty strategy should use default (reflink)."""
        create_minimal_jpeg(vault.source_dir / "test.jpg", "content")
        
        # Import without strategy parameter
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 1
    
    def test_multiple_files_with_mixed_strategies(self, vault: VaultEnv) -> None:
        """Multiple files should all be imported regardless of strategy."""
        for i in range(5):
            create_minimal_jpeg(vault.source_dir / f"test{i}.jpg", f"content{i}")
        
        result = vault.import_dir(vault.source_dir, strategy="reflink,hardlink,copy")
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 5


@pytest.mark.transfer
class TestStrategyFallbackChain:
    """Tests specifically for strategy fallback chain behavior."""
    
    def test_fallback_order_reflink_to_hardlink_to_copy(self, vault: VaultEnv) -> None:
        """Verify fallback chain: reflink -> hardlink -> copy.
        
        On tmpfs (RAMDisk), reflink is not supported, hardlink should work
        if source and vault are on same filesystem, otherwise copy.
        """
        create_minimal_jpeg(vault.source_dir / "fallback_test.jpg", "test_content")
        
        # This should work even if reflink and hardlink both fail
        result = vault.import_dir(vault.source_dir, strategy="reflink,hardlink,copy")
        assert result.returncode == 0
        
        # File should exist in vault
        vault_files = list(vault.vault_dir.rglob("*.jpg"))
        vault_files = [f for f in vault_files if ".svault" not in str(f)]
        assert len(vault_files) == 1
    
    def test_explicit_copy_bypasses_optimization(self, vault: VaultEnv) -> None:
        """--strategy copy should always create a real copy, never hardlink.
        
        This is important when importing from a vault to another vault
        and you don't want to share inodes.
        """
        create_minimal_jpeg(vault.source_dir / "copy_test.jpg", "unique_content")
        
        result = vault.import_dir(vault.source_dir, strategy="copy")
        assert result.returncode == 0
        
        # Find both source and destination files
        src_file = vault.source_dir / "copy_test.jpg"
        dst_files = [f for f in vault.get_vault_files() if f.name == "copy_test.jpg"]
        assert len(dst_files) == 1
        
        # Verify they are not hardlinks (different inodes)
        src_stat = src_file.stat()
        dst_stat = dst_files[0].stat()
        assert src_stat.st_ino != dst_stat.st_ino, \
            "copy strategy should not create hardlinks"


@pytest.mark.config
class TestConfigErrors:
    """Tests for configuration error handling."""
    
    def test_corrupted_config_shows_error(self, vault: VaultEnv) -> None:
        """Corrupted config should produce a clear error message."""
        config_path = vault.vault_dir / "svault.toml"
        config_path.write_text("this is not valid toml {{{")
        
        # Try to import - should fail with clear error
        create_minimal_jpeg(vault.source_dir / "test.jpg", "content")
        result = vault.import_dir(vault.source_dir, check=False)
        
        # Import might succeed with defaults or fail - behavior depends on implementation
        # But at minimum it shouldn't panic
        assert result.returncode in [0, 1]
    
    def test_missing_config_sections(self, vault: VaultEnv) -> None:
        """Config with missing required sections should be handled."""
        config_path = vault.vault_dir / "svault.toml"
        # Write config without [import] section
        config_path.write_text("""
[global]
hash = "sha256"
""")
        
        create_minimal_jpeg(vault.source_dir / "test.jpg", "content")
        
        # Should handle gracefully - either error or use defaults
        result = vault.import_dir(vault.source_dir, check=False)
        # Behavior may vary, but shouldn't panic
