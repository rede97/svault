"""FUSE 测试专用 pytest fixtures

继承主测试框架的 fixtures，添加 FUSE 专用支持。
"""

from __future__ import annotations

import os
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any, Callable, Generator

import pytest

# 将父目录加入路径以继承主 fixtures
sys.path.insert(0, str(Path(__file__).parent.parent))
from conftest import VaultEnv, get_target_dir

# 尝试导入 FUSE
try:
    import fuse
    FUSE_AVAILABLE = True
    FUSE_BACKEND = "fusepy"
except ImportError:
    try:
        import pyfuse3
        FUSE_AVAILABLE = True
        FUSE_BACKEND = "pyfuse3"
    except ImportError:
        FUSE_AVAILABLE = False
        FUSE_BACKEND = None


def pytest_configure(config: pytest.Config) -> None:
    """配置 FUSE 测试标记"""
    config.addinivalue_line("markers", "fuse: 需要 FUSE 支持的测试")
    config.addinivalue_line("markers", "slow: 慢速测试（长时间运行）")
    config.addinivalue_line("markers", "interrupt: 中断场景测试")


def pytest_collection_modifyitems(config: pytest.Config, items: list) -> None:
    """如果没有 FUSE 支持，跳过所有 fuse 标记的测试"""
    if not FUSE_AVAILABLE:
        skip_fuse = pytest.mark.skip(reason="FUSE 库未安装 (pip install fusepy)")
        for item in items:
            if "fuse" in item.keywords:
                item.add_marker(skip_fuse)


@pytest.fixture(scope="session")
def fuse_available() -> bool:
    """检查 FUSE 是否可用"""
    return FUSE_AVAILABLE


@pytest.fixture(scope="session")
def fuse_backend() -> str | None:
    """返回 FUSE 后端类型"""
    return FUSE_BACKEND


@pytest.fixture(scope="function")
def fuse_mount_point(tmp_path: Path) -> Path:
    """提供 FUSE 挂载点"""
    mount_point = tmp_path / "fuse_mount"
    mount_point.mkdir(parents=True, exist_ok=True)
    return mount_point


# 延迟导入 FaultInjectedFS，避免 FUSE 未安装时导入错误
@pytest.fixture(scope="function")
def fault_inject_fs_class() -> type | None:
    """提供 FaultInjectedFS 类"""
    if not FUSE_AVAILABLE:
        return None
    
    from fault_inject_fs import FaultInjectedFS
    return FaultInjectedFS


@pytest.fixture(scope="function")
def vault_with_fuse_source(
    vault: VaultEnv,
    fuse_mount_point: Path,
    fault_inject_fs_class: type | None,
) -> Generator[tuple[VaultEnv, Path, Any], None, None]:
    """提供配置了 FUSE 源目录的 vault 环境
    
    Yields:
        (vault_env, fuse_mount_point, fs_instance)
    
    使用示例：
        def test_example(vault_with_fuse_source):
            vault, fuse_mount, fs = vault_with_fuse_source
            # 配置故障注入
            fs.add_read_fault(offset=1024, error=errno.EIO)
            # 执行导入
            vault.import_dir(fuse_mount)
    """
    if not FUSE_AVAILABLE or fault_inject_fs_class is None:
        pytest.skip("FUSE 不可用")
    
    fs = fault_inject_fs_class(
        root=vault.source_dir,
        mount_point=fuse_mount_point,
    )
    
    # 启动 FUSE 文件系统（在后台线程）
    fs_thread = threading.Thread(target=fs.start, daemon=True)
    fs_thread.start()
    
    # 等待挂载完成
    time.sleep(0.5)
    
    try:
        yield vault, fuse_mount_point, fs
    finally:
        # 清理
        fs.stop()
        fs_thread.join(timeout=5)


@pytest.fixture
def slow_write_fs(fault_inject_fs_class: type | None) -> Callable:
    """创建慢速写入 FUSE 文件系统的工厂"""
    if fault_inject_fs_class is None:
        pytest.skip("FUSE 不可用")
    
    def _create(source_dir: Path, delay_ms: float = 100):
        fs = fault_inject_fs_class(
            root=source_dir,
            delay_read=delay_ms,
            delay_write=delay_ms,
        )
        return fs
    return _create


@pytest.fixture
def error_at_offset_fs(fault_inject_fs_class: type | None) -> Callable:
    """创建在特定偏移量注入错误的 FUSE 文件系统工厂"""
    if fault_inject_fs_class is None:
        pytest.skip("FUSE 不可用")
    
    def _create(source_dir: Path, fault_config: list[dict]):
        """
        Args:
            fault_config: 错误配置列表
                [{"offset": 1024, "error": errno.EIO, "when": "read"}, ...]
        """
        fs = fault_inject_fs_class(
            root=source_dir,
            fault_config=fault_config,
        )
        return fs
    return _create


@pytest.fixture
def pause_at_offset_fs(fault_inject_fs_class: type | None) -> Callable:
    """创建在特定偏移量暂停的 FUSE 文件系统工厂"""
    if fault_inject_fs_class is None:
        pytest.skip("FUSE 不可用")
    
    def _create(source_dir: Path, pause_config: list[dict]):
        """
        Args:
            pause_config: 暂停配置列表
                [{"offset": 2048, "event": threading.Event()}, ...]
        """
        fs = fault_inject_fs_class(
            root=source_dir,
            pause_config=pause_config,
        )
        return fs
    return _create
