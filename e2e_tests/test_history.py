"""History command tests.

Tests the event-log query functionality that provides an audit trail
of all vault changes.

中文场景说明：
- 审计追踪：所有变更都记录在事件日志中，history 命令用于查询
- 过滤：支持按时间、事件类型、文件路径过滤
- 输出格式：支持人类可读表格和 JSON
"""

from __future__ import annotations

import json
from datetime import datetime, timedelta, timezone

import pytest

from conftest import VaultEnv, copy_fixture


class TestHistoryBasic:
    """Basic history query tests."""

    def test_history_shows_import_events(self, vault: VaultEnv) -> None:
        """History should list file.imported events after import."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        result = vault.run("history", capture=True)
        assert result.returncode == 0
        assert "file.imported" in result.stdout

    def test_history_json_output(self, vault: VaultEnv) -> None:
        """History --output=json should return valid JSON."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        result = vault.run("history", "--output=json", capture=True)
        assert result.returncode == 0
        data = json.loads(result.stdout)
        assert "events" in data
        assert len(data["events"]) >= 1
        event = data["events"][0]
        assert "seq" in event
        assert "occurred_at" in event
        assert "event_type" in event
        assert "payload" in event


class TestHistoryFilters:
    """History filtering tests."""

    def test_history_filter_by_event_type(self, vault: VaultEnv) -> None:
        """History --event-type should only return matching events."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        result = vault.run(
            "history", "--event-type", "file.imported", "--output=json", capture=True
        )
        assert result.returncode == 0
        data = json.loads(result.stdout)
        for event in data["events"]:
            assert event["event_type"] == "file.imported"

    def test_history_filter_by_file_path(self, vault: VaultEnv) -> None:
        """History --file should only return events for that file."""
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "samsung_photo.jpg")
        vault.import_dir(vault.source_dir)

        files = vault.db_files()
        target_path = files[0]["path"]

        result = vault.run(
            "history", "--file", target_path, "--output=json", capture=True
        )
        assert result.returncode == 0
        data = json.loads(result.stdout)
        assert len(data["events"]) >= 1
        for event in data["events"]:
            assert target_path in event["payload"]

    def test_history_limit(self, vault: VaultEnv) -> None:
        """History --limit should restrict the number of returned events."""
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "samsung_photo.jpg")
        vault.import_dir(vault.source_dir)

        result = vault.run("history", "--limit", "1", "--output=json", capture=True)
        assert result.returncode == 0
        data = json.loads(result.stdout)
        assert len(data["events"]) <= 1

    def test_history_filter_by_date_range(self, vault: VaultEnv) -> None:
        """History --from and --to should filter by time range."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        yesterday = (datetime.now(timezone.utc) - timedelta(days=1)).strftime("%Y-%m-%d")
        tomorrow = (datetime.now(timezone.utc) + timedelta(days=1)).strftime("%Y-%m-%d")

        result = vault.run(
            "history",
            "--from", yesterday,
            "--to", tomorrow,
            "--output=json",
            capture=True,
        )
        assert result.returncode == 0
        data = json.loads(result.stdout)
        assert len(data["events"]) >= 1

    def test_history_future_range_returns_empty(self, vault: VaultEnv) -> None:
        """History with a future date range should return no events."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        future = (datetime.now(timezone.utc) + timedelta(days=7)).strftime("%Y-%m-%d")
        far_future = (datetime.now(timezone.utc) + timedelta(days=14)).strftime("%Y-%m-%d")

        result = vault.run(
            "history",
            "--from", future,
            "--to", far_future,
            capture=True,
        )
        assert result.returncode == 0
        assert "No events found" in result.stdout or "events" in result.stdout
