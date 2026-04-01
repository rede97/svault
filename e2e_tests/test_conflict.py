"""Filename conflict handling tests.

This module tests automatic renaming when multiple devices produce the same filename.

中文场景说明：
- 多台相机同文件名：摄影师使用多台相机拍摄，默认命名冲突（DSC0001.jpg）
- 婚礼/活动摄影：多台相同型号相机同时工作，产生大量同名文件
- 文件重命名策略：验证 DSC0001.jpg → DSC0001.1.jpg 的自动重命名

必要性：
- 防止文件覆盖导致数据丢失
- 确保所有照片都能导入（不丢失任何一张）
- 摄影师常用场景（多机位拍摄）

Coverage of old test_rules.json scenarios:
- s7_camera_a_first to s14_camera_h_conflict: test_two_cameras_same_filename,
  test_multiple_cameras_same_filename, test_eight_camera_stress_test
"""

from __future__ import annotations

import re

import pytest

from conftest import VaultEnv, assert_file_imported, assert_path_contains, copy_fixture


@pytest.mark.conflict
class TestFilenameConflict:
    """Test filename conflict resolution."""
    
    def test_two_cameras_same_filename(self, vault: VaultEnv) -> None:
        """Two cameras with same filename - second should be renamed.
        
        Scenario:
        - camera_a/DSC0001.jpg (first)
        - camera_b/DSC0001.jpg (second, different content)
        
        Expected:
        - First: DSC0001.jpg
        - Second: DSC0001.1.jpg
        """
        copy_fixture(vault, "camera_a/DSC0001.jpg", subdir="camera_a")
        copy_fixture(vault, "camera_b/DSC0001.jpg", subdir="camera_b")
        
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        filenames = [Path(f["path"]).name for f in files]
        
        # Both should be imported
        assert len(files) == 2
        
        # One should be original name, one renamed
        assert "DSC0001.jpg" in filenames
        renamed = [f for f in filenames if re.match(r"DSC0001\.\d+\.jpg", f)]
        assert len(renamed) == 1, f"Expected one renamed file, got: {filenames}"
    
    @pytest.mark.parametrize("camera_count", [2, 4, 8])
    def test_multiple_cameras_same_filename(self, vault: VaultEnv, camera_count: int) -> None:
        """Multiple cameras with same filename - all should be imported with unique names.
        
        Uses camera_a through camera_h fixtures which have:
        - Same filename: DSC0001.jpg
        - Same device: Sony A7IV
        - Same date: 2024-05-03
        - Different content (different GPS coordinates)
        """
        cameras = [f"camera_{chr(ord('a') + i)}" for i in range(camera_count)]
        
        for cam in cameras:
            fixture_path = f"{cam}/DSC0001.jpg"
            copy_fixture(vault, fixture_path, subdir=cam)
        
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        assert len(files) == camera_count, f"Expected {camera_count} files, got {len(files)}"
        
        filenames = [Path(f["path"]).name for f in files]
        
        # First one should keep original name
        assert "DSC0001.jpg" in filenames
        
        # Rest should be renamed
        renamed = [f for f in filenames if re.match(r"DSC0001\.\d+\.jpg", f)]
        assert len(renamed) == camera_count - 1
    
    def test_eight_camera_stress_test(self, vault: VaultEnv) -> None:
        """Stress test with 8 cameras (maximum conflict scenario from fixtures)."""
        for letter in "abcdefgh":
            copy_fixture(vault, f"camera_{letter}/DSC0001.jpg", subdir=f"camera_{letter}")
        
        vault.import_dir(vault.source_dir)
        
        files = vault.db_files()
        assert len(files) == 8
        
        # Verify all in correct location
        for f in files:
            assert_path_contains(f["path"], "2024", "05-03", "Sony A7IV")
        
        # Verify no camera subdir names in paths (flattened structure)
        for cam in ["camera_a", "camera_b", "camera_c", "camera_d",
                    "camera_e", "camera_f", "camera_g", "camera_h"]:
            for f in files:
                assert cam not in f["path"], f"Path should not contain {cam}"
    
    def test_same_name_same_content_is_duplicate_not_conflict(self, vault: VaultEnv) -> None:
        """Same filename AND same content should be treated as duplicate, not conflict."""
        copy_fixture(vault, "camera_a/DSC0001.jpg", subdir="cam1")
        # Copy same file to cam2 (same content)
        import shutil
        src = vault.source_dir / "cam1" / "DSC0001.jpg"
        (vault.source_dir / "cam2").mkdir(exist_ok=True)
        shutil.copy2(src, vault.source_dir / "cam2" / "DSC0001.jpg")
        
        vault.import_dir(vault.source_dir)
        
        # Only one should be imported (first), second is duplicate
        files = vault.db_files()
        assert len(files) == 1
        assert Path(files[0]["path"]).name == "DSC0001.jpg"


from pathlib import Path
