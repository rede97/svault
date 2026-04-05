"""Svault FUSE 故障注入测试包

本包提供基于 FUSE 的深度故障注入测试能力，用于验证 svault 在极端 IO 
场景下的健壮性。

主要模块：
- fault_inject_fs: 故障注入 FUSE 文件系统实现
- conftest: FUSE 专用 pytest fixtures
- test_*_fuse.py: 各场景的 FUSE 测试

使用示例：
    cd tests/e2e/fuse_tests && ./run_fuse.sh -v
"""

__version__ = "0.1.0"
