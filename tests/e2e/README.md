# Svault E2E Tests (pytest)

基于 pytest 的端到端测试框架，替代旧的 `run_tests.py`。

## 特性

- **pytest-native**: 使用标准 pytest 特性（fixtures、markers、parametrize）
- **Hypothesis 支持**: 属性测试发现边界情况
- **RAMDisk 隔离**: 测试在内存文件系统中运行，不污染项目目录
- **复用现有脚本**: 使用 `tests/setup_ramdisk.sh` 管理 RAMDisk
- **灵活配置**: 命令行参数控制 RAMDisk 大小和行为

## 快速开始

### 0. 系统依赖（强制）

| 工具 | 用途 | 安装 |
|------|------|------|
| **exiftool** | **强制**。EXIF 固件写入的公认标准实现，确保测试固件与真实相机文件一致。缺失时 `run.sh` 直接退出、`source_factory` 直接 `pytest.fail` | `sudo apt install libimage-exiftool-perl` / `brew install exiftool` |
| strace | `test_import_interruption.py` 信号注入 | `sudo apt install strace` |
| FUSE + fusepy | `fuse_tests/` 故障注入 | `sudo apt install fuse3 && pip install fusepy` |
| ffmpeg | 部分视频元数据测试 | `sudo apt install ffmpeg` |

### 1. 环境设置（使用 uv）

```bash
cd tests/e2e

# 创建虚拟环境
uv venv --python=python3.12

# 安装依赖
uv pip install -e "."
```

### 2. 运行测试

```bash
# 运行所有测试（默认使用 debug 构建）
./run.sh

# 详细输出
./run.sh -v

# 只运行特定测试
./run.sh -v -k test_import

# 使用 release 构建
./run.sh --release

# 使用更大的 RAMDisk
./run.sh --ramdisk-size 512m

# 使用 1GB RAMDisk 并在测试后清理
./run.sh --ramdisk-size 1g --cleanup

# 包含 FUSE 深度故障注入测试（慢速，需要 FUSE 支持）
./run.sh --fuse

# Release 构建 + FUSE 测试
./run.sh --release --fuse
```

## 命令行选项

| 选项 | 说明 | 默认值 |
|------|------|--------|
| `--fuse` | 包含 FUSE 深度故障注入测试 | False (默认排除) |
| `--release` | 使用 release 构建（默认使用 debug） | False |
| `--ramdisk-size` | RAMDisk 大小 (e.g., 128m, 512m, 1g) | 256m |
| `--ramdisk-path` | RAMDisk 挂载路径 | /tmp/svault-ramdisk |
| `--cleanup` | 测试后清理 RAMDisk | False (保留用于检查) |
| `-k EXPRESSION` | 只运行匹配名称的测试 | - |
| `-m MARK` | 只运行特定标记的测试 | - |
| `-v` | 详细输出 | - |

## 测试分类

```bash
# 只运行快速测试（跳过 chaos 和 property）
./run.sh -m "not slow and not property"

# 只运行文件名冲突测试
./run.sh -m conflict

# 只运行去重测试
./run.sh -m dedup

# 只运行属性测试（Hypothesis）
./run.sh -m property
```

## 测试文件说明

> **套件宪法（2026-08-06 重设计）：**
> 1. **一测试一契约**——每个测试锁定一个唯一的可观测契约（退出码 / DB 状态 /
>    文件系统状态 / 输出内容）。禁止参数缩放版（N=2/8/20 同判据）、禁止数据源变体
>    （同判据换 fixture）。
> 2. **禁止弱断言**——`rc in [0, 1]`、`len(x) >= 1`、恒真条件均不允许；
>    发现即加固（锁真实行为）或删除。
> 3. **文件按命令域组织**——文件名 = 命令名；跨命令流程（恢复、中断、管线）独立成文件。
> 4. **覆盖归属唯一**——每个行为契约只有一个负责测试。新增测试前先查下表归属。
> 5. 故障注入（FUSE/strace 信号）归 `fuse_tests/` 与 `test_import_interruption.py`，
>    判据以 [docs/failure-handling.md](../../docs/failure-handling.md) 为准。

| 文件 | 内容 | 标记 |
|------|------|------|
| `test_import.py` | 主导入流程、EXIF/设备/回退路径组织、CLI 交互、`--force`、`--show-dup`、边界（截断/嵌套/空目录） | `chaos`（边界组） |
| `test_import_dedup.py` | 身份判定矩阵：去重 + 文件名冲突 + CRC 碰撞 | `dedup`, `conflict` |
| `test_import_recovery.py` | 幂等重跑、增量导入、修改识别、vault 内移动 | - |
| `test_import_interruption.py` | strace 信号中断恢复、并发修改、不可读/伪装文件 | - |
| `test_import_cross_fs.py` | 跨文件系统导入（ext4/btrfs/reflink） | `cross_fs`, Linux |
| `test_import_disk_full.py` | ENOSPC 行为（loopback ext4 真实满盘） | - |
| `test_import_video_metadata.py` | 视频元数据提取（creation_time、设备） | - |
| `test_add.py` | `add` 注册 vault 内文件、去重、vault 内移动检测 | - |
| `test_update.py` | `update` 路径修正、missing 标记、dry-run | - |
| `test_clone.py` | `clone` 单向导出（过滤、审计事件） | - |
| `test_sync.py` | `sync` vault 间同步（幂等、冲突、moved、只读源） | - |
| `test_scan_import_pipeline.py` | scan → files-from 管道工作流、空格路径往返契约 | debug binary |
| `test_binding.py` | 复合媒体绑定（Live Photo 等） | - |
| `test_media_formats.py` | 媒体格式矩阵（base/别名/大小写三契约）、过滤、路径模板 | - |
| `test_raw_id.py` | RAW 唯一 ID 去重 | - |
| `test_path_compatibility.py` | 跨平台路径格式（Unix 风格存储） | - |
| `test_config_transfer.py` | 配置（创建/错误处理）、传输策略、hardlink 升级 | - |
| `test_property.py` | Hypothesis 属性测试 | `property`, `slow` |
| `test_verify.py` | verify（损坏检测/算法选择/摘要）、recheck、background-hash、db verify-chain | `verify` |
| `fuse_tests/` | **FUSE 深度故障注入测试** | `fuse`, `slow` |

## 使用 Fixtures

```python
def test_example(vault: VaultEnv, source_factory: callable):
    """示例测试展示 fixtures 用法。"""
    # 创建测试文件
    source_factory(
        "test.jpg",
        exif_date="2024:05:01 10:30:00",
        exif_make="Apple",
        exif_model="iPhone 15",
    )
    
    # 执行导入
    vault.import_dir(vault.source_dir)
    
    # 验证结果
    row = vault.find_file_in_db("test.jpg")[0]
    assert row["status"] == "imported"
    assert "Apple iPhone 15" in row["path"]
```

## 从旧测试框架迁移

旧命令:
```bash
python3 tests/run_tests.py --verbose --chaos
```

新命令:
```bash
./run.sh -v -m "chaos"
```

## RAMDisk 管理

**测试自动管理 RAMDisk**，无需手动设置：

```bash
# RAMDisk 由 Python fixtures 自动挂载（如果需要）
./run.sh

# 手动管理（仅用于调试/检查）
bash tests/setup_ramdisk.sh --clean  # 手动挂载
cd /tmp/svault-ramdisk/vault         # 检查状态
bash tests/setup_ramdisk.sh --umount # 手动卸载
```

**注意**：`setup_ramdisk.sh` 仅用于手动调试。测试框架使用 `conftest.py` 中的 `RamDisk` 类自动管理挂载。

## FUSE 深度故障注入测试

对于需要**精确 IO 控制**的场景（如传输中途中断、字节级错误注入），使用 FUSE 测试框架。

### 运行方式

**方式 1：与主测试一起运行（推荐）**
```bash
# 默认排除 FUSE 测试
./run.sh

# 包含 FUSE 测试（需要 FUSE 支持）
./run.sh --fuse

# 只运行 FUSE 测试
./run.sh --fuse -k fuse
```

**方式 2：单独运行 FUSE 测试（调试专用）**
```bash
cd fuse_tests

# 安装依赖（fusepy）
pip install -r requirements.txt

# 运行所有 FUSE 测试
./run_fuse.sh

# 只运行特定测试
./run_fuse.sh -v -k test_import_pause

# 调试模式（保留挂载点）
./run_fuse.sh --debug --keep-mount
```

### FUSE 测试 vs 常规测试

| 特性 | 常规测试 | FUSE 测试 |
|-----|---------|----------|
| 运行速度 | 快 | 慢（有 overhead） |
| IO 控制精度 | 时间估算 | 字节级精确 |
| 依赖 | 仅 Python | FUSE 内核模块 |
| 使用场景 | 回归测试 | 深度故障注入 |

### FUSE 测试覆盖场景

- **传输中断**：在文件读取到 25%/50%/75% 时精确暂停
- **错误注入**：特定偏移量返回 EIO/ENOSPC/EAGAIN
- **延迟模拟**：慢速存储、网络抖动
- **数据损坏**：传输中篡改数据，验证校验和检测

详见 [fuse_tests/README.md](./fuse_tests/README.md) 和 [fuse_tests/VALIDATION_PLAN.md](./fuse_tests/VALIDATION_PLAN.md)

## Troubleshooting

### ROS 环境冲突

如果遇到 `launch_testing` 相关错误，确保使用 `./run.sh` 运行，它会清理环境：

```bash
# 错误: ModuleNotFoundError: No module named 'yaml'
# 正确做法:
./run.sh
```

### RAMDisk 权限问题

```bash
# 如果 RAMDisk 有权限问题，手动清理
bash tests/setup_ramdisk.sh --umount
# 或者使用 sudo
sudo umount /tmp/svault-ramdisk 2>/dev/null || true
```

### 测试失败后检查状态

测试后 RAMDisk 默认保留（除非使用 `--cleanup`），可以检查状态：

```bash
# 检查上次测试的 vault
cd /tmp/svault-ramdisk/vault
# 使用 debug 构建（默认）或 release 构建（加 --release）
../../target/debug/svault status
# 或: ../../target/release/svault status

python3 -c "import sqlite3; conn = sqlite3.connect('.svault/vault.db'); print(conn.execute('SELECT path, status FROM files').fetchall())"

# 完成后手动清理
bash tests/setup_ramdisk.sh --umount
```
