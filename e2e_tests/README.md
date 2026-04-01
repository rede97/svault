# Svault E2E Tests (pytest)

基于 pytest 的端到端测试框架，替代旧的 `run_tests.py`。

## 特性

- **pytest-native**: 使用标准 pytest 特性（fixtures、markers、parametrize）
- **Hypothesis 支持**: 属性测试发现边界情况
- **RAMDisk 隔离**: 测试在内存文件系统中运行，不污染项目目录
- **复用现有脚本**: 使用 `tests/setup_ramdisk.sh` 管理 RAMDisk
- **灵活配置**: 命令行参数控制 RAMDisk 大小和行为

## 快速开始

### 1. 环境设置（使用 uv）

```bash
cd e2e_tests

# 创建虚拟环境
uv venv --python=python3.12

# 安装依赖
uv pip install -e "."
```

### 2. 运行测试

```bash
# 运行所有测试
./run.sh

# 详细输出
./run.sh -v

# 只运行特定测试
./run.sh -v -k test_import

# 使用更大的 RAMDisk
./run.sh --ramdisk-size 512m

# 使用 1GB RAMDisk 并在测试后清理
./run.sh --ramdisk-size 1g --cleanup
```

## 命令行选项

| 选项 | 说明 | 默认值 |
|------|------|--------|
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

| 文件 | 内容 | 标记 |
|------|------|------|
| `test_import_basic.py` | 基础导入场景 | - |
| `test_import_force.py` | 强制导入与重复文件 | `force` |
| `test_import_ignore.py` | Vault 自我保护扫描过滤 | `ignore` |
| `test_recheck.py` | Recheck 工作流 | `recheck` |
| `test_verify.py` | 完整性校验 | `verify` |
| `test_conflict.py` | 文件名冲突处理 | `conflict` |
| `test_dedup.py` | 三层去重系统 | `dedup` |
| `test_chaos.py` | 边界/异常场景 | `chaos`, `slow` |
| `test_property.py` | Hypothesis 属性测试 | `property`, `slow` |
| `test_atomic_verification.py` | 原子验证限制 | `atomic` |
| `test_concurrent_modification.py` | 并发修改与恢复 | `concurrent` |

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
../../target/release/svault status
python3 -c "import sqlite3; conn = sqlite3.connect('.svault/vault.db'); print(conn.execute('SELECT path, status FROM files').fetchall())"

# 完成后手动清理
bash tests/setup_ramdisk.sh --umount
```
