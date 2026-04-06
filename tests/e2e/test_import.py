"""Import functionality tests.

Merged from:
- test_import_basic.py: Normal import scenarios, duplicate detection
- test_import_force.py: --force flag behavior
- test_import_ignore.py: Vault self-protection

中文场景说明：
- 标准 EXIF 导入：用户从 iPhone/相机导入带完整元数据的照片（90%场景）
- 无设备信息：某些经过编辑或老照片丢失设备信息
- 无 EXIF：截图、扫描件等没有拍摄元数据的文件
- Samsung 设备：测试 Android 设备的特殊处理
- 重复检测：用户多次导入同一批照片，避免存储浪费
- 强制导入：覆盖已有文件或恢复被删除的文件
- Vault 自保护：导入时不扫描 vault 自身目录
"""

from __future__ import annotations

import json
import sqlite3
import tempfile
import time
from pathlib import Path

import pytest

from conftest import (
    VaultEnv,
    assert_file_imported,
    assert_path_contains,
    copy_fixture,
    create_minimal_jpeg,
)


class TestNormalImport:
    """Test normal import scenarios with various EXIF conditions."""
    
    def test_import_with_exif_date_and_device(self, vault: VaultEnv, source_factory: callable) -> None:
        """Import file with EXIF date and Apple device info.
        
        Expected: File imported to $year/$mon-$day/$device/$filename
        """
        source_factory(
            "apple_test.jpg",
            exif_date="2024:05:01 10:30:00",
            exif_make="Apple",
            exif_model="iPhone 15",
        )
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        row = assert_file_imported(vault, "apple_test.jpg")
        assert_path_contains(
            row["path"],
            "2024",
            "05-01",
            "Apple iPhone 15",
            "apple_test.jpg",
        )
        assert row["crc32c"] is not None
    
    def test_import_no_device(self, vault: VaultEnv, source_factory: callable) -> None:
        """Import file with EXIF date but no Make/Model.
        
        Expected: device=Unknown
        """
        source_factory(
            "no_device.jpg",
            exif_date="2024:05:01 18:00:00",
        )
        
        vault.import_dir(vault.source_dir)
        
        row = assert_file_imported(vault, "no_device.jpg")
        assert_path_contains(row["path"], "2024", "05-01", "Unknown")
    
    def test_import_no_exif_uses_mtime(self, vault: VaultEnv, source_factory: callable) -> None:
        """Import file without EXIF - should use mtime fallback.
        
        Expected: Path derived from file modification time
        """
        target_ts = time.mktime(time.strptime("2024:03:15 08:00:00", "%Y:%m:%d %H:%M:%S"))
        source_factory("no_exif.jpg", mtime=target_ts)
        
        vault.import_dir(vault.source_dir)
        
        row = assert_file_imported(vault, "no_exif.jpg")
        assert_path_contains(row["path"], "2024", "03-15", "Unknown")
    
    def test_import_samsung_device(self, vault: VaultEnv, source_factory: callable) -> None:
        """Import Samsung device photo.
        
        Expected: Model already starts with 'Samsung', no duplication
        """
        source_factory(
            "samsung.jpg",
            exif_date="2024:05:02 14:20:00",
            exif_make="Samsung",
            exif_model="Galaxy S24",
        )
        
        vault.import_dir(vault.source_dir)
        
        row = assert_file_imported(vault, "samsung.jpg")
        assert_path_contains(row["path"], "Samsung")
        assert "Samsung Samsung" not in row["path"]
    
    def test_import_avoids_redundant_make(self, vault: VaultEnv, source_factory: callable) -> None:
        """Model starting with Make should not duplicate Make name.
        
        Expected: "Apple iPhone 14" not "Apple Apple iPhone 14"
        """
        source_factory(
            "apple_redundant.jpg",
            exif_date="2024:05:02 09:00:00",
            exif_make="Apple",
            exif_model="Apple iPhone 14",
        )
        
        vault.import_dir(vault.source_dir)
        
        row = assert_file_imported(vault, "apple_redundant.jpg")
        assert "Apple iPhone 14" in row["path"]
        assert "Apple Apple" not in row["path"]


# Note: Duplicate detection tests are in test_import_dedup.py

class TestForceImport:
    """Test `import --force` behavior."""

    def test_force_import_duplicate(self, vault: VaultEnv, source_factory: callable) -> None:
        """Force-importing an exact duplicate should overwrite the vault file."""
        source_factory(
            "photo.jpg",
            exif_date="2024:01:01 12:00:00",
            exif_make="Apple",
        )

        r1 = vault.import_dir(vault.source_dir)
        assert r1.returncode == 0
        data1 = json.loads(r1.stdout)
        assert data1["imported"] == 1

        # Without force: skipped as duplicate
        r2 = vault.import_dir(vault.source_dir)
        assert r2.returncode == 0
        data2 = json.loads(r2.stdout)
        assert data2["duplicate"] == 1

        # With force: re-processed
        r3 = vault.import_dir(vault.source_dir, force=True)
        assert r3.returncode == 0
        data3 = json.loads(r3.stdout)
        assert data3["imported"] == 1

        rows = vault.find_file_in_db("photo.jpg")
        assert len(rows) == 1
        assert rows[0]["status"] == "imported"

        files = vault.get_vault_files("photo.jpg")
        assert len(files) == 1

        v = vault.run("verify")
        assert v.returncode == 0

    def test_force_import_same_name_different_content(
        self, vault: VaultEnv, source_factory: callable
    ) -> None:
        """Two different files that resolve to the same vault path."""
        common_mtime = time.time()
        source_factory(
            "IMG.jpg",
            content=b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00" + b"A" * 20,
            mtime=common_mtime,
            subdir="dir_a",
        )
        source_factory(
            "IMG.jpg",
            content=b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00" + b"B" * 20,
            mtime=common_mtime,
            subdir="dir_b",
        )

        r1 = vault.import_dir(vault.source_dir / "dir_a")
        assert r1.returncode == 0
        hash1 = vault.find_file_in_db("IMG.jpg")[0]["xxh3_128"]

        r2 = vault.import_dir(vault.source_dir / "dir_b", force=True)
        assert r2.returncode == 0

        vault_files = vault.get_vault_files("IMG*.jpg")
        assert len(vault_files) == 2
        basenames = {f.name for f in vault_files}
        assert "IMG.jpg" in basenames
        assert "IMG.1.jpg" in basenames

        db_rows = vault.db_query(
            "SELECT * FROM files WHERE path LIKE '%IMG%.jpg%' AND status = 'imported'"
        )
        assert len(db_rows) == 2
        hashes = {r["xxh3_128"] for r in db_rows}
        assert hash1 in hashes

    def test_force_import_recovers_deleted_file(
        self, vault: VaultEnv, source_factory: callable
    ) -> None:
        """Force-importing after the vault copy was deleted should restore it."""
        source_factory(
            "photo.jpg",
            exif_date="2024:01:01 12:00:00",
            exif_make="Apple",
        )

        r1 = vault.import_dir(vault.source_dir)
        assert r1.returncode == 0

        vault_files = vault.get_vault_files("photo.jpg")
        assert len(vault_files) == 1
        vault_files[0].unlink()

        r2 = vault.import_dir(vault.source_dir, force=True)
        assert r2.returncode == 0
        data2 = json.loads(r2.stdout)
        assert data2["imported"] == 1

        restored = vault.get_vault_files("photo.jpg")
        assert len(restored) == 1

        v = vault.run("verify")
        assert v.returncode == 0


class TestImportIgnoresVault:
    """Test that import skips the vault when source is an ancestor."""

    def test_import_from_ancestor_skips_vault(self, vault: VaultEnv) -> None:
        """Importing from a parent directory must not re-import vault files."""
        with tempfile.TemporaryDirectory() as tmp:
            tree = Path(tmp)
            vault_sub = tree / "myvault"
            src_sub = tree / "my_source"
            vault_sub.mkdir()
            src_sub.mkdir()

            vault.run("init", cwd=vault_sub, check=True)

            header = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00"
            (src_sub / "outside.jpg").write_bytes(header + b"A" * 20)
            (vault_sub / "existing.jpg").write_bytes(header + b"B" * 20)

            r = vault.run(
                "import",
                "--yes",
                "--output=json",
                "--target",
                str(vault_sub),
                str(tree),
            )
            assert r.returncode == 0, f"Import failed: {r.stderr}"
            data = json.loads(r.stdout)

            assert data["imported"] == 1, f"Expected 1 imported, got {data}"
            assert data["total"] == 1

            db_path = vault_sub / ".svault" / "vault.db"
            conn = sqlite3.connect(str(db_path))
            try:
                rows = conn.execute(
                    "SELECT path FROM files WHERE path LIKE '%.svault%'"
                ).fetchall()
                assert len(rows) == 0, f"Vault metadata leaked: {rows}"

                rows = conn.execute(
                    "SELECT path FROM files WHERE path LIKE '%existing.jpg%'"
                ).fetchall()
                assert len(rows) == 0, f"Vault file was re-imported: {rows}"
            finally:
                conn.close()


class TestExistingFixtures:
    """Tests using pre-generated fixture files."""
    
    def test_fixture_apple_with_exif(self, vault: VaultEnv) -> None:
        """Test with pre-generated apple_with_exif.jpg fixture."""
        copy_fixture(vault, "apple_with_exif.jpg")
        
        vault.import_dir(vault.source_dir)
        
        row = assert_file_imported(vault, "apple_with_exif.jpg")
        assert_path_contains(row["path"], "2024", "05-01", "Apple iPhone 15")
    
    def test_fixture_samsung(self, vault: VaultEnv) -> None:
        """Test with pre-generated samsung_photo.jpg fixture."""
        copy_fixture(vault, "samsung_photo.jpg")
        
        vault.import_dir(vault.source_dir)
        
        row = assert_file_imported(vault, "samsung_photo.jpg")
        assert "Samsung" in row["path"]


# ========== EXIF Fallback Tests (merged from test_exif_fallback.py) ==========

class TestExifFallback:
    """Tests for EXIF fallback scenarios (missing or incomplete EXIF metadata).
    
    中文场景说明：
    - 无 EXIF 照片：使用文件修改时间(mtime)作为拍摄时间，设备显示为"Unknown"
    - 部分 EXIF：有设备信息但无日期，或反之
    - 无效 EXIF：日期格式错误，损坏的 EXIF 数据
    """
    
    def test_no_exif_uses_mtime_for_date(self, vault: VaultEnv) -> None:
        """Photo without EXIF should use file mtime for path organization."""
        import os
        
        # Set specific mtime
        target_time = time.mktime(time.strptime("2024:06:15 14:30:00", "%Y:%m:%d %H:%M:%S"))
        copy_fixture(vault, "no_exif.jpg")
        
        # Adjust mtime after copying
        test_file = vault.source_dir / "no_exif.jpg"
        test_file.touch()
        os.utime(test_file, (target_time, target_time))
        
        vault.import_dir(vault.source_dir)
        
        rows = vault.db_files()
        assert len(rows) == 1
        
        # Path should contain date from mtime
        assert "2024" in rows[0]["path"]
        assert "Unknown" in rows[0]["path"]
    
    def test_no_exif_device_is_unknown(self, vault: VaultEnv) -> None:
        """Photo without EXIF should have device = 'Unknown'."""
        copy_fixture(vault, "no_exif.jpg")
        
        vault.import_dir(vault.source_dir)
        
        rows = vault.db_files()
        assert len(rows) == 1
        assert "Unknown" in rows[0]["path"]
    
    def test_corrupted_exif_fallback_to_mtime(self, vault: VaultEnv) -> None:
        """Corrupted EXIF should fallback to mtime, not crash."""
        import os
        target_time = time.mktime(time.strptime("2024:09:01 08:00:00", "%Y:%m:%d %H:%M:%S"))
        
        test_file = vault.source_dir / "corrupted_exif.jpg"
        
        # Create JPEG with invalid EXIF
        jpeg_header = b'\xff\xd8\xff\xe1\x00\x10Exif\x00\x00'
        corrupted_data = b'CORRUPTED_DATA_NOT_VALID_EXIF'
        image_data = b'\xff\xd9'
        
        test_file.write_bytes(jpeg_header + corrupted_data + image_data)
        os.utime(test_file, (target_time, target_time))
        
        result = vault.import_dir(vault.source_dir)
        assert result.returncode == 0
        
        rows = vault.db_files()
        assert len(rows) == 1
        assert "Unknown" in rows[0]["path"]
    
    def test_partial_exif_device_only_no_date(self, vault: VaultEnv) -> None:
        """Photo with device but no date should use mtime for date."""
        from conftest import EXIFTOOL_AVAILABLE
        
        if not EXIFTOOL_AVAILABLE:
            pytest.skip("exiftool not available")
        
        import os
        import subprocess
        
        target_time = time.mktime(time.strptime("2024:08:10 16:00:00", "%Y:%m:%d %H:%M:%S"))
        
        test_file = vault.source_dir / "device_only.jpg"
        create_minimal_jpeg(test_file, "content")
        os.utime(test_file, (target_time, target_time))
        
        subprocess.run([
            "exiftool", "-overwrite_original", "-P",
            "-Make=TestCamera", "-Model=TestModel",
            str(test_file)
        ], capture_output=True)
        
        vault.import_dir(vault.source_dir)
        
        rows = vault.db_files()
        assert len(rows) == 1
        assert "2024" in rows[0]["path"]
