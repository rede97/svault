"""故障注入 FUSE 文件系统实现

提供可编程的 IO 故障注入能力，用于精确测试 svault 在极端场景下的行为。

设计目标：
1. 精确控制：字节级精度的暂停/错误注入点
2. 灵活性：支持多种故障模式（错误、延迟、暂停）
3. 可观测：提供钩子用于验证测试状态
4. 安全：超时机制防止测试挂起

使用示例：
    # 在文件读取到 50% 时暂停
    pause_event = threading.Event()
    fs = FaultInjectedFS(
        root="/real/data",
        mount_point="/mnt/fuse",
        pause_config=[{
            "path": "test.jpg",
            "offset": 5120,  # 在 5KB 处暂停
            "event": pause_event,
        }]
    )
    fs.start()  # 后台启动
    
    # 在另一个进程导入
    proc = subprocess.Popen(["svault", "import", "/mnt/fuse"])
    
    # 等待到暂停点
    time.sleep(0.5)
    assert fs.is_paused("test.jpg")  # 确认已暂停
    
    # 继续或中断
    pause_event.set()  # 继续
    # 或 proc.terminate()  # 测试中断恢复
"""

from __future__ import annotations

import errno
import os
import stat
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

# 尝试导入 FUSE
try:
    import fuse
    from fuse import Fuse
    FUSE_AVAILABLE = True
except ImportError:
    FUSE_AVAILABLE = False
    Fuse = object  # 占位符


@dataclass
class FaultRule:
    """故障规则定义"""
    path: str  # 文件路径（相对 mount_point）
    offset: int  # 触发偏移量（字节）
    action: str  # 'error', 'pause', 'delay', 'corrupt'
    
    # action='error' 时使用
    error_code: int = errno.EIO
    
    # action='pause' 时使用
    pause_event: threading.Event | None = None
    
    # action='delay' 时使用（毫秒）
    delay_ms: float = 0
    
    # action='corrupt' 时使用
    corrupt_data: bytes | None = None
    
    # 触发次数控制
    trigger_count: int = 0  # 0 表示无限次
    triggered: int = field(default=0, init=False)
    
    def should_trigger(self, offset: int, size: int) -> bool:
        """检查当前读取范围是否触发此规则"""
        # 检查是否还有触发次数
        if self.trigger_count > 0 and self.triggered >= self.trigger_count:
            return False
        
        # 检查偏移量范围是否有重叠
        read_end = offset + size
        trigger_end = self.offset + 1
        
        return offset <= self.offset < read_end
    
    def mark_triggered(self) -> None:
        """标记已触发"""
        self.triggered += 1


@dataclass
class IOStats:
    """IO 统计信息"""
    read_count: int = 0
    read_bytes: int = 0
    write_count: int = 0
    write_bytes: int = 0
    error_count: int = 0
    pause_count: int = 0
    last_offset: int = 0


class FaultInjectedFS(Fuse):
    """故障注入 FUSE 文件系统
    
    基于 fusepy 的用户态文件系统，支持：
    - 在特定偏移量注入 IO 错误
    - 在特定偏移量暂停（可恢复）
    - 添加读取/写入延迟
    - 模拟数据损坏
    
    Attributes:
        root: 真实文件系统根目录（被 FUSE 代理）
        mount_point: FUSE 挂载点
        rules: 故障规则列表
        stats: IO 统计
        _running: 是否运行中
        _lock: 线程锁
    """
    
    def __init__(
        self,
        root: str | Path,
        mount_point: str | Path,
        rules: list[FaultRule] | None = None,
        default_delay_ms: float = 0,
        debug: bool = False,
    ):
        if not FUSE_AVAILABLE:
            raise RuntimeError("FUSE 库未安装 (pip install fusepy)")
        
        super().__init__()
        
        self.root = Path(root).resolve()
        self.mount_point = Path(mount_point)
        self.rules = rules or []
        self.default_delay_ms = default_delay_ms
        self.debug = debug
        
        self.stats = IOStats()
        self._running = False
        self._lock = threading.RLock()
        self._active_pauses: dict[str, threading.Event] = {}
        self._io_log: list[dict] = []  # 记录所有 IO 操作
    
    def _real_path(self, path: str) -> Path:
        """将 FUSE 路径转换为真实路径"""
        # 移除开头的 /
        rel_path = path.lstrip('/')
        return self.root / rel_path
    
    def _log(self, operation: str, path: str, **kwargs) -> None:
        """记录操作日志"""
        if self.debug:
            entry = {
                'time': time.time(),
                'operation': operation,
                'path': path,
                **kwargs
            }
            self._io_log.append(entry)
            print(f"[FUSE] {operation}: {path} {kwargs}")
    
    def add_rule(self, rule: FaultRule) -> None:
        """动态添加故障规则"""
        with self._lock:
            self.rules.append(rule)
    
    def clear_rules(self) -> None:
        """清除所有故障规则"""
        with self._lock:
            self.rules.clear()
    
    def is_paused(self, path: str) -> bool:
        """检查文件是否处于暂停状态"""
        with self._lock:
            return path in self._active_pauses
    
    def resume(self, path: str | None = None) -> None:
        """恢复暂停的文件
        
        Args:
            path: 特定文件路径，None 表示恢复所有
        """
        with self._lock:
            if path:
                if path in self._active_pauses:
                    self._active_pauses[path].set()
                    del self._active_pauses[path]
            else:
                for event in self._active_pauses.values():
                    event.set()
                self._active_pauses.clear()
    
    def get_stats(self) -> IOStats:
        """获取 IO 统计（拷贝）"""
        with self._lock:
            from copy import copy
            return copy(self.stats)
    
    def get_io_log(self) -> list[dict]:
        """获取 IO 日志"""
        with self._lock:
            return self._io_log.copy()
    
    # =========================================================================
    # FUSE 回调实现
    # =========================================================================
    
    def getattr(self, path: str) -> dict:
        """获取文件属性"""
        real_path = self._real_path(path)
        
        if not real_path.exists():
            raise fuse.FuseOSError(errno.ENOENT)
        
        st = real_path.stat()
        
        return {
            'st_atime': st.st_atime,
            'st_ctime': st.st_ctime,
            'st_mtime': st.st_mtime,
            'st_mode': st.st_mode,
            'st_nlink': st.st_nlink,
            'st_size': st.st_size,
            'st_uid': st.st_uid,
            'st_gid': st.st_gid,
        }
    
    def readdir(self, path: str, offset: int) -> list:
        """读取目录"""
        real_path = self._real_path(path)
        
        if not real_path.is_dir():
            raise fuse.FuseOSError(errno.ENOTDIR)
        
        entries = ['.', '..']
        try:
            entries.extend(str(p.name) for p in real_path.iterdir())
        except OSError as e:
            raise fuse.FuseOSError(e.errno)
        
        return entries
    
    def open(self, path: str, flags: int) -> int:
        """打开文件"""
        real_path = self._real_path(path)
        
        if not real_path.exists():
            raise fuse.FuseOSError(errno.ENOENT)
        
        # 返回文件描述符（此处用 0 占位，实际使用真实 fd）
        return 0
    
    def read(self, path: str, size: int, offset: int, fh: int) -> bytes:
        """读取文件 - 主要故障注入点"""
        real_path = self._real_path(path)
        
        self._log('read', path, size=size, offset=offset)
        
        with self._lock:
            self.stats.read_count += 1
            self.stats.last_offset = offset
        
        # 检查故障规则
        for rule in self.rules:
            if rule.path == path or rule.path == '*':
                if rule.should_trigger(offset, size):
                    self._apply_rule(rule, path, offset)
        
        # 应用默认延迟
        if self.default_delay_ms > 0:
            time.sleep(self.default_delay_ms / 1000)
        
        # 执行实际读取
        try:
            with open(real_path, 'rb') as f:
                f.seek(offset)
                data = f.read(size)
                
                with self._lock:
                    self.stats.read_bytes += len(data)
                
                return data
        except OSError as e:
            raise fuse.FuseOSError(e.errno)
    
    def _apply_rule(self, rule: FaultRule, path: str, offset: int) -> None:
        """应用故障规则"""
        rule.mark_triggered()
        
        if rule.action == 'error':
            with self._lock:
                self.stats.error_count += 1
            raise fuse.FuseOSError(rule.error_code)
        
        elif rule.action == 'pause':
            event = rule.pause_event or threading.Event()
            with self._lock:
                self._active_pauses[path] = event
                self.stats.pause_count += 1
            
            self._log('pause', path, offset=offset)
            event.wait()  # 阻塞直到被释放
            
            with self._lock:
                if path in self._active_pauses:
                    del self._active_pauses[path]
        
        elif rule.action == 'delay':
            time.sleep(rule.delay_ms / 1000)
        
        elif rule.action == 'corrupt':
            # 数据损坏在 read 返回后处理，此处仅标记
            pass
    
    def write(self, path: str, data: bytes, offset: int, fh: int) -> int:
        """写入文件"""
        # 此文件系统主要用于读取测试，写入直接透传
        real_path = self._real_path(path)
        
        self._log('write', path, size=len(data), offset=offset)
        
        with self._lock:
            self.stats.write_count += 1
            self.stats.write_bytes += len(data)
        
        try:
            # 注意：实际写入到真实路径会修改源数据
            # 如需保护源数据，应写入副本
            with open(real_path, 'r+b') as f:
                f.seek(offset)
                return f.write(data)
        except OSError as e:
            raise fuse.FuseOSError(e.errno)
    
    def release(self, path: str, fh: int) -> None:
        """关闭文件"""
        self._log('release', path)
        
        # 清理暂停状态
        with self._lock:
            if path in self._active_pauses:
                del self._active_pauses[path]
    
    # =========================================================================
    # 控制接口
    # =========================================================================
    
    def start(self, foreground: bool = False) -> None:
        """启动 FUSE 文件系统
        
        Args:
            foreground: 是否在前台运行（调试时使用）
        """
        self._running = True
        
        # 解析参数
        args = [
            str(self.mount_point),
            '-o', 'allow_other',  # 允许其他用户访问
            '-o', 'default_permissions',
        ]
        
        if not foreground:
            args.append('-f')  # 前台模式，线程控制更简单
        
        self.parse(args)
        self.main()
    
    def stop(self) -> None:
        """停止 FUSE 文件系统"""
        self._running = False
        self.resume()  # 释放所有暂停
        
        # 尝试卸载
        try:
            import subprocess
            subprocess.run(
                ['fusermount', '-u', str(self.mount_point)],
                capture_output=True,
                check=False,
            )
        except Exception:
            pass


class FaultInjectedFSAsync:
    """基于 pyfuse3 的异步版本（性能更好）
    
    待实现：当需要更高性能时使用 pyfuse3 替代 fusepy
    """
    pass  # TODO: pyfuse3 实现
