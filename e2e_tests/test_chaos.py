"""Chaos/edge case tests.

Tests for less common scenarios and potential failure modes.

中文场景说明：
- 损坏文件：传输中断导致的损坏 JPEG、截断文件
- 文件移动：导入前用户整理了文件目录结构
- 空目录：导入空文件夹或只有子目录的文件夹
- 重复导入：同一目录导入两次，第二次应该全部命中缓存

必要性：
- 系统鲁棒性：确保遇到异常文件不崩溃
- 用户体验：优雅处理各种边界情况
- 数据安全：损坏文件应被识别而非静默导入

Coverage of old test_rules.json scenarios:
- c1_rename_before_import: test_renamed_before_import
- c2_move_to_subdir: test_moved_subdirectory
- c3_interrupt_copy: test_truncated_jpeg_handling
- c5_repeat_import: test_second_import_all_duplicates (TestRepeatedImport)
"""

from __future__ import annotations

import shutil
from pathlib import Path

import pytest

from conftest import VaultEnv, copy_fixture, create_minimal_jpeg


@pytest.mark.chaos
@pytest.mark.slow
class TestChaosScenarios:
    """Chaos scenarios that may be slower or less stable."""
    
    def test_truncated_jpeg_handling(self, vault: VaultEnv) -> None:
        """Import a truncated/corrupt JPEG file.
        
        Expected: Should handle gracefully, may fail hash but not crash
        """
        # Create a truncated JPEG
        corrupt = vault.source_dir / "corrupt.jpg"
        header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
        corrupt.write_bytes(header + b'some_data_but_no_end_marker')
        
        # Should not crash
        result = vault.import_dir(vault.source_dir, check=False)
        
        # Check that we got a result (even if it has errors)
        assert result.returncode in [0, 1]  # 0 = success, 1 = some files failed
    
    def test_moved_subdirectory(self, vault: VaultEnv) -> None:
        """File moved to subdirectory before import.
        
        Scenario: User organizes files into subdirs before importing
        Expected: All files found by recursive walk
        """
        copy_fixture(vault, "apple_with_exif.jpg")
        
        # Move to nested subdirectory
        nested = vault.source_dir / "2024" / "vacation" / "iphone"
        nested.mkdir(parents=True)
        (vault.source_dir / "apple_with_exif.jpg").rename(nested / "apple_with_exif.jpg")
        
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        assert len(files) == 1
        assert "apple_with_exif.jpg" in files[0]["path"]
    
    def test_renamed_before_import(self, vault: VaultEnv) -> None:
        """File renamed before import - should still detect as duplicate if content same."""
        # First import
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        # Clear source
        for f in vault.source_dir.rglob("*"):
            if f.is_file():
                f.unlink()
        
        # Copy same fixture with different name
        copy_fixture(vault, "apple_with_exif.jpg")
        (vault.source_dir / "apple_with_exif.jpg").rename(
            vault.source_dir / "renamed_before_import.jpg"
        )
        
        # Second import
        vault.import_dir(vault.source_dir)
        
        # Should be detected as duplicate (not in DB)
        files = vault.db_files()
        assert len(files) == 1  # Still only one
    
    def test_empty_directory(self, vault: VaultEnv) -> None:
        """Import empty directory."""
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        files = vault.db_files()
        assert len(files) == 0
    
    def test_directory_with_only_subdirs(self, vault: VaultEnv) -> None:
        """Import directory containing only empty subdirectories."""
        for subdir in ["empty1", "empty2", "nested/empty3"]:
            (vault.source_dir / subdir).mkdir(parents=True)
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        files = vault.db_files()
        assert len(files) == 0


class TestRepeatedImport:
    """Test repeated import scenarios."""
    
    def test_second_import_all_duplicates(self, vault: VaultEnv) -> None:
        """Second import of same directory should mark all as duplicates."""
        # Copy multiple fixtures
        for fixture in ["apple_with_exif.jpg", "samsung_photo.jpg", "no_exif.jpg"]:
            try:
                copy_fixture(vault, fixture)
            except Exception:
                pass  # Some fixtures may not exist
        
        # Also create a test file
        create_minimal_jpeg(vault.source_dir / "test.jpg")
        
        # First import
        result1 = vault.import_dir(vault.source_dir)
        initial_count = len(vault.db_files())
        assert initial_count > 0
        
        # Second import of same directory
        result2 = vault.import_dir(vault.source_dir)
        
        # Should still have same count (no new files)
        final_count = len(vault.db_files())
        assert final_count == initial_count
    
    def test_import_with_new_and_existing_files(self, vault: VaultEnv) -> None:
        """Import mix of new and already-imported files."""
        # First batch
        f1 = vault.source_dir / "batch1.jpg"
        create_minimal_jpeg(f1, "batch1")
        
        vault.import_dir(vault.source_dir)
        assert len(vault.db_files()) == 1
        
        # Second batch: one old, one new
        f2 = vault.source_dir / "batch2.jpg"
        create_minimal_jpeg(f2, "batch2")
        
        vault.import_dir(vault.source_dir)
        
        # Should have both
        files = vault.db_files()
        assert len(files) == 2
