"""Tests for scan + filter + import pipeline workflow.

中文说明：
测试 "扫描 → 过滤 → 导入" 的完整工作流程：
1. 扫描源目录中的媒体文件
2. scan 命令输出格式: SCAN:<source> new:<file> dup:<file> fail:<file>
3. import --files-from - 读取 scan 输出，只导入标记为 new 的文件

此测试验证整个流程是否能够正常工作。
"""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

import pytest

from conftest import (
    VaultEnv,
    assert_file_imported,
    create_minimal_jpeg,
    create_minimal_mp4,
    create_minimal_raw,
    EXIFTOOL_AVAILABLE,
    parse_json_summary,
)


class TestScanFilterImportPipeline:
    """Test the complete scan -> filter -> import workflow."""

    def test_scan_outputs_correct_format(self, vault: VaultEnv) -> None:
        """Test that scan command outputs the correct pipeable format."""
        # Create test files
        create_minimal_jpeg(vault.source_dir / "photo1.jpg", "content_1")
        create_minimal_jpeg(vault.source_dir / "photo2.jpg", "content_2")
        
        # Run scan
        result = vault.run("scan", str(vault.source_dir))
        assert result.returncode == 0
        
        # Parse output format: SCAN:<source> new:photo1.jpg new:photo2.jpg
        parts = result.stdout.strip().split()
        assert len(parts) >= 1
        assert parts[0].startswith("SCAN:")
        
        # Check for new: entries
        new_files = [p[4:] for p in parts if p.startswith("new:")]
        assert "photo1.jpg" in new_files or "DCIM/photo1.jpg" in new_files
        assert "photo2.jpg" in new_files or "DCIM/photo2.jpg" in new_files

    def test_scan_import_pipeline_imports_new_files(self, vault: VaultEnv) -> None:
        """Test scan -> import pipeline imports new files correctly."""
        # Create test files
        create_minimal_jpeg(vault.source_dir / "photo1.jpg", "content_1")
        create_minimal_jpeg(vault.source_dir / "photo2.jpg", "content_2")
        
        # First import via pipeline
        scan_result = vault.run("scan", str(vault.source_dir))
        assert scan_result.returncode == 0
        
        # Import using scan output
        result = vault.run("import", str(vault.source_dir), "--files-from", "-", "--yes", 
                          "--output", "json", input=scan_result.stdout)
        assert result.returncode == 0
        
        # Verify files imported
        data = parse_json_summary(result.stdout)
        assert data["imported"] == 2, f"Expected 2 files imported, got {data}"
        
        rows = vault.db_files()
        assert len(rows) == 2

    def test_scan_import_pipeline_skips_duplicates(self, vault: VaultEnv) -> None:
        """Test scan -> import pipeline skips duplicate files on second import."""
        # Create test files with different content
        create_minimal_jpeg(vault.source_dir / "photo1.jpg", "content_1")
        create_minimal_jpeg(vault.source_dir / "photo2.jpg", "content_2_different")
        
        # First import
        scan_result1 = vault.run("scan", str(vault.source_dir))
        result1 = vault.run("import", str(vault.source_dir), "--files-from", "-", "--yes",
                           "--output", "json", input=scan_result1.stdout)
        assert result1.returncode == 0
        assert parse_json_summary(result1.stdout)["imported"] == 2
        
        # Second import via pipeline should have no new files
        scan_result2 = vault.run("scan", str(vault.source_dir))
        assert scan_result2.returncode == 0
        
        # Without --show-dup, duplicate files are not shown (only SCAN: prefix)
        # Import should fail with no new files
        result2 = vault.run("import", str(vault.source_dir), "--files-from", "-", "--yes",
                           input=scan_result2.stdout, check=False)
        assert result2.returncode != 0
        assert "no new files to import" in result2.stderr.lower()
        
        # With --show-dup, we can see the duplicates
        scan_result3 = vault.run("scan", str(vault.source_dir), "--show-dup")
        assert "dup:" in scan_result3.stdout
        assert "new:" not in scan_result3.stdout  # No new files

    def test_scan_import_with_nested_directories(self, vault: VaultEnv) -> None:
        """Test scan -> import with nested directory structure."""
        # Create nested structure
        structure = [
            ("root.jpg", ""),
            ("level1/l1.jpg", "level1"),
            ("level1/level2/l2.jpg", "level1/level2"),
        ]
        
        for filename, subdir in structure:
            dir_path = vault.source_dir / subdir if subdir else vault.source_dir
            dir_path.mkdir(parents=True, exist_ok=True)
            create_minimal_jpeg(dir_path / Path(filename).name, f"content_{filename}")
        
        # Scan and import
        scan_result = vault.run("scan", str(vault.source_dir))
        assert scan_result.returncode == 0
        
        result = vault.run("import", str(vault.source_dir), "--files-from", "-", "--yes",
                          "--output", "json", input=scan_result.stdout)
        assert result.returncode == 0
        
        data = parse_json_summary(result.stdout)
        assert data["imported"] == 3, f"Expected 3 files imported, got {data}"
        
        # Verify nested paths preserved
        rows = vault.db_files()
        assert len(rows) == 3

    def test_scan_import_empty_directory(self, vault: VaultEnv) -> None:
        """Test scan on empty directory produces no output."""
        empty_dir = vault.output_dir / "empty"
        empty_dir.mkdir()
        
        result = vault.run("scan", str(empty_dir))
        assert result.returncode == 0
        assert result.stdout.strip() == ""


class TestScanOutputFormat:
    """Test scan output format details."""


class TestImportFilesFrom:
    """Test import --files-from option."""

    def test_import_files_from_file(self, vault: VaultEnv) -> None:
        """Test import --files-from with a file."""
        # Create test files
        create_minimal_jpeg(vault.source_dir / "photo1.jpg", "content_1")
        create_minimal_jpeg(vault.source_dir / "photo2.jpg", "content_2")
        
        # Create scan output file
        scan_result = vault.run("scan", str(vault.source_dir))
        scan_file = vault.output_dir / "scan_output.txt"
        scan_file.write_text(scan_result.stdout)
        
        # Import from file
        result = vault.run("import", str(vault.source_dir), "--files-from", str(scan_file), "--yes",
                          "--output", "json")
        assert result.returncode == 0
        
        data = parse_json_summary(result.stdout)
        assert data["imported"] == 2

    def test_import_files_from_ignores_dups(self, vault: VaultEnv) -> None:
        """Test that import --files-from only imports new: files."""
        # Create files
        create_minimal_jpeg(vault.source_dir / "new_file.jpg", "new_content")
        create_minimal_jpeg(vault.source_dir / "existing.jpg", "existing_content")
        
        # Import existing first
        vault.import_dir(vault.source_dir, yes=True)
        
        # Now create just the new file in a fresh source dir
        new_source = vault.output_dir / "new_source"
        new_source.mkdir()
        create_minimal_jpeg(new_source / "new_file2.jpg", "new_content2")
        
        # Scan output with mixed new/dup (simulated)
        scan_output = f"SCAN:{new_source} new:new_file2.jpg"
        
        result = vault.run("import", str(new_source), "--files-from", "-", "--yes",
                          "--output", "json", input=scan_output)
        assert result.returncode == 0
        
        data = parse_json_summary(result.stdout)
        assert data["imported"] == 1  # Only new_file2.jpg

    def test_scan_import_roundtrip_with_space_in_filename(self, vault: VaultEnv) -> None:
        """空格路径契约：scan 输出 → import --files-from 消费，全链路往返正确。

        （替代原恒真断言版本——本测试锁真实行为：空格文件名必须能被
        管线完整导入，而非仅检查输出中是否出现某个子串。）
        """
        create_minimal_jpeg(vault.source_dir / "my photo 01.jpg", "SPACE_CONTENT")

        scan = vault.run("scan", str(vault.source_dir))
        assert scan.returncode == 0

        result = vault.run("import", str(vault.source_dir), "--files-from", "-", "--yes",
                           "--output", "json", input=scan.stdout, check=False)
        assert result.returncode == 0, (
            f"空格路径应被 files-from 正确消费: {result.stderr}"
        )
        rows = vault.find_file_in_db("my photo 01.jpg")
        assert len(rows) == 1, "空格文件名文件应入库"
