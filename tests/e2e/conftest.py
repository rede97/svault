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

# Check optional tool availability
def _check_tool(tool: str) -> bool:
    """Check if a command-line tool is available."""
    return shutil.which(tool) is not None

# Optional tool availability
FFMPEG_AVAILABLE = _check_tool("ffmpeg")
EXIFTOOL_AVAILABLE = _check_tool("exiftool")

# pytest fixtures for optional tools
@pytest.fixture(scope="session")
def ffmpeg_available() -> bool:
    """Check if ffmpeg is installed and available."""
    return FFMPEG_AVAILABLE

@pytest.fixture(scope="session")
def exiftool_available() -> bool:
    """Check if exiftool is installed and available."""
    return EXIFTOOL_AVAILABLE


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
    parser.addoption(
        "--test-dir",
        action="store",
        default=None,
        help="Custom test directory (instead of RAMDisk). Use this to test on specific filesystems. "
             "When specified, tests will use this directory instead of mounting a RAMDisk.",
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


@pytest.fixture(scope="session")
def custom_test_dir(request: pytest.FixtureRequest) -> Path | None:
    """Get custom test directory from command line option."""
    test_dir = request.config.getoption("--test-dir")
    return Path(test_dir) if test_dir else None


# =============================================================================
# Path Configuration
# =============================================================================

E2E_DIR = Path(__file__).parent.resolve()
PROJECT_ROOT = E2E_DIR.parent.parent  # tests/e2e/ -> tests/ -> project root
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
        input: str | None = None,
    ) -> subprocess.CompletedProcess[str]:
        """Run svault command.
        
        Args:
            *args: Command arguments (e.g., "status", "import", "--yes", "/path")
            check: If True, raise CalledProcessError on non-zero exit
            capture: If True, capture stdout/stderr
            cwd: Working directory (defaults to vault_dir)
            input: String to pass to stdin (for piping)
        
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
        if input is not None:
            kwargs["input"] = input
        
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
        hash: str | None = None,  # Deprecated: now uses config default_hash only
        strategy: str | None = None,
        force: bool = False,
        full_id: bool = False,
    ) -> subprocess.CompletedProcess[str]:
        """Import a directory into the vault.
        
        Note: Uses absolute path to work around a bug in svault's walk function
        which returns absolute paths instead of relative paths.
        
        Note: hash parameter is deprecated and ignored. Use config file instead.
        """
        args = []
        if output_json:
            args.append("--output=json")
        args.append("import")
        if yes:
            args.append("--yes")
        # hash parameter is deprecated - now uses config default_hash only
        if strategy:
            args.extend(["--strategy", strategy])
        if force:
            args.append("--force")
        if full_id:
            args.append("--full-id")
        # Use absolute path to avoid path duplication bug
        source_path = Path(source).resolve()
        args.append(str(source_path))
        
        return self.run(*args, check=check)
    
    def status(self, check: bool = True) -> subprocess.CompletedProcess[str]:
        """Get vault status."""
        return self.run("status", check=check)
    
    def set_hash_algorithm(self, algo: str) -> None:
        """Set the hash algorithm in svault.toml config.
        
        Args:
            algo: Hash algorithm ("xxh3_128", "sha256", etc.)
        """
        config_path = self.vault_dir / "svault.toml"
        if config_path.exists():
            content = config_path.read_text()
            # Replace existing hash setting
            if 'hash = ' in content:
                import re
                content = re.sub(r'hash = "[^"]*"', f'hash = "{algo}"', content)
            else:
                # Add to [global] section
                content = content.replace(
                    "[global]",
                    f'[global]\nhash = "{algo}"'
                )
            config_path.write_text(content)
    
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
            "SELECT path, size, mtime, crc32c, xxh3_128, sha256, status, imported_at FROM files"
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
def exiftool_available() -> bool:
    """Check if exiftool is installed and available in PATH.
    
    Tests that require EXIF manipulation can use this to skip if not available.
    """
    return shutil.which("exiftool") is not None


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
        cmd.extend(["-p", "svault", "-q"])
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
    custom_test_dir: Path | None,
) -> Generator[RamDisk, None, None]:
    """Provide a test directory for test isolation.
    
    Priority:
    1. If --test-dir is specified, use that directory directly (no RAMDisk mount)
    2. Otherwise, use RAMDisk (mount tmpfs)
    3. On Windows, always use a regular temp directory
    4. Falls back to regular temp directory if RAMDisk mounting fails
    
    This allows testing on specific filesystems by using --test-dir=/path/to/ext4
    """
    # If custom test directory is specified, use it directly
    if custom_test_dir:
        custom_test_dir.mkdir(parents=True, exist_ok=True)
        rd = RamDisk(custom_test_dir, size=ramdisk_size)
        rd._mounted = True  # Mark as mounted (it's a real directory)
        yield rd
        # Note: We never cleanup custom test directories
        return
    
    # On Windows, always use temp directory (no RAMDisk)
    if IS_WINDOWS:
        rd = RamDisk(None, size=ramdisk_size)
        rd.mount()
        yield rd
        if cleanup_after_tests:
            rd.cleanup()
        return
        
    # Use configured path and size (RAMDisk)
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
def test_dir(ramdisk: RamDisk) -> Path:
    """Provide the test base directory.
    
    This is a simple wrapper around ramdisk.path to hide the RamDisk
    implementation detail from test code. Tests should use this when
    they just need a directory path, not RamDisk-specific features.
    
    Returns:
        Path to the test directory (either RAMDisk or user-specified via --test-dir)
    """
    return ramdisk.path


@pytest.fixture(scope="function")
def vault(ramdisk: RamDisk, svault_binary: Path) -> Generator[VaultEnv, None, None]:
    """Provide an initialized vault environment.
    
    The vault is created in the test directory, ensuring isolation.
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
    
    Returns a function that creates media files with specified properties.
    Supports JPEG, PNG, TIFF, MP4, MOV, HEIC, and RAW formats.
    
    Example:
        def test_something(source_factory, vault):
            source_factory("test.jpg", exif_date="2024:05:01 10:30:00")
            source_factory("test.png")
            source_factory("test.mp4")
            vault.import_dir(vault.source_dir)
    """
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
            filename: Name of the file to create (extension determines format)
            content: Raw file content (if None, creates appropriate format)
            exif_date: EXIF DateTimeOriginal for image files (format: "2024:05:01 10:30:00")
            exif_make: Camera make for EXIF
            exif_model: Camera model for EXIF
            mtime: Modification time (Unix timestamp)
            subdir: Optional subdirectory under source_dir
        
        Returns:
            Path to created file
        
        Note:
            - Format is auto-detected from file extension
            - If exiftool is not installed, EXIF data will be silently skipped
            - Tests requiring EXIF should check for exiftool availability
        """
        target_dir = vault.source_dir
        if subdir:
            target_dir = target_dir / subdir
            target_dir.mkdir(parents=True, exist_ok=True)
        
        filepath = target_dir / filename
        
        if content is not None:
            # Use provided raw content
            filepath.write_bytes(content)
        else:
            # Auto-detect format from extension
            ext = filepath.suffix.lower().lstrip('.')
            
            # Use content marker for uniqueness
            content_marker = f"test_{filename}"
            
            # Create media file based on extension
            try:
                create_media_file(filepath, ext, content_marker)
                
                # Add EXIF for supported image formats
                if exif_date or exif_make or exif_model:
                    if ext in ('jpg', 'jpeg', 'tiff', 'tif') and shutil.which("exiftool"):
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
            except ValueError:
                # Unknown extension, create as generic file
                filepath.write_bytes(b"test content")
        
        # Set mtime if requested
        if mtime is not None:
            os.utime(filepath, (mtime, mtime))
        
        return filepath
    
    return _create


# =============================================================================
# Helper Functions - Media File Creation
# =============================================================================

def create_minimal_jpeg(path: Path, content_marker: str = "") -> None:
    """Create a minimal valid JPEG file.
    
    Args:
        path: Output file path
        content_marker: String to embed in image data for uniqueness
    
    Note:
        Uses PIL if available, otherwise falls back to a basic JPEG structure
        that is compatible with exiftool.
    """
    try:
        from PIL import Image
        # Create a small valid JPEG that exiftool can process
        img = Image.new('RGB', (16, 16), color=(128, 128, 128))
        img.save(path, format='JPEG', quality=85)
        
        # Append content marker for uniqueness if needed
        if content_marker:
            with open(path, 'ab') as f:
                f.write(b'\n' + content_marker.encode())
    except ImportError:
        # Fallback: Create a minimal but valid JPEG structure
        # This is a 1x1 pixel gray JPEG with proper segment structure
        # Structure: SOI + APP0 (JFIF) + DQT + SOF0 + DHT + SOS + EOI
        import io
        
        # Build a minimal valid JPEG
        data = io.BytesIO()
        
        # SOI (Start of Image)
        data.write(b'\xff\xd8')
        
        # APP0 (JFIF marker) - properly formatted
        app0 = b'JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
        data.write(b'\xff\xe0')
        data.write((len(app0) + 2).to_bytes(2, 'big'))
        data.write(app0)
        
        # DQT (Define Quantization Table) - minimal
        dqt = bytes([0] + [16] * 64)  # Table 0, all values = 16
        data.write(b'\xff\xdb')
        data.write((len(dqt) + 2).to_bytes(2, 'big'))
        data.write(dqt)
        
        # SOF0 (Start of Frame - Baseline DCT)
        sof = bytes([8, 0, 1, 0, 1, 1, 0x11, 0])  # 8-bit, 1x1 pixel, 1 component
        data.write(b'\xff\xc0')
        data.write((len(sof) + 2).to_bytes(2, 'big'))
        data.write(sof)
        
        # DHT (Define Huffman Table) - minimal DC table
        dht = bytes([0x00] + [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 
                     0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])
        data.write(b'\xff\xc4')
        data.write((len(dht) + 2).to_bytes(2, 'big'))
        data.write(dht)
        
        # SOS (Start of Scan)
        sos = bytes([1, 1, 0, 0, 0x3f, 0])  # 1 component, component 1, table 0
        data.write(b'\xff\xda')
        data.write((len(sos) + 2).to_bytes(2, 'big'))
        data.write(sos)
        
        # Minimal scan data (1 MCU)
        data.write(b'\x00')
        
        # EOI (End of Image)
        data.write(b'\xff\xd9')
        
        path.write_bytes(data.getvalue())


def create_minimal_png(path: Path, content_marker: str = "") -> None:
    """Create a minimal valid PNG file.
    
    Args:
        path: Output file path
        content_marker: String to embed in image data for uniqueness
    """
    try:
        from PIL import Image
        # Create a 1x1 RGB image
        img = Image.new('RGB', (1, 1), color=(128, 64, 32))
        img.save(path, format='PNG')
        
        # If content marker provided, append it to make file unique
        if content_marker:
            with open(path, 'ab') as f:
                f.write(content_marker.encode())
    except ImportError:
        # Fallback: Create minimal PNG signature + IHDR chunk
        # PNG signature
        png_sig = b'\x89PNG\r\n\x1a\n'
        # Minimal IHDR chunk (1x1, 8-bit RGB)
        ihdr_data = b'\x00\x00\x00\x01\x00\x00\x00\x01\x08\x02\x00\x00\x00\x90wS\xde'
        ihdr_chunk = b'\x00\x00\x00\x0dIHDR' + ihdr_data
        # IDAT chunk (minimal compressed data)
        idat_data = b'\x78\x9c\x62\xf8\x0f\x00\x00\x01\x01\x00\x05'
        idat_chunk = b'\x00\x00\x00\x0bIDAT' + idat_data + b'\x18\xd7N\xfc'
        # IEND chunk
        iend_chunk = b'\x00\x00\x00\x00IEND\xaeB`\x82'
        
        marker_bytes = content_marker.encode() if content_marker else b''
        path.write_bytes(png_sig + ihdr_chunk + idat_chunk + iend_chunk + marker_bytes)


def create_minimal_tiff(path: Path, content_marker: str = "") -> None:
    """Create a minimal valid TIFF file.
    
    Args:
        path: Output file path
        content_marker: String to embed in image data for uniqueness
    """
    try:
        from PIL import Image
        img = Image.new('RGB', (1, 1), color=(64, 128, 192))
        img.save(path, format='TIFF')
        
        if content_marker:
            with open(path, 'ab') as f:
                f.write(content_marker.encode())
    except ImportError:
        # Fallback: Create minimal TIFF header
        # TIFF little-endian magic + minimal IFD
        tiff_header = b'II\x2a\x00\x08\x00\x00\x00'  # Magic + IFD offset
        # Minimal IFD with 0 entries + next IFD pointer (0)
        ifd = b'\x00\x00\x00\x00\x00\x00\x00\x00'
        marker_bytes = content_marker.encode() if content_marker else b''
        path.write_bytes(tiff_header + ifd + marker_bytes)


def create_minimal_mp4(path: Path, content_marker: str = "") -> None:
    """Create a minimal valid MP4/MOV file.
    
    Creates a minimal MP4 file structure that can be parsed by media tools.
    
    Args:
        path: Output file path
        content_marker: String to embed in file for uniqueness
    """
    try:
        # Try to use ffmpeg if available for proper MP4 creation
        import shutil
        if shutil.which("ffmpeg"):
            # Create a 1-second black video using ffmpeg
            cmd = [
                "ffmpeg", "-y",
                "-f", "lavfi",
                "-i", "color=c=black:s=2x2:d=1",
                "-pix_fmt", "yuv420p",
                "-c:v", "libx264",
                "-preset", "ultrafast",
                "-crf", "51",  # Lowest quality = smallest size
                str(path)
            ]
            result = subprocess.run(cmd, capture_output=True, check=False)
            if result.returncode == 0 and path.exists():
                if content_marker:
                    with open(path, 'ab') as f:
                        f.write(content_marker.encode())
                return
    except Exception:
        pass
    
    # Fallback: Create minimal MP4 structure (may not be playable but recognizable)
    # ftyp box
    ftyp = b'\x00\x00\x00\x20ftypisom\x00\x00\x00\x00isommp41\x00\x00\x00\x00'
    # moov box (minimal movie header)
    moov = b'\x00\x00\x00\x08moov'
    # mdat box (empty media data)
    mdat = b'\x00\x00\x00\x08mdat'
    
    marker_bytes = content_marker.encode() if content_marker else b''
    path.write_bytes(ftyp + moov + mdat + marker_bytes)


def create_minimal_mov(path: Path, content_marker: str = "") -> None:
    """Create a minimal MOV (QuickTime) file.
    
    Args:
        path: Output file path
        content_marker: String to embed in file for uniqueness
    """
    # MOV uses similar structure to MP4 but with different brand
    # ftyp box with qt brand
    ftyp = b'\x00\x00\x00\x14ftypqt\x20\x20\x00\x00\x00\x00qt\x20\x20'
    # moov box
    moov = b'\x00\x00\x00\x08moov'
    # mdat box
    mdat = b'\x00\x00\x00\x08mdat'
    
    marker_bytes = content_marker.encode() if content_marker else b''
    path.write_bytes(ftyp + moov + mdat + marker_bytes)


def create_minimal_heic(path: Path, content_marker: str = "") -> None:
    """Create a minimal HEIC/HEIF file.
    
    Args:
        path: Output file path
        content_marker: String to embed in file for uniqueness
    """
    # HEIC uses ISO Base Media File Format (similar to MP4)
    # ftyp box with heic brand
    ftyp = b'\x00\x00\x00\x18ftypheic\x00\x00\x00\x00heicmif1\x00\x00\x00\x00'
    # meta box (minimal)
    meta = b'\x00\x00\x00\x08meta'
    
    marker_bytes = content_marker.encode() if content_marker else b''
    path.write_bytes(ftyp + meta + marker_bytes)


def create_minimal_raw(path: Path, content_marker: str = "") -> None:
    """Create a minimal RAW image file (DNG format).
    
    Args:
        path: Output file path
        content_marker: String to embed in file for uniqueness
    """
    # DNG is a TIFF variant
    # TIFF header (little-endian) + DNG magic
    tiff_header = b'II\x2a\x00\x08\x00\x00\x00'
    # Minimal IFD
    ifd = b'\x00\x00\x00\x00\x00\x00\x00\x00'
    # DNG version tag data
    dng_magic = b'DNG\x00'
    
    marker_bytes = content_marker.encode() if content_marker else b''
    path.write_bytes(tiff_header + ifd + dng_magic + marker_bytes)


def create_dng_with_exif(
    path: Path, 
    content_marker: str = "",
    body_serial: str = "",
    image_unique_id: str = ""
) -> None:
    """Create a real minimal DNG file with EXIF metadata for RAW ID testing.
    
    Creates a proper TIFF-based DNG with:
    - Complete TIFF header and IFD structure
    - 8x8 grayscale image data
    - EXIF sub-IFD with BodySerialNumber and ImageUniqueID
    
    This is a REAL DNG file, not a fake. It uses proper TIFF/DNG structure
    that both exiftool and Rust's exif crate can read.
    
    Args:
        path: Output file path (should have .dng extension)
        content_marker: String to embed for uniqueness (unused, kept for API compat)
        body_serial: Camera body serial number (BodySerialNumber tag, 0xA431)
        image_unique_id: Image unique ID (ImageUniqueID tag, 0xA420)
    """
    import struct
    
    # Build DNG file sections
    sections = []
    
    # TIFF Header (8 bytes): II + 42 + IFD offset
    header = b'II' + struct.pack('<H', 42) + struct.pack('<I', 8)
    sections.append(header)
    
    # Image specs
    width, height = 8, 8
    bits_per_sample = 8
    compression = 1  # Uncompressed
    photometric = 1  # BlackIsZero
    samples_per_pixel = 1
    image_data_size = width * height  # 64 bytes
    
    # Calculate offsets
    main_ifd_size = 2 + 11 * 12 + 4  # count + 11 entries + next pointer
    strip_offset = 8 + main_ifd_size
    exif_ifd_offset = strip_offset + image_data_size
    
    # Main IFD entries (sorted by tag number as per TIFF spec)
    main_entries = []
    def add_entry(tag, type_id, count, value):
        main_entries.append((tag, struct.pack('<HHII', tag, type_id, count, value)))
    
    add_entry(256, 3, 1, width)                    # ImageWidth
    add_entry(257, 3, 1, height)                   # ImageLength
    add_entry(258, 3, 1, bits_per_sample << 8)     # BitsPerSample (inline)
    add_entry(259, 3, 1, compression)              # Compression
    add_entry(262, 3, 1, photometric)              # PhotometricInterpretation
    add_entry(273, 4, 1, strip_offset)             # StripOffsets
    add_entry(277, 3, 1, samples_per_pixel)        # SamplesPerPixel
    add_entry(278, 3, 1, height)                   # RowsPerStrip
    add_entry(279, 4, 1, image_data_size)          # StripByteCounts
    add_entry(34665, 4, 1, exif_ifd_offset)        # ExifIFDPointer
    add_entry(50706, 1, 4, 0x00000401)             # DNGVersion (1.4.0.0)
    
    # Sort by tag number
    main_entries.sort(key=lambda x: x[0])
    
    # Build main IFD
    main_ifd = struct.pack('<H', len(main_entries))
    for _, entry_data in main_entries:
        main_ifd += entry_data
    main_ifd += struct.pack('<I', 0)  # No next IFD
    sections.append(main_ifd)
    
    # Image data (64 gray pixels)
    # Use content_marker to make each file unique if needed
    if content_marker:
        base_value = sum(ord(c) for c in content_marker) % 256
    else:
        base_value = 128
    image_data = bytes([base_value] * image_data_size)
    sections.append(image_data)
    
    # EXIF IFD
    exif_entries = []
    exif_data_offset = exif_ifd_offset + 2 + 2 * 12 + 4  # After EXIF IFD
    
    def add_exif_entry(tag, data_bytes):
        if len(data_bytes) <= 4:
            # Fit inline with padding
            value = struct.unpack('<I', data_bytes.ljust(4, b'\x00'))[0]
            exif_entries.append((tag, struct.pack('<HHII', tag, 2, len(data_bytes), value)))
        else:
            # Store offset
            nonlocal exif_data_offset
            exif_entries.append((tag, struct.pack('<HHII', tag, 2, len(data_bytes), exif_data_offset)))
            exif_data_offset += len(data_bytes)
            return data_bytes
        return b''
    
    exif_data = b''
    
    # ImageUniqueID tag = 0xA420 (42016)
    if image_unique_id:
        id_bytes = image_unique_id.encode('utf-8') + b'\x00'
        exif_data += add_exif_entry(0xA420, id_bytes)
    
    # BodySerialNumber tag = 0xA431 (42033)
    if body_serial:
        serial_bytes = body_serial.encode('utf-8') + b'\x00'
        exif_data += add_exif_entry(0xA431, serial_bytes)
    
    # Sort EXIF entries by tag
    exif_entries.sort(key=lambda x: x[0])
    
    # Build EXIF IFD
    exif_ifd = struct.pack('<H', len(exif_entries))
    for _, entry_data in exif_entries:
        exif_ifd += entry_data
    exif_ifd += struct.pack('<I', 0)  # No next IFD
    
    sections.append(exif_ifd)
    sections.append(exif_data)
    
    # Write file
    with open(path, 'wb') as f:
        for section in sections:
            f.write(section)


def create_media_file(
    path: Path,
    format: str,
    content_marker: str = "",
    **kwargs
) -> None:
    """Create a media file of specified format.
    
    Unified interface for creating test media files in various formats.
    
    Args:
        path: Output file path
        format: File format - 'jpeg', 'jpg', 'png', 'tiff', 'tif', 'mp4', 'mov', 
                'heic', 'heif', 'dng', 'raw'
        content_marker: String to embed for uniqueness
        **kwargs: Additional format-specific options
        
    Raises:
        ValueError: If format is not supported
    """
    format_lower = format.lower()
    
    creators = {
        'jpeg': create_minimal_jpeg,
        'jpg': create_minimal_jpeg,
        'png': create_minimal_png,
        'tiff': create_minimal_tiff,
        'tif': create_minimal_tiff,
        'mp4': create_minimal_mp4,
        'mov': create_minimal_mov,
        'heic': create_minimal_heic,
        'heif': create_minimal_heic,
        'dng': create_minimal_raw,
        'raw': create_minimal_raw,
    }
    
    if format_lower not in creators:
        raise ValueError(f"Unsupported format: {format}. "
                        f"Supported: {list(creators.keys())}")
    
    creators[format_lower](path, content_marker)


def copy_fixture(vault: VaultEnv, fixture_name: str, subdir: str | None = None) -> Path:
    """Copy a fixture file to the source directory.
    
    Args:
        vault: Vault environment
        fixture_name: Name of fixture in tests/e2e/fixtures/source/
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
