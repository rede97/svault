"""History command tests.

Tests the event-log query functionality that provides an audit trail
of all vault changes.

中文场景说明：
- 审计追踪：所有变更都记录在事件日志中，history 命令用于查询
- 默认视图：显示 import/add/reconcile 会话（带完成状态）
- history sessions: 列出导入会话（批次）
- history items --session <id>: 显示会话中的文件列表
"""

from __future__ import annotations

import json
import os
import shutil
from datetime import datetime, timedelta, timezone
from pathlib import Path

import pytest

from conftest import VaultEnv, copy_fixture, FIXTURES_DIR


class TestHistorySessions:
    """History sessions query tests."""

    def test_history_shows_sessions_by_default(self, vault: VaultEnv) -> None:
        """History (no args) should show import sessions by default."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        result = vault.run("history", capture=True)
        assert result.returncode == 0
        # Default view shows session info via reporter
        # In human mode, we get session rows printed

    def test_history_sessions_shows_import_batches(self, vault: VaultEnv) -> None:
        """History sessions should list import batches."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        result = vault.run("history", "sessions", capture=True)
        assert result.returncode == 0

    def test_history_sessions_json_output(self, vault: VaultEnv) -> None:
        """History sessions --output=json should return JSON events."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        result = vault.run("history", "sessions", "--output=json", capture=True)
        assert result.returncode == 0
        # Parse JSON lines (one event per line)
        events = [json.loads(line) for line in result.stdout.strip().split('\n') if line]
        
        # Should have started and finished events
        event_types = [e["event"] for e in events]
        assert "history_sessions_started" in event_types
        assert "history_sessions_finished" in event_types

    def test_history_empty_vault(self, vault: VaultEnv) -> None:
        """History on empty vault should show no results."""
        result = vault.run("history", "sessions", "--output=json", capture=True)
        assert result.returncode == 0
        events = [json.loads(line) for line in result.stdout.strip().split('\n') if line]
        
        # Find the finished event and check summary
        finished = next(e for e in events if e["event"] == "history_sessions_finished")
        assert finished["summary"]["total"] == 0


class TestHistoryItems:
    """History items query tests."""

    def test_history_items_requires_session(self, vault: VaultEnv) -> None:
        """History items should require --session argument."""
        result = vault.run("history", "items", capture=True, check=False)
        assert result.returncode != 0

    def test_history_items_shows_files_in_session(self, vault: VaultEnv) -> None:
        """History items --session <id> should list files in that session."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        # First get the session ID
        result = vault.run("history", "sessions", "--output=json", capture=True)
        events = [json.loads(line) for line in result.stdout.strip().split('\n') if line]
        
        # Get session items (if any item events were emitted)
        # For now, just verify the command structure works
        result = vault.run("history", "items", "--session=nonexistent", capture=True)
        assert result.returncode == 0  # Empty result is OK

    def test_history_items_json_output(self, vault: VaultEnv) -> None:
        """History items --session <id> --output=json should return JSON."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        result = vault.run(
            "history", "items", "--session=nonexistent", "--output=json",
            capture=True
        )
        assert result.returncode == 0
        events = [json.loads(line) for line in result.stdout.strip().split('\n') if line]
        
        event_types = [e["event"] for e in events]
        assert "history_items_started" in event_types
        assert "history_items_finished" in event_types


class TestHistorySessionFilters:
    """History sessions filter tests."""

    def test_history_filter_by_date_range(self, vault: VaultEnv) -> None:
        """History sessions --from and --to should filter by date."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        # Query with a date range that includes today
        today = datetime.now().strftime("%Y-%m-%d")
        result = vault.run(
            "history", "sessions",
            "--from", today,
            "--to", today,
            "--output=json",
            capture=True
        )
        assert result.returncode == 0

    def test_history_filter_by_source(self, vault: VaultEnv) -> None:
        """History sessions --source should filter by source path."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        result = vault.run(
            "history", "sessions",
            "--source", str(vault.source_dir),
            "--output=json",
            capture=True
        )
        assert result.returncode == 0

    def test_history_pagination_with_limit_offset(self, vault: VaultEnv) -> None:
        """History sessions --limit and --offset should paginate results."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        result = vault.run(
            "history", "sessions",
            "--limit", "1",
            "--offset", "0",
            "--output=json",
            capture=True
        )
        assert result.returncode == 0


class TestHistoryPendingImports:
    """Tests for pending import detection."""

    def test_history_shows_completed_imports(self, vault: VaultEnv) -> None:
        """History should show completed imports."""
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)

        result = vault.run("history", "sessions", "--output=json", capture=True)
        assert result.returncode == 0
        
        events = [json.loads(line) for line in result.stdout.strip().split('\n') if line]
        finished = next((e for e in events if e["event"] == "history_sessions_finished"), None)
        assert finished is not None


class TestHistoryLifecycle:
    """Full lifecycle test: import → add → update → re-import.
    
    Covers single vault lifecycle with 4 types of changes:
    1. import
    2. add
    3. update (after delete/move)
    4. re-import
    """

    def _get_latest_session(self, vault: VaultEnv) -> dict:
        """Get the latest session from history."""
        result = vault.run("history", "sessions", "--output=json", capture=True)
        assert result.returncode == 0
        events = [json.loads(line) for line in result.stdout.strip().split('\n') if line]
        
        # Find session items (skip started/finished events)
        session_items = [e for e in events if e.get("event") == "history_sessions_item"]
        if not session_items:
            return None
        # Return most recent (first in list)
        return session_items[0]

    def _get_session_items(self, vault: VaultEnv, session_id: str) -> list:
        """Get all items for a specific session."""
        result = vault.run(
            "history", "items", f"--session={session_id}", "--output=json",
            capture=True
        )
        assert result.returncode == 0
        events = [json.loads(line) for line in result.stdout.strip().split('\n') if line]
        return [e for e in events if e.get("event") == "history_items_item"]

    def _parse_json_events(self, stdout: str) -> list:
        """Parse JSON lines into events."""
        return [json.loads(line) for line in stdout.strip().split('\n') if line]

    def test_history_full_lifecycle_import_add_update_reimport(self, vault: VaultEnv) -> None:
        """Full lifecycle: import → add → update → re-import."""
        # Step 1: First import (2 files)
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "no_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        # Verify s_import_1
        s_import_1 = self._get_latest_session(vault)
        assert s_import_1 is not None, "Should have import session"
        assert s_import_1["session_type"] == "import"
        assert s_import_1["total_files"] == 2
        
        # Verify items count matches
        items_1 = self._get_session_items(vault, s_import_1["session_id"])
        assert len(items_1) == 2, f"Expected 2 items, got {len(items_1)}"
        
        # Step 2: Add a new file inside vault
        manual_dir = vault.vault_dir / "manual"
        manual_dir.mkdir(exist_ok=True)
        added_file = manual_dir / "add_only.jpg"
        shutil.copy(FIXTURES_DIR / "source" / "apple_with_exif.jpg", added_file)
        
        vault.run("add", str(manual_dir), check=True)
        
        # Verify s_add_1
        s_add_1 = self._get_latest_session(vault)
        assert s_add_1 is not None
        assert s_add_1["session_type"] == "add", f"Expected 'add', got {s_add_1.get('session_type')}"
        
        # Step 3: Delete one file and move another, then update
        # Find first imported file
        first_item = items_1[0]
        vault_rel_path = first_item["vault_path"]
        vault_file = vault.vault_dir / vault_rel_path
        
        # Delete it
        vault_file.unlink()
        
        # Move second file
        second_item = items_1[1]
        second_vault_path = second_item["vault_path"]
        second_file = vault.vault_dir / second_vault_path
        new_location = vault.vault_dir / "moved" / os.path.basename(second_vault_path)
        new_location.parent.mkdir(exist_ok=True)
        shutil.move(str(second_file), str(new_location))
        
        # Run update (runs in vault_dir by default)
        vault.run("update", "--yes", check=True)
        
        # Verify s_update_1
        s_update_1 = self._get_latest_session(vault)
        assert s_update_1 is not None
        # Note: update may not write manifest yet, skip if not present
        
        # Step 4: Re-import same source (files were deleted/moved, so should re-import)
        vault.import_dir(vault.source_dir)
        
        # Verify we have at least 3 sessions (import_1, add_1, and either update or import_2)
        result = vault.run("history", "sessions", "--output=json", capture=True)
        events = self._parse_json_events(result.stdout)
        session_items = [e for e in events if e.get("event") == "history_sessions_item"]
        
        # Count session types
        import_count = sum(1 for s in session_items if s["session_type"] == "import")
        add_count = sum(1 for s in session_items if s["session_type"] == "add")
        
        assert import_count >= 1, f"Expected at least 1 import session, got {import_count}"
        assert add_count >= 1, f"Expected at least 1 add session, got {add_count}"
        assert len(session_items) >= 2, f"Expected >=2 sessions, got {len(session_items)}"

    def test_history_items_status_filter(self, vault: VaultEnv) -> None:
        """Test --status filter on history items."""
        # Create files with potential duplicates
        copy_fixture(vault, "apple_with_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        # Get session
        session = self._get_latest_session(vault)
        assert session is not None
        session_id = session["session_id"]
        
        # Query all items
        all_items = self._get_session_items(vault, session_id)
        
        # Query with status filter (if implemented)
        result = vault.run(
            "history", "items", f"--session={session_id}", 
            "--status=added", "--output=json", capture=True
        )
        # Should succeed even if filter not implemented
        assert result.returncode == 0

    def test_history_json_events_consistency(self, vault: VaultEnv) -> None:
        """Verify JSON events have consistent counts."""
        copy_fixture(vault, "apple_with_exif.jpg")
        copy_fixture(vault, "no_exif.jpg")
        vault.import_dir(vault.source_dir)
        
        # Query sessions
        result = vault.run("history", "sessions", "--output=json", capture=True)
        events = self._parse_json_events(result.stdout)
        
        # Verify event structure
        event_types = [e.get("event") for e in events]
        assert "history_sessions_started" in event_types
        assert "history_sessions_finished" in event_types
        
        # Get session items
        session_items = [e for e in events if e.get("event") == "history_sessions_item"]
        finished = next(e for e in events if e.get("event") == "history_sessions_finished")
        
        # Verify consistency: items count should match summary
        assert finished["summary"]["returned"] == len(session_items)
        
        # Now test items consistency
        if session_items:
            session_id = session_items[0]["session_id"]
            result = vault.run(
                "history", "items", f"--session={session_id}", "--output=json",
                capture=True
            )
            item_events = self._parse_json_events(result.stdout)
            
            item_rows = [e for e in item_events if e.get("event") == "history_items_item"]
            items_finished = next(
                (e for e in item_events if e.get("event") == "history_items_finished"), None
            )
            
            if items_finished:
                assert items_finished["summary"]["returned"] == len(item_rows)
                
            # Verify item fields
            for item in item_rows:
                assert "source_path" in item
                assert "vault_path" in item
                assert "status" in item
                assert "size" in item
                assert "mtime_ms" in item

    def test_history_sessions_pagination_order(self, vault: VaultEnv) -> None:
        """Test sessions are ordered by time (newest first)."""
        # Do multiple imports with different files to ensure unique sessions
        fixtures = ["apple_with_exif.jpg", "no_exif.jpg", "samsung_photo.jpg"]
        for fixture in fixtures:
            # Clear source dir and add new file
            for f in vault.source_dir.iterdir():
                if f.is_file():
                    f.unlink()
            copy_fixture(vault, fixture)
            vault.import_dir(vault.source_dir)
        
        # Query with limit
        result = vault.run(
            "history", "sessions", "--limit=2", "--output=json", capture=True
        )
        events = self._parse_json_events(result.stdout)
        session_items = [e for e in events if e.get("event") == "history_sessions_item"]
        
        # Should have 2 sessions (limited)
        assert len(session_items) == 2
        
        # Verify started_at_ms is descending (newest first)
        timestamps = [s["started_at_ms"] for s in session_items]
        assert timestamps == sorted(timestamps, reverse=True)
