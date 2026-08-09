"""Identity-based import tests: deduplication and conflict resolution.

Tests the three-layer deduplication system:
1. CRC32C (fast, check first/last 64KB)
2. XXH3-128 (strong hash of full file)
3. SHA-256 (cryptographic verification)

Also tests filename conflict resolution when same name but different content.

中文场景说明：
- 重复导入：用户多次导入同一批照片（如从相机和云备份分别导入）
- 重命名后导入：用户重命名文件后再次导入
- 跨目录重复：同一文件存在于多个子目录中
- 多机冲突：多台相机产生同名文件（DSC0001.jpg）

身份判定矩阵：
| 内容 | 文件名 | 结果 |
|------|--------|------|
| 相同 | 相同 | duplicate (不重复导入) |
| 相同 | 不同 | duplicate (重命名检测) |
| 不同 | 相同 | conflict (自动重命名) |
| 不同 | 不同 | 正常导入 |

必要性：
- 节省存储空间（避免保存多份相同内容）
- 保持数据库整洁（无重复记录）
- 快速检测（CRC32C 避免计算完整哈希）
- 防止多机拍摄时的文件覆盖

Merged from:
- test_import_conflict.py: filename conflict resolution tests
"""

from __future__ import annotations

import shutil
from pathlib import Path

import pytest

from conftest import VaultEnv, assert_file_duplicate, assert_file_imported, assert_path_contains, copy_fixture, create_minimal_jpeg

import re


@pytest.mark.dedup
class TestCompareLevel:
    """--compare-level：对指纹疑似重复的源端哈希再验证（recheck 的替代能力）

    契约（cli.md / failure-handling §3.1）：
    - fast（默认）：指纹（size+CRC 头尾 64KB+扩展名）命中即判 duplicate
    - mid：指纹命中时对源文件算完整 XXH3-128 与 DB 比对，不符则按新文件导入
    - high：DB 有 sha256 用 sha256，否则回退 xxh3（本套件默认导入无 sha256，
      故 high 行为 == mid）
    """

    def _import_padded(self, vault: VaultEnv, name: str = "big.jpg") -> Path:
        """导入一个 200KB 文件（CRC 盲区位于中段 [64KB,136KB)）"""
        src = vault.source_dir / name
        create_minimal_jpeg(src, "PADDED")
        with open(src, "ab") as f:
            f.write(b"\xff" * (200 * 1024 - src.stat().st_size))
        assert vault.import_dir(vault.source_dir, strategy="copy").returncode == 0
        return src

    def _flip_middle(self, path: Path, byte: bytes) -> None:
        data = bytearray(path.read_bytes())
        data[100 * 1024 : 100 * 1024 + 1] = byte  # CRC 盲区内
        path.write_bytes(bytes(data))

    def test_fast_trusts_fingerprint_despite_middle_edit(self, vault: VaultEnv) -> None:
        src = self._import_padded(vault)
        self._flip_middle(src, b"\xcd")

        result = vault.import_dir(vault.source_dir, strategy="copy")
        assert result.returncode == 0
        assert len(vault.db_files()) == 1, "fast：盲区修改不改变指纹，仍为 duplicate"

    def test_mid_catches_middle_edit_and_reimports(self, vault: VaultEnv) -> None:
        src = self._import_padded(vault)
        self._flip_middle(src, b"\xcd")

        result = vault.run(
            "--yes", "import", str(vault.source_dir),
            "--strategy", "copy", "--compare-level", "mid",
        )
        assert result.returncode == 0
        assert len(vault.db_files()) == 2, "mid：哈希不符应推翻指纹判定，重新导入"
        # 目标名被占 → 自动改名生成第二个物理副本
        jpgs = [f for f in vault.get_vault_files("*.jpg")]
        assert len(jpgs) == 2

    def test_high_falls_back_to_xxh3_without_db_sha256(self, vault: VaultEnv) -> None:
        src = self._import_padded(vault)
        self._flip_middle(src, b"\xce")

        result = vault.run(
            "--yes", "import", str(vault.source_dir),
            "--strategy", "copy", "-c", "2",  # 数字别名 = high
        )
        assert result.returncode == 0
        assert len(vault.db_files()) == 2, "high（无 DB sha256 → xxh3）应同样检出"


@pytest.mark.dedup
class TestDeduplication:
    """Test deduplication at various levels."""
    
    def test_same_file_imported_twice(self, vault: VaultEnv) -> None:
        """Importing same file twice should detect duplicates."""
        # Create and import first time
        test_file = vault.source_dir / "test.jpg"
        header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
        test_file.write_bytes(header + b'unique_content_12345')
        
        vault.import_dir(vault.source_dir)
        assert_file_imported(vault, "test.jpg")
        
        # Import same directory again
        result = vault.import_dir(vault.source_dir)
        
        # Should still only have one file in DB
        files = vault.db_files()
        assert len(files) == 1


@pytest.mark.dedup


# ========== Tests migrated from test_import_conflict.py ==========

@pytest.mark.conflict
class TestFilenameConflict:
    """Filename conflict resolution for same-name, different-content files."""

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

        assert len(files) == 2
        assert "DSC0001.jpg" in filenames
        renamed = [f for f in filenames if re.match(r"DSC0001\.\d+\.jpg", f)]
        assert len(renamed) == 1, f"Expected one renamed file, got: {filenames}"

    @pytest.mark.parametrize("camera_count", [2, 4, 8])
    def test_multiple_cameras_same_filename(self, vault: VaultEnv, camera_count: int) -> None:
        """Multiple cameras with same filename - all should be imported with unique names."""
        cameras = [f"camera_{chr(ord('a') + i)}" for i in range(camera_count)]

        for cam in cameras:
            fixture_path = f"{cam}/DSC0001.jpg"
            copy_fixture(vault, fixture_path, subdir=cam)

        vault.import_dir(vault.source_dir)

        files = vault.db_files()
        assert len(files) == camera_count, f"Expected {camera_count} files, got {len(files)}"

        filenames = [Path(f["path"]).name for f in files]
        assert "DSC0001.jpg" in filenames

        renamed = [f for f in filenames if re.match(r"DSC0001\.\d+\.jpg", f)]
        assert len(renamed) == camera_count - 1

@pytest.mark.dedup
class TestBatchDeduplication:
    """Test deduplication within a single batch."""
    
    def test_multiple_duplicates_in_same_batch(self, vault: VaultEnv) -> None:
        """Multiple copies of same file in one import batch."""
        # Create original and 5 duplicates in same batch
        header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
        content = header + b'batch_duplicate_test'
        
        for i in range(6):
            f = vault.source_dir / f"file_{i}.jpg"
            f.write_bytes(content)
        
        vault.import_dir(vault.source_dir)
        
        # Only one should be imported
        files = vault.db_files()
        assert len(files) == 1
        assert files[0]["status"] == "imported"
    
    def test_cross_directory_duplicates(self, vault: VaultEnv) -> None:
        """Duplicates scattered across subdirectories."""
        header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
        content = header + b'cross_dir_test'
        
        # Create same file in multiple subdirs
        for subdir in ["day1", "day2", "day3", "backup"]:
            (vault.source_dir / subdir).mkdir(exist_ok=True)
            f = vault.source_dir / subdir / "photo.jpg"
            f.write_bytes(content)
        
        vault.import_dir(vault.source_dir)
        
        # Only one should be imported
        files = vault.db_files()
        assert len(files) == 1


    def test_crc_collision_same_prefix_different_content(self, vault: VaultEnv) -> None:
        """CRC collision: Two files with same first 64KB but different content.
        
        JPEG files use CRC strategy Head(64KB), so if two files have identical
        first 64KB but differ afterwards, they will have same CRC but different
        strong hash (XXH3-128).
        
        Expected behavior:
        - Stage B (CRC): Both files have same CRC → marked as "Duplicate"
        - Stage D (Strong Hash): Different XXH3-128 → confirmed as different files
        - Both files imported with conflict resolution (photo.jpg, photo.1.jpg)
        """
        # Create two files with same JPEG header but different content after
        header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
        # Add padding to make files larger than 64KB to ensure collision
        padding = b'\x00' * (65 * 1024)  # 65KB padding
        
        f1 = vault.source_dir / "photo_a.jpg"
        f2 = vault.source_dir / "photo_b.jpg"
        
        # File 1: header + padding + "AAAA..."
        f1.write_bytes(header + padding + b'A' * 1000)
        # File 2: header + padding + "BBBB..." (same CRC header, different content)
        f2.write_bytes(header + padding + b'B' * 1000)
        
        # Both files should have same CRC (first 64KB identical)
        import zlib
        crc1 = zlib.crc32(header + padding[:65536 - len(header)])
        crc2 = zlib.crc32(header + padding[:65536 - len(header)])
        assert crc1 == crc2, "Test setup error: CRC should be identical"
        
        # Import both files
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        # Both files should be imported (not duplicates, different content)
        files = vault.db_files()
        assert len(files) == 2, f"Expected 2 files (different content), got {len(files)}"
        
        # Verify different strong hashes
        hashes = {f["xxh3_128"] for f in files}
        assert len(hashes) == 2, "Files should have different XXH3-128 hashes"


# ========== Tests migrated from test_import.py ==========

@pytest.mark.dedup
class TestDuplicateDetection:
    """Test duplicate file detection based on content hash.
    
    Migrated from test_import.py to consolidate deduplication tests.
    """
    
    def test_exact_duplicate_not_imported(self, vault: VaultEnv, source_factory: callable) -> None:
        """Exact byte-for-byte duplicate should not be imported twice."""
        source_factory(
            "original.jpg",
            exif_date="2024:05:01 10:00:00",
            exif_make="Test",
            exif_model="Camera",
        )
        
        vault.import_dir(vault.source_dir)
        assert_file_imported(vault, "original.jpg")
        
        # Create duplicate with different name
        original = vault.source_dir / "original.jpg"
        duplicate = vault.source_dir / "duplicate.jpg"
        import shutil
        shutil.copy2(original, duplicate)
        
        vault.import_dir(vault.source_dir)
        assert_file_duplicate(vault, "duplicate.jpg")
    
    @pytest.mark.parametrize("dup_count", [1, 3, 6])
    def test_multiple_duplicates(self, vault: VaultEnv, source_factory: callable, dup_count: int) -> None:
        """Test handling of multiple duplicates in batch."""
        source_factory(
            "original.jpg",
            exif_date="2024:05:01 10:00:00",
            exif_make="Test",
            exif_model="Camera",
        )
        
        vault.import_dir(vault.source_dir)
        
        original = vault.source_dir / "original.jpg"
        for i in range(dup_count):
            dup_path = vault.source_dir / f"duplicate_{i}.jpg"
            import shutil
            shutil.copy2(original, dup_path)
        
        vault.import_dir(vault.source_dir)
        
        for i in range(dup_count):
            assert_file_duplicate(vault, f"duplicate_{i}.jpg")
        
        files = vault.db_files()
        assert len(files) == 1
