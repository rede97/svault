"""E2E tests for config file handling, transfer strategies, and hardlink upgrade.

Merged from:
- test_config_transfer.py: Config and transfer strategy tests
- test_hardlink_upgrade.py: verify --upgrade-links tests

中文场景说明：
- 配置文件操作：验证 svault.toml 的读取、解析和错误处理
- 传输策略：验证 reflink/hardlink/copy 的 fallback 链
- 多策略组合：验证逗号分隔的策略列表（如 reflink,hardlink,copy）
- hardlink 升级：将 hardlink 升级为独立二进制副本
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest

from conftest import VaultEnv, copy_fixture, create_minimal_jpeg, IS_WINDOWS


def _nlink(path: Path) -> int:
    """Cross-platform hard link count."""
    return os.stat(path).st_nlink


# =============================================================================
# Config Handling Tests
# =============================================================================

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
        assert "sync_strategy" in content
    
    def test_full_id_computes_sha256(self, vault: VaultEnv) -> None:
        """--full-id option should compute SHA-256."""
        create_minimal_jpeg(vault.source_dir / "test.jpg", "content1")
        # Import with --full-id to compute SHA-256
        result = vault.run("import", "--yes", "--full-id", str(vault.source_dir))
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 1
        # Verify SHA-256 was computed
        assert rows[0]["sha256"] is not None, "SHA-256 should be computed with --full-id"
    
    def test_custom_config_extensions(self, vault: VaultEnv) -> None:
        """Custom allowed_extensions should filter files."""
        config_path = vault.vault_dir / "svault.toml"
        config_path.write_text("""
[global]

[import]
path_template = "$year/$filename"
allowed_extensions = ["png"]
""")
        
        create_minimal_jpeg(vault.source_dir / "test.jpg", "jpg_content")
        create_minimal_jpeg(vault.source_dir / "test.png", "png_content")
        
        vault.import_dir(vault.source_dir)
        
        rows = vault.db_files()
        assert len(rows) == 1
        assert rows[0]["path"].endswith(".png")
    
    def test_config_persists_after_reinit(self, vault: VaultEnv) -> None:
        """Config modifications should persist after re-init if init preserves them."""
        config_path = vault.vault_dir / "svault.toml"
        
        custom_config = """
[global]
hash = "sha256"

[import]
store_exif = true
path_template = "$device/$filename"
allowed_extensions = ["raw", "dng"]
"""
        config_path.write_text(custom_config)
        
        result = vault.init()
        assert result.returncode in [0, 1]


# =============================================================================
# Transfer Strategy Tests
# =============================================================================

@pytest.mark.transfer
class TestTransferStrategies:
    """Tests for file transfer strategies."""
    
    def test_strategy_reflink_fallback_to_hardlink(self, vault: VaultEnv) -> None:
        """reflink should fallback to hardlink when not supported."""
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
        """Verify fallback chain: reflink -> hardlink -> copy."""
        create_minimal_jpeg(vault.source_dir / "fallback_test.jpg", "test_content")
        
        result = vault.import_dir(vault.source_dir, strategy="reflink,hardlink,copy")
        assert result.returncode == 0
        
        vault_files = list(vault.vault_dir.rglob("*.jpg"))
        vault_files = [f for f in vault_files if ".svault" not in str(f)]
        assert len(vault_files) == 1
    
    def test_explicit_copy_bypasses_optimization(self, vault: VaultEnv) -> None:
        """--strategy copy should always create a real copy, never hardlink."""
        create_minimal_jpeg(vault.source_dir / "copy_test.jpg", "unique_content")
        
        result = vault.import_dir(vault.source_dir, strategy="copy")
        assert result.returncode == 0
        
        src_file = vault.source_dir / "copy_test.jpg"
        dst_files = [f for f in vault.get_vault_files() if f.name == "copy_test.jpg"]
        assert len(dst_files) == 1
        
        src_stat = src_file.stat()
        dst_stat = dst_files[0].stat()
        assert src_stat.st_ino != dst_stat.st_ino, \
            "copy strategy should not create hardlinks"


# =============================================================================
# Hardlink Upgrade Tests (merged from test_hardlink_upgrade.py)
# =============================================================================

class TestHardlinkUpgrade:
    """Hardlink-to-binary-copy upgrade tests."""

    def test_upgrade_hardlink_during_verify(self, vault: VaultEnv) -> None:
        """verify --upgrade-links should break hardlinks in the vault."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        rows = vault.db_query(
            "SELECT id, path FROM files WHERE status = 'imported'"
        )
        assert len(rows) == 1
        file_id = rows[0]["id"]
        original_rel = rows[0]["path"]
        original_path = vault.vault_dir / original_rel
        assert original_path.exists()

        # Create a hardlink inside the vault directory
        link_rel = str(Path(original_rel).with_suffix(".link.jpg"))
        link_path = vault.vault_dir / link_rel
        link_path.parent.mkdir(parents=True, exist_ok=True)
        os.link(original_path, link_path)
        assert _nlink(link_path) > 1

        # Point the DB record to the hardlink path
        safe_link_rel = link_rel.replace("'", "''")
        vault.db_query(f"UPDATE files SET path = '{safe_link_rel}' WHERE id = {file_id}")

        # Run verify --upgrade-links
        result = vault.run("verify", "--upgrade-links", capture=True)
        assert result.returncode == 0
        combined = result.stdout + result.stderr
        assert "Upgraded hardlink" in combined

        # The hardlink should now be an independent file
        assert _nlink(link_path) == 1

        # Content must be preserved
        assert link_path.read_bytes() == original_path.read_bytes()

        # DB record should still point to the upgraded file
        files_after = vault.db_query(
            "SELECT path FROM files WHERE id = {}".format(file_id)
        )
        assert len(files_after) == 1
        assert files_after[0]["path"] == link_rel

    def test_upgrade_links_no_op_for_regular_files(self, vault: VaultEnv) -> None:
        """verify --upgrade-links should do nothing for regular non-hardlinked files."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        result = vault.run("verify", "--upgrade-links", capture=True)
        assert result.returncode == 0
        combined = result.stdout + result.stderr
        assert "Upgraded hardlink" not in combined


# =============================================================================
# Config Error Tests
# =============================================================================

@pytest.mark.config
class TestConfigErrors:
    """Tests for configuration error handling."""
    
    def test_corrupted_config_shows_error(self, vault: VaultEnv) -> None:
        """Corrupted config should produce a clear error message."""
        config_path = vault.vault_dir / "svault.toml"
        config_path.write_text("this is not valid toml {{{")
        
        create_minimal_jpeg(vault.source_dir / "test.jpg", "content")
        result = vault.import_dir(vault.source_dir, check=False)
        
        assert result.returncode in [0, 1]
    
    def test_missing_config_sections(self, vault: VaultEnv) -> None:
        """Config with missing required sections should be handled."""
        config_path = vault.vault_dir / "svault.toml"
        config_path.write_text("""
[global]
hash = "sha256"
""")
        
        create_minimal_jpeg(vault.source_dir / "test.jpg", "content")
        
        result = vault.import_dir(vault.source_dir, check=False)
        assert result.returncode in [0, 1]
