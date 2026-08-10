"""Info command tests: EXIF dump, hash lookup, ffprobe integration.

Contracts:
- `info <path>`：DB 记录 + EXIF（图像）/ ffprobe（视频）
- `info --hash <hex|prefix>`：xxh3/sha256 全量或唯一前缀定位
- 人类输出为表格；--output=json 输出结构化报告
"""

from __future__ import annotations

import json
import shutil
import subprocess

import pytest

from conftest import VaultEnv, copy_fixture


def _import_apple(vault: VaultEnv) -> str:
    copy_fixture(vault, "apple_with_exif.jpg")
    assert vault.import_dir(vault.source_dir).returncode == 0
    return next(f["path"] for f in vault.db_files() if "apple" in f["path"])


class TestInfoByPath:
    def test_info_shows_db_facts_and_exif(self, vault: VaultEnv) -> None:
        rel = _import_apple(vault)

        report = json.loads(vault.run("--output=json", "info", rel).stdout)
        assert report["on_disk"] is True
        assert report["db"]["path"] == rel
        assert report["db"]["xxh3_128"]
        assert report["db"]["status"] == "imported"

        tags = {tag for tag, _ in report["exif"]}
        assert "DateTimeOriginal" in tags
        assert "Make" in tags or "Model" in tags

    def test_info_untracked_file(self, vault: VaultEnv) -> None:
        stray = vault.vault_dir / "stray.jpg"
        copy_fixture(vault, "apple_with_exif.jpg")
        shutil.copy(vault.source_dir / "apple_with_exif.jpg", stray)

        report = json.loads(vault.run("--output=json", "info", str(stray)).stdout)
        assert report["on_disk"] is True
        assert report["db"] is None
        assert len(report["exif"]) > 0


class TestInfoByHash:
    def test_full_and_unique_prefix_lookup(self, vault: VaultEnv) -> None:
        rel = _import_apple(vault)
        row = vault.db_files()[0]
        full = row["xxh3_128"].hex()

        by_full = json.loads(vault.run("--output=json", "info", "--hash", full).stdout)
        assert by_full["db"]["path"] == rel

        by_prefix = json.loads(vault.run("--output=json", "info", "--hash", full[:8]).stdout)
        assert by_prefix["db"]["path"] == rel

    def test_unknown_hash_errors(self, vault: VaultEnv) -> None:
        _import_apple(vault)
        result = vault.run("info", "--hash", "deadbeef", check=False)
        assert result.returncode != 0
        assert "no file matches" in (result.stderr + result.stdout)


@pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not installed")
class TestInfoVideo:
    def test_ffprobe_streams_reported(self, vault: VaultEnv) -> None:
        # 用 ffmpeg 生成 0.2s 真实 mp4（测试固件不允许假文件）
        src = vault.source_dir / "clip.mp4"
        subprocess.run(
            [
                "ffmpeg", "-y", "-v", "error",
                "-f", "lavfi", "-i", "color=c=black:duration=0.2:size=64x64:rate=10",
                str(src),
            ],
            check=True,
        )
        assert vault.import_dir(vault.source_dir).returncode == 0
        rel = next(f["path"] for f in vault.db_files() if f["path"].endswith(".mp4"))

        report = json.loads(vault.run("--output=json", "info", rel).stdout)
        assert report["ffprobe_available"] is True
        assert report["video"] is not None
        streams = report["video"]["streams"]
        assert any(s.get("codec_type") == "video" for s in streams)
