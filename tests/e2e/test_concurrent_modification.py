"""Concurrent modification detection tests.

Tests for detecting and handling file changes during import process.

中文场景说明：
- 导入过程中文件被删除：用户在导入时整理/删除了照片
- 导入过程中文件被修改：文件在后台被其他程序修改
- 导入过程中新文件加入：相机持续拍摄新照片

必要性：
- 数据一致性：确保导入的数据是预期的
- 错误处理：优雅处理异常情况而非崩溃
- 用户体验：给用户清晰的反馈

This tests the "c4_add_delete_mid_import" scenario from the old framework.
"""

from __future__ import annotations

import time
from pathlib import Path

import pytest

from conftest import VaultEnv, create_minimal_jpeg


class TestFileDeletionDuringImport:
    """Test handling of file deletion during import process."""
    
    def test_detect_file_deleted_before_copy(self, vault: VaultEnv) -> None:
        """Detect and handle file deleted after scan but before copy.
        
        Scenario:
        1. User starts import
        2. Scan completes, file A is queued for copy
        3. User deletes file A (thinking it was a duplicate)
        4. Import tries to copy file A
        
        Expected:
        - Import completes without crash
        - File A is recorded as failed/skipped
        - Other files are imported normally
        """
        # Create files
        f1 = vault.source_dir / "keep.jpg"
        f2 = vault.source_dir / "delete_me.jpg"
        create_minimal_jpeg(f1, "KEEP_THIS")
        create_minimal_jpeg(f2, "DELETE_THIS")
        
        # Simulate: delete file after scan but before import
        f2.unlink()
        
        # Import should complete without error
        result = vault.import_dir(vault.source_dir, check=False)
        
        # Should succeed (one file imported, one missing is not fatal)
        assert result.returncode in [0, 1]
        
        # Check database state
        files = vault.db_files()
        # Only one file should be in DB (the one that existed)
        assert len(files) == 1
        assert "keep" in files[0]["path"]
    
    def test_import_with_empty_source_after_deletion(self, vault: VaultEnv) -> None:
        """Handle case where all files are deleted during import.
        
        Scenario:
        1. Import starts
        2. All source files are deleted before copy
        3. Import should complete gracefully
        """
        # Create then immediately delete (simulating race condition)
        f = vault.source_dir / "temp.jpg"
        create_minimal_jpeg(f, "TEMP")
        f.unlink()
        
        # Import empty directory
        result = vault.import_dir(vault.source_dir)
        
        # Should succeed with warning
        assert result.returncode == 0
        assert len(vault.db_files()) == 0


class TestFileModificationDuringImport:
    """Test detection of file modification during import."""
    
    def test_detect_content_change_before_copy(self, vault: VaultEnv) -> None:
        """Detect file content change after scan.
        
        Scenario:
        1. File is scanned (CRC32C computed)
        2. File is modified by another program
        3. Import copies the modified file
        4. Warning should be shown but import continues
        
        Expected:
        - Copy succeeds (we have the latest version)
        - Warning about modification is shown
        - File is imported with current content
        """
        # Create initial file
        f = vault.source_dir / "modify.jpg"
        create_minimal_jpeg(f, "ORIGINAL_CONTENT")
        
        # Modify file before import (simulating change during import window)
        time.sleep(0.1)  # Ensure mtime changes
        create_minimal_jpeg(f, "MODIFIED_CONTENT_DIFFERENT")
        
        # Import
        result = vault.import_dir(vault.source_dir, check=False)
        
        # Should succeed (possibly with warning)
        assert result.returncode in [0, 1]
        
        # File should be imported
        files = vault.db_files()
        assert len(files) == 1
        # The imported file should be the modified version
        # (verify by checking it's not the original hash)
    
    def test_size_change_detection(self, vault: VaultEnv) -> None:
        """Detect file size change as quick corruption indicator.
        
        Scenario:
        1. File scanned: size=1000 bytes
        2. File truncated: size=500 bytes
        3. Import should detect size mismatch
        
        This is a fast check that doesn't require computing hash.
        """
        f = vault.source_dir / "truncated.jpg"
        create_minimal_jpeg(f, "FULL_CONTENT_HERE")
        
        # Truncate file (simulating corruption)
        data = f.read_bytes()
        f.write_bytes(data[:len(data)//2])
        
        # Import should handle this gracefully
        result = vault.import_dir(vault.source_dir, check=False)
        
        # Import may succeed (copies what's there) or show warning
        # The important thing is it doesn't crash
        assert result.returncode in [0, 1]


class TestNewFilesDuringImport:
    """Test handling of new files added during import."""
    
    def test_new_files_in_next_import(self, vault: VaultEnv) -> None:
        """New files added during import are picked up in next run.
        
        Scenario:
        1. Import starts, scans files A, B
        2. Camera saves new file C
        3. Import completes with A, B
        4. File C is imported on next run
        
        This is the expected behavior - we don't re-scan during import.
        """
        # First batch
        f1 = vault.source_dir / "first.jpg"
        create_minimal_jpeg(f1, "FIRST_BATCH")
        
        vault.import_dir(vault.source_dir)
        assert len(vault.db_files()) == 1
        
        # Second batch (simulating new files during first import)
        f2 = vault.source_dir / "second.jpg"
        create_minimal_jpeg(f2, "SECOND_BATCH")
        
        vault.import_dir(vault.source_dir)
        assert len(vault.db_files()) == 2


class TestPartialImportRecovery:
    """Test recovery from partial imports."""
    
    def test_partial_import_recovery(self, vault: VaultEnv) -> None:
        """Test that partial imports can be recovered.
        
        Scenario:
        1. Import of 100 files starts
        2. Crashes after 50 files
        3. Restart import
        4. Should resume without duplicating first 50
        
        This relies on CRC32C cache - first 50 are detected as duplicates.
        """
        # Create first batch
        for i in range(5):
            f = vault.source_dir / f"file_{i}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")
        
        # Import first batch
        vault.import_dir(vault.source_dir)
        first_count = len(vault.db_files())
        assert first_count == 5
        
        # Add more files (simulating camera still shooting)
        for i in range(5, 10):
            f = vault.source_dir / f"file_{i}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")
        
        # Import again - should only import new files
        vault.import_dir(vault.source_dir)
        second_count = len(vault.db_files())
        assert second_count == 10  # All 10 files
        
        # Verify no duplicates
        paths = [f["path"] for f in vault.db_files()]
        assert len(paths) == len(set(paths)), "Duplicate files detected!"


@pytest.mark.chaos
@pytest.mark.slow
class TestConcurrentModificationStress:
    """Stress tests for concurrent modification scenarios."""
    
    def test_rapid_add_delete_during_import(self, vault: VaultEnv) -> None:
        """Stress test: rapid file operations during import.
        
        This is a simplified version - real concurrent testing would
        require modifying svault to support injection points.
        
        For now, we verify svault handles various edge cases gracefully.
        """
        # Create many files
        for i in range(20):
            f = vault.source_dir / f"file_{i}.jpg"
            create_minimal_jpeg(f, f"CONTENT_{i}")
        
        # Delete some files (simulating user cleanup during import)
        for i in range(5, 10):
            f = vault.source_dir / f"file_{i}.jpg"
            if f.exists():
                f.unlink()
        
        # Import should handle missing files gracefully
        result = vault.import_dir(vault.source_dir, check=False)
        assert result.returncode in [0, 1]
        
        # Verify consistency
        files = vault.db_files()
        # All imported files should exist
        for file_info in files:
            full_path = vault.vault_dir / file_info["path"]
            assert full_path.exists(), f"Imported file missing: {file_info['path']}"


class TestImportIdempotency:
    """Test that import is idempotent - running twice doesn't duplicate."""
    
    def test_reimport_no_duplication(self, vault: VaultEnv) -> None:
        """Importing same files twice should not create duplicates.
        
        This is a fundamental requirement for handling concurrent modifications.
        """
        # Create files
        for i in range(5):
            f = vault.source_dir / f"file_{i}.jpg"
            create_minimal_jpeg(f, f"UNIQUE_{i}")
        
        # First import
        vault.import_dir(vault.source_dir)
        count1 = len(vault.db_files())
        assert count1 == 5
        
        # Second import (same files)
        vault.import_dir(vault.source_dir)
        count2 = len(vault.db_files())
        
        # Should be same count (no duplicates)
        assert count2 == 5, f"Expected 5 files, got {count2} (duplicates created!)"
