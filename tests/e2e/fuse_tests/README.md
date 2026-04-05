# FUSE 深度故障注入测试

基于 FUSE (Filesystem in Userspace) 的精确 IO 控制测试框架，用于验证 svault 在极端场景下的健壮性。

## 为什么需要 FUSE 测试

常规测试使用 `time.sleep()` 估算中断时机，不可靠：
```python
proc = subprocess.Popen([...])
time.sleep(0.3)  # 祈祷现在在传输中途
proc.send_signal(signal.SIGTERM)  # 可能文件已传完，或还没开始
```

FUSE 方案提供**精确控制**：
```python
# 在第 N 个字节读取时暂停
if offset == target_offset:
    self.pause_event.wait()  # 精确控制，想停多久停多久
```

## 测试能力

| 能力 | 常规测试 | FUSE 测试 |
|-----|---------|----------|
| 精确暂停点 | ❌ 时间估算 | ✅ 字节级精确 |
| 传输中途中断 | ❌ 不可靠 | ✅ read() 中 pause |
| 特定偏移错误 | ❌ 无法实现 | ✅ offset == N 时 EIO |
| 慢速存储模拟 | ❌ 需真实慢磁盘 | ✅ 添加 delay |
| 间歇性故障 | ❌ 难模拟 | ✅ 概率性错误注入 |
| 网络存储异常 | ❌ 需真实网络 | ✅ EAGAIN/ETIMEDOUT |

## 测试场景覆盖

### Import 中断场景
- [ ] 文件读取到 50% 时源文件变为不可读
- [ ] 写入到特定偏移时磁盘满 (ENOSPC)
- [ ] 传输中途网络断开 (EIO/EAGAIN)
- [ ] 慢速存储导致超时触发
- [ ] 部分写入后校验和失败

### Recheck 中断场景
- [ ] 校验到一半源文件被修改
- [ ] vault 文件读取时返回错误
- [ ] 校验过程被暂停后恢复

### 极端边界场景
- [ ] 单个字节读取返回 EIO
- [ ] 交替成功/失败的读取模式
- [ ] 读取延迟逐渐增加到超时

## 项目结构

```
fuse_tests/
├── README.md              # 本文件
├── run_fuse.sh            # 测试运行脚本
├── requirements.txt       # 依赖：fusepy 或 pyfuse3
├── conftest.py            # FUSE fixtures
├── fault_inject_fs.py     # 故障注入 FUSE 实现
├── test_import_fuse.py    # Import FUSE 测试
├── test_recheck_fuse.py   # Recheck FUSE 测试
└── test_verify_fuse.py    # Verify FUSE 测试
```

## 快速开始

### 1. 安装依赖

```bash
cd tests/e2e/fuse_tests

# Ubuntu/Debian
sudo apt-get install fuse3 libfuse3-dev

# Fedora
sudo dnf install fuse3 fuse3-devel

# macOS
brew install macfuse

# Python 依赖
pip install fusepy  # 或 pyfuse3（性能更好，但编译复杂）
```

### 2. 运行测试

```bash
# 运行所有 FUSE 测试
./run_fuse.sh

# 详细输出
./run_fuse.sh -v

# 特定测试
./run_fuse.sh -v -k test_read_interrupt_at_50_percent

# 调试模式（保留挂载点用于检查）
./run_fuse.sh --debug --keep-mount
```

## 架构设计

### FaultInjectedFS 类

```python
class FaultInjectedFS(fuse.Fuse):
    """可编程故障注入文件系统"""
    
    def __init__(self, *args, **kwargs):
        self.fault_config = {
            'read_faults': [],      # 读取时注入的错误
            'write_faults': [],     # 写入时注入的错误  
            'pause_points': [],     # 暂停点配置
            'delay_ms': 0,          # 全局延迟
        }
    
    def read(self, path, size, offset):
        # 检查是否需要暂停
        if self.should_pause(path, offset):
            self.pause_event.wait()
        
        # 检查是否需要注入错误
        if self.should_fault(path, offset, 'read'):
            raise fuse.FuseError(errno.EIO)
        
        # 应用延迟
        if self.delay_ms:
            time.sleep(self.delay_ms / 1000)
        
        return super().read(path, size, offset)
```

### 与主测试框架的关系

```
tests/e2e/
├── conftest.py           # 主 fixtures (VaultEnv, source_factory)
├── test_*.py             # 常规测试（快速、稳定）
└── fuse_tests/           # FUSE 测试（深度、精确）
    ├── conftest.py       # 继承主 fixtures + FUSE 专用
    └── test_*.py         # FUSE 专用测试
```

**设计原则**：
1. FUSE 测试是补充而非替代，常规测试覆盖 90% 场景
2. FUSE 测试独立运行，不拖慢主测试套件
3. 共享 fixtures，保持测试编写风格一致

## 已知限制

| 限制 | 说明 |
|-----|------|
| Linux 为主 | FUSE 在 Linux 最稳定，macOS 次之，Windows 需 WinFsp |
| 需要权限 | 某些场景需要 root 或 fuse 组权限 |
| 性能开销 | FUSE 有用户态/内核态切换开销，不适合大文件 |
| 单线程 | fusepy 默认单线程，需显式启用多线程测试并发 |

## 未来扩展

- [ ] 支持 pyfuse3（性能更好，支持 asyncio）
- [ ] NFS 行为模拟（属性缓存、弱一致性）
- [ ] SMB 异常模拟（锁冲突、会话过期）
- [ ] MTP 设备模拟（通过 FUSE 暴露 MTP 协议行为）
