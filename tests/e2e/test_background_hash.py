"""Background-hash command tests.

Tests the incremental SHA-256 computation for files imported with
only the fast hash (XXH3-128).

中文场景说明：
- 默认导入使用 XXH3-128，SHA-256 留空以节省时间
- background-hash 在系统空闲时补齐 SHA-256
- 适合大量导入后异步完成强哈希
"""

from __future__ import annotations

import pytest

from conftest import VaultEnv, copy_fixture


class TestBackgroundHashBasic:
    """Basic background-hash tests."""

    def test_background_hash_computes_missing_sha256(self, vault: VaultEnv) -> None:
        """background-hash should compute SHA-256 for files imported with fast hash."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        # Verify sha256 is initially NULL
        files = vault.db_files()
        assert len(files) == 1
        assert files[0]["sha256"] is None

        result = vault.run("verify", "--background-hash", capture=True)
        assert result.returncode == 0
        combined = result.stdout + result.stderr
        assert "Processed 1 file(s), 0 failed." in combined

        # Verify sha256 is now populated
        files = vault.db_files()
        assert len(files) == 1
        assert files[0]["sha256"] is not None
        assert len(files[0]["sha256"]) > 0

    def test_background_hash_no_pending_files(self, vault: VaultEnv) -> None:
        """background-hash should report 0 processed when there are no pending files."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir, hash="secure")

        # SHA-256 already present because of secure hash import
        result = vault.run("verify", "--background-hash", capture=True)
        assert result.returncode == 0
        combined = result.stdout + result.stderr
        assert "Processed 0 file(s), 0 failed." in combined


class TestBackgroundHashOptions:
    """background-hash option tests."""

    def test_background_hash_limit(self, vault: VaultEnv) -> None:
        """background-hash --limit should cap the number of processed files."""
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "samsung_photo.jpg")
        vault.import_dir(vault.source_dir)

        pending_before = [f for f in vault.db_files() if f["sha256"] is None]
        assert len(pending_before) == 2

        result = vault.run("verify", "--background-hash", "--background-hash-limit", "1", capture=True)
        assert result.returncode == 0
        combined = result.stdout + result.stderr
        assert "Processed 1 file(s), 0 failed." in combined

        pending_after = [f for f in vault.db_files() if f["sha256"] is None]
        assert len(pending_after) == 1

    def test_background_hash_nice_does_not_fail(self, vault: VaultEnv) -> None:
        """background-hash --nice should run without errors."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        result = vault.run("verify", "--background-hash", "--background-hash-nice", capture=True)
        assert result.returncode == 0
        combined = result.stdout + result.stderr
        assert "Processed 1 file(s), 0 failed." in combined
