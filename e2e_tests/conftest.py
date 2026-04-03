"""Pytest configuration and fixtures for Svault E2E tests.

This module provides:
    - RAMDisk management for test isolation
    - Vault environment setup/teardown
    - Test data generation utilities
    - Helper functions for common assertions

中文说明：
本模块提供测试基础设施，确保每个测试在隔离环境中运行：
- RAMDisk：使用内存文件系统，避免污染项目目录
- VaultEnv：封装 vault 操作（init, import, db_query）
- source_factory：快速生成带 EXIF 的测试图片
- 测试隔离：每个测试使用独立的 vault 和 source 目录

必要性：
- 隔离性：测试互不干扰，可并行运行
- 可重复性：每次测试从干净状态开始
- 性能：RAMDisk 比磁盘快，加速测试
- 安全性：测试中的 bug 不会删除真实数据
"""

from __future__ import annotations

import json
import os
import platform
import shutil
import sqlite3
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Generator

import pytest

# Check if running on Windows
IS_WINDOWS = platform.system() == "Windows"


# =============================================================================
# Pytest Configuration
# =============================================================================

def pytest_addoption(parser: pytest.Parser) -> None:
    """Add custom command-line options."""
    parser.addoption(
        "--ramdisk-size",
        action="store",
        default="256m",
        help="RAMDisk size (e.g., 128m, 512m, 1g). Default: 256m",
    )
    parser.addoption(
        "--release",
        action="store_true",
        default=False,
        help="Use release build of svault instead of debug",
    )
    parser.addoption(
        "--ramdisk-path",
        action="store",
        default="/tmp/svault-ramdisk",
        help="RAMDisk mount path. Default: /tmp/svault-ramdisk",
    )
    parser.addoption(
        "--cleanup",
        action="store_true",
        default=False,
        help="Cleanup RAMDisk after tests (default: keep for inspection)",
    )


@pytest.fixture(scope="session")
def ramdisk_size(request: pytest.FixtureRequest) -> str:
    """Get RAMDisk size from command line option."""
    return request.config.getoption("--ramdisk-size")


@pytest.fixture(scope="session")
def ramdisk_path(request: pytest.FixtureRequest) -> Path:
    """Get RAMDisk path from command line option."""
    return Path(request.config.getoption("--ramdisk-path"))


@pytest.fixture(scope="session")
def cleanup_after_tests(request: pytest.FixtureRequest) -> bool:
    """Get cleanup flag from command line option."""
    return request.config.getoption("--cleanup")


# =============================================================================
# Path Configuration
# =============================================================================

E2E_DIR = Path(__file__).parent.resolve()
PROJECT_ROOT = E2E_DIR.parent
TESTS_DIR = PROJECT_ROOT / "tests"
FIXTURES_DIR = E2E_DIR / "fixtures"
def get_target_dir(release: bool = False) -> Path:
    return PROJECT_ROOT / "target" / ("release" if release else "debug")


# =============================================================================
# RAMDisk Management (uses tests/setup_ramdisk.sh)
# =============================================================================

class RamDisk:
    """Manage a tmpfs RAMDisk for test isolation (Linux/macOS).
    On Windows, uses a regular temp directory.
    
    Reuses the setup_ramdisk.sh script from the main test framework
    to ensure consistency between old and new test systems.
    """
    
    DEFAULT_PATH = Path("/tmp/svault-ramdisk") if not IS_WINDOWS else Path(tempfile.gettempdir()) / "svault-test"
    
    def __init__(self, path: Path | None = None, size: str = "256m") -> None:
        self.path = path or self.DEFAULT_PATH
        self.size = size
        self._mounted = False
        self._setup_script = PROJECT_ROOT / "tests" / "setup_ramdisk.sh"
        self._is_windows = IS_WINDOWS
    
    def mount(self) -> None:
        """Mount the RAMDisk using setup_ramdisk.sh or direct mount.
        On Windows, creates a temp directory instead."""
        if self._is_windows:
            # On Windows, just create a temp directory
            self.path = Path(tempfile.mkdtemp(prefix="svault-"))
            self._mounted = True
            return
            
        if self._is_mounted():
            self._mounted = True
            return
        
        # Use direct mount to support custom size
        # setup_ramdisk.sh uses fixed 128m size
        self._mount_direct()
        self._mounted = True
    
    def _mount_direct(self) -> None:
        """Direct mount (fallback if setup script missing)."""
        self.path.mkdir(parents=True, exist_ok=True)
        
        cmd = ["mount", "-t", "tmpfs", "-o", f"size={self.size}", "tmpfs", str(self.path)]
        try:
            subprocess.run(cmd, check=True, capture_output=True)
        except subprocess.CalledProcessError:
            subprocess.run(["sudo"] + cmd, check=True)
        
        uid, gid = os.getuid(), os.getgid()
        subprocess.run(["sudo", "chown", f"{uid}:{gid}", str(self.path)], check=False)
    
    def umount(self) -> None:
        """Unmount the RAMDisk."""
        if not self._mounted or self._is_windows:
            return
        
        # Don't unmount if using shared RAMDisk - let setup_ramdisk.sh manage it
        # This allows test parallelization and inspection after tests
        pass
    
    def cleanup(self) -> None:
        """Force unmount - use only when necessary."""
        if self._is_windows:
            # On Windows, remove the temp directory
            if self.path.exists():
                shutil.rmtree(self.path, ignore_errors=True)
            self._mounted = False
            return
            
        if self._is_mounted():
            try:
                subprocess.run(["umount", str(self.path)], check=True, capture_output=True)
            except subprocess.CalledProcessError:
                subprocess.run(["sudo", "umount", str(self.path)], check=False)
        self._mounted = False
    
    def _is_mounted(self) -> bool:
        """Check if the path is a mountpoint."""
        if self._is_windows:
            return self._mounted
        result = subprocess.run(
            ["mountpoint", "-q", str(self.path)],
            capture_output=True
        )
        return result.returncode == 0
    
    def __enter__(self) -> RamDisk:
        self.mount()
        return self
    
    def __exit__(self, *args: Any) -> None:
        # Don't auto-umount to allow inspection and reuse
        pass


# =============================================================================
# Vault Environment
# =============================================================================

@dataclass
class VaultEnv:
    """Encapsulates a test vault environment.
    
    Attributes:
        root: Root directory of the test environment (usually RAMDisk)
        binary: Path to the svault binary
        vault_dir: Directory where vault is initialized (.svault/ lives here)
        source_dir: Directory containing source files to import
        output_dir: Directory for test outputs (logs, reports)
    """
    root: Path
    binary: Path
    vault_dir: Path
    source_dir: Path
    output_dir: Path
    
    def run(
        self,
        *args: str,
        check: bool = True,
        capture: bool = True,
        cwd: Path | None = None,
    ) -> subprocess.CompletedProcess[str]:
        """Run svault command.
        
        Args:
            *args: Command arguments (e.g., "status", "import", "--yes", "/path")
            check: If True, raise CalledProcessError on non-zero exit
            capture: If True, capture stdout/stderr
            cwd: Working directory (defaults to vault_dir)
        
        Returns:
            CompletedProcess with stdout, stderr, returncode
        """
        cmd = [str(self.binary)] + list(args)
        kwargs: dict[str, Any] = {
            "check": check,
            "text": True,
            "cwd": str(cwd or self.vault_dir),
        }
        if capture:
            kwargs["capture_output"] = True
        
        # On Windows, set encoding to handle console output properly
        if IS_WINDOWS:
            kwargs["encoding"] = "utf-8"
            kwargs["errors"] = "replace"
        
        return subprocess.run(cmd, **kwargs)
    
    def init(self, check: bool = True) -> subprocess.CompletedProcess[str]:
        """Initialize a new vault in vault_dir."""
        # Clean up any existing vault first
        svault_meta = self.vault_dir / ".svault"
        if svault_meta.exists():
            shutil.rmtree(svault_meta)
        config = self.vault_dir / "svault.toml"
        if config.exists():
            config.unlink()
        
        return self.run("init", check=check)
    
    def import_dir(
        self,
        source: Path | str,
        yes: bool = True,
        output_json: bool = True,
        check: bool = True,
        hash: str | None = None,
        strategy: str | None = None,
        force: bool = False,
    ) -> subprocess.CompletedProcess[str]:
        """Import a directory into the vault.
        
        Note: Uses absolute path to work around a bug in svault's walk function
        which returns absolute paths instead of relative paths.
        """
        args = []
        if output_json:
            args.append("--output=json")
        args.append("import")
        if yes:
            args.append("--yes")
        if hash:
            args.extend(["-H", hash])
        if strategy:
            args.extend(["--strategy", strategy])
        if force:
            args.append("--force")
        # Use absolute path to avoid path duplication bug
        source_path = Path(source).resolve()
        args.append(str(source_path))
        
        return self.run(*args, check=check)
    
    def status(self, check: bool = True) -> subprocess.CompletedProcess[str]:
        """Get vault status."""
        return self.run("status", check=check)
    
    def db_query(self, query: str) -> list[dict[str, Any]]:
        """Execute SQL query against vault database.
        
        Args:
            query: SQL query string
        
        Returns:
            List of rows as dictionaries
        """
        db_path = self.vault_dir / ".svault" / "vault.db"
        if not db_path.exists():
            return []
        
        conn = sqlite3.connect(str(db_path))
        conn.row_factory = sqlite3.Row
        try:
            cur = conn.execute(query)
            conn.commit()
            rows = [dict(row) for row in cur.fetchall()]
            return rows
        finally:
            conn.close()
    
    def db_files(self) -> list[dict[str, Any]]:
        """Get all rows from the files table."""
        return self.db_query(
            "SELECT path, size, mtime, crc32c_val, xxh3_128, sha256, status, imported_at FROM files"
        )
    
    def find_file_in_db(self, filename: str) -> list[dict[str, Any]]:
        """Find DB rows by filename (basename match)."""
        basename = Path(filename).name
        return [r for r in self.db_files() if Path(r["path"]).name == basename]
    
    def get_vault_files(self, pattern: str = "*") -> list[Path]:
        """Get list of files in vault storage (excluding .svault/)."""
        # Files are stored directly in vault_dir, not in a 'vault' subdirectory
        if not self.vault_dir.exists():
            return []
        # Exclude .svault/ directory
        files = []
        for f in self.vault_dir.rglob(pattern):
            if f.is_file() and ".svault" not in str(f.relative_to(self.vault_dir)):
                files.append(f)
        return files


# =============================================================================
# Pytest Fixtures
# =============================================================================

@pytest.fixture(scope="session")
def svault_binary(request: pytest.FixtureRequest) -> Path:
    """Path to the compiled svault binary.
    
    Builds in debug mode by default; use --release for release builds.
    """
    release = request.config.getoption("--release")
    target_dir = get_target_dir(release)
    binary_name = "svault.exe" if IS_WINDOWS else "svault"
    binary = target_dir / binary_name
    
    if not binary.exists():
        # Build the binary
        cmd = ["cargo", "build"]
        if release:
            cmd.append("--release")
        cmd.extend(["-p", "svault-cli", "-q"])
        result = subprocess.run(
            cmd,
            cwd=PROJECT_ROOT,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            pytest.fail(f"Failed to build svault: {result.stderr}")
    
    if not binary.exists():
        pytest.fail(f"Binary not found at {binary}")
    
    return binary


@pytest.fixture(scope="function")
def ramdisk(
    tmp_path: Path,
    ramdisk_size: str,
    ramdisk_path: Path,
    cleanup_after_tests: bool,
) -> Generator[RamDisk, None, None]:
    """Provide a RAMDisk for test isolation.
    
    Uses command-line options --ramdisk-size and --ramdisk-path.
    On Windows, always uses a regular temp directory.
    Falls back to regular temp directory if mounting fails.
    """
    # On Windows, always use temp directory (no RAMDisk)
    if IS_WINDOWS:
        rd = RamDisk(None, size=ramdisk_size)
        rd.mount()
        yield rd
        if cleanup_after_tests:
            rd.cleanup()
        return
        
    # Use configured path and size
    rd = RamDisk(ramdisk_path, size=ramdisk_size)
    
    try:
        rd.mount()
        yield rd
    except subprocess.CalledProcessError:
        # Fallback to regular temp directory
        mount_point = tmp_path / "ramdisk"
        mount_point.mkdir(parents=True, exist_ok=True)
        rd._mounted = True  # Pretend it's mounted
        rd.path = mount_point
        yield rd
    finally:
        if cleanup_after_tests:
            rd.cleanup()


@pytest.fixture(scope="function")
def vault(ramdisk: RamDisk, svault_binary: Path) -> Generator[VaultEnv, None, None]:
    """Provide an initialized vault environment.
    
    The vault is created in the RAMDisk, ensuring isolation.
    Automatically cleaned up after test.
    """
    import uuid
    
    root = ramdisk.path
    
    # Use unique directory for each test to ensure isolation
    test_id = str(uuid.uuid4())[:8]
    vault_dir = root / f"vault_{test_id}"
    source_dir = root / f"source_{test_id}"
    output_dir = root / f"output_{test_id}"
    
    env = VaultEnv(
        root=root,
        binary=svault_binary,
        vault_dir=vault_dir,
        source_dir=source_dir,
        output_dir=output_dir,
    )
    
    # Create directories
    env.vault_dir.mkdir(parents=True, exist_ok=True)
    env.source_dir.mkdir(parents=True, exist_ok=True)
    env.output_dir.mkdir(parents=True, exist_ok=True)
    
    # Initialize vault
    env.init()
    
    yield env
    
    # Cleanup: remove test directories
    import shutil
    if vault_dir.exists():
        shutil.rmtree(vault_dir)
    if source_dir.exists():
        shutil.rmtree(source_dir)
    if output_dir.exists():
        shutil.rmtree(output_dir)


@pytest.fixture(scope="function")
def source_factory(vault: VaultEnv) -> callable:
    """Factory fixture for creating test source files.
    
    Returns a function that creates JPEG files with specified EXIF data.
    
    Example:
        def test_something(source_factory, vault):
            source_factory("test.jpg", exif_date="2024:05:01 10:30:00")
            vault.import_dir(vault.source_dir)
    """
    try:
        from PIL import Image
        PIL_AVAILABLE = True
    except ImportError:
        PIL_AVAILABLE = False
    
    def _create(
        filename: str,
        content: bytes | None = None,
        exif_date: str | None = None,
        exif_make: str | None = None,
        exif_model: str | None = None,
        mtime: float | None = None,
        subdir: str | None = None,
    ) -> Path:
        """Create a test file in the source directory.
        
        Args:
            filename: Name of the file to create
            content: Raw file content (if None, creates minimal JPEG)
            exif_date: EXIF DateTimeOriginal (format: "2024:05:01 10:30:00")
            exif_make: Camera make
            exif_model: Camera model
            mtime: Modification time (Unix timestamp)
            subdir: Optional subdirectory under source_dir
        
        Returns:
            Path to created file
        """
        target_dir = vault.source_dir
        if subdir:
            target_dir = target_dir / subdir
            target_dir.mkdir(parents=True, exist_ok=True)
        
        filepath = target_dir / filename
        
        if content is not None:
            filepath.write_bytes(content)
        elif PIL_AVAILABLE:
            # Create minimal JPEG
            img = Image.new("RGB", (4, 4), color=(128, 64, 32))
            img.save(filepath, format="JPEG", quality=85)
            
            # Add EXIF if requested using exiftool
            if exif_date or exif_make or exif_model:
                cmd = ["exiftool", "-overwrite_original", "-ignoreMinorErrors"]
                if exif_date:
                    cmd.extend([
                        f"-DateTimeOriginal={exif_date}",
                        f"-DateTime={exif_date}",
                    ])
                if exif_make:
                    cmd.append(f"-Make={exif_make}")
                if exif_model:
                    cmd.append(f"-Model={exif_model}")
                cmd.append(str(filepath))
                
                subprocess.run(cmd, check=False, capture_output=True)
        else:
            # Minimal JPEG header without EXIF
            header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
            filepath.write_bytes(header + b'\x00' * 100)
        
        # Set mtime if requested
        if mtime is not None:
            os.utime(filepath, (mtime, mtime))
        
        return filepath
    
    return _create


# =============================================================================
# Helper Functions
# =============================================================================

def create_minimal_jpeg(path: Path, content_marker: str = "") -> None:
    """Create a minimal valid JPEG file.
    
    Args:
        path: Output file path
        content_marker: String to embed in image data for uniqueness
    """
    header = b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
    marker_bytes = content_marker.encode() if content_marker else b''
    path.write_bytes(header + marker_bytes + b'\xff\xd9')


def copy_fixture(vault: VaultEnv, fixture_name: str, subdir: str | None = None) -> Path:
    """Copy a fixture file to the source directory.
    
    Args:
        vault: Vault environment
        fixture_name: Name of fixture in e2e_tests/fixtures/source/
        subdir: Optional subdirectory
    
    Returns:
        Path to copied file in source_dir
    """
    source = FIXTURES_DIR / "source" / fixture_name
    if not source.exists():
        pytest.fail(f"Fixture not found: {source}")
    
    target_dir = vault.source_dir
    if subdir:
        target_dir = target_dir / subdir
        target_dir.mkdir(parents=True, exist_ok=True)
    
    target = target_dir / Path(fixture_name).name
    shutil.copy2(source, target)
    return target


def assert_file_imported(vault: VaultEnv, filename: str) -> dict[str, Any]:
    """Assert that a file was imported and return its DB row.
    
    Args:
        vault: Vault environment
        filename: Expected filename in vault
    
    Returns:
        DB row dictionary
    
    Raises:
        AssertionError: If file not found or not imported
    """
    rows = vault.find_file_in_db(filename)
    assert len(rows) > 0, f"File {filename} not found in database"
    
    for row in rows:
        assert row.get("status") == "imported", f"File {filename} status is {row.get('status')}"
    
    return rows[0]


def assert_file_duplicate(vault: VaultEnv, filename: str) -> None:
    """Assert that a file was detected as duplicate (not in DB)."""
    rows = vault.find_file_in_db(filename)
    assert len(rows) == 0, f"Duplicate file {filename} should not be in database"


def assert_path_contains(path: str, *substrings: str) -> None:
    """Assert that path contains all expected substrings."""
    for sub in substrings:
        assert sub in path, f"Path '{path}' does not contain '{sub}'"
