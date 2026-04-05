# FUSE 故障注入测试验证计划

## 目标

验证 svault 在精确控制的 IO 故障场景下的健壮性，包括：
1. 传输中途中断的数据一致性
2. 部分写入文件的检测和处理
3. 错误恢复和重试机制
4. 极端边界条件下的行为

## 测试架构

```
fuse_tests/
├── test_import_fuse.py      # Import 故障注入测试
├── test_recheck_fuse.py     # Recheck 故障注入测试
├── test_verify_fuse.py      # Verify 故障注入测试
└── test_edge_cases_fuse.py  # 极端边界测试
```

## 详细测试计划

### 1. test_import_fuse.py - Import 中断测试

#### 1.1 精确偏移量中断

**测试: `test_import_pause_at_25_percent`**
- **目标**: 在文件读取到 25% 时暂停，验证 svault 状态
- **步骤**:
  1. 创建 10KB 测试文件
  2. 配置 FUSE 在 offset=2560 处 pause
  3. 启动异步 import 进程
  4. 等待 pause 触发（FUSE 报告 is_paused）
  5. 验证：数据库中有未完成记录？/ 文件部分写入？
  6. 发送 SIGTERM 终止 import
  7. 释放 pause，重新 import
  8. 验证最终一致性

**测试: `test_import_pause_at_50_resume`**
- **目标**: 暂停后继续，验证断点续传
- **步骤**:
  1. 同上配置在 50% 处 pause
  2. 启动 import
  3. 确认 pause 后，直接释放（不终止）
  4. 验证 import 能自动完成
  5. 验证文件完整（哈希正确）

**测试: `test_import_pause_multiple_files`**
- **目标**: 多文件场景下的精确控制
- **步骤**:
  1. 创建 10 个 1KB 文件
  2. 配置在第 5 个文件的 50% 处 pause
  3. 验证前 4 个文件已完成，第 5 个部分完成

#### 1.2 IO 错误注入

**测试: `test_import_eio_at_offset`**
- **目标**: 特定偏移量返回 EIO，验证错误处理
- **步骤**:
  1. 创建 10KB 文件
  2. 配置在 offset=5120 处返回 EIO
  3. 执行 import
  4. 验证：svault 报告错误 / 部分导入的文件被清理或标记

**测试: `test_import_enospc_simulation`**
- **目标**: 模拟磁盘满（写入时返回 ENOSPC）
- **步骤**:
  1. 配置 write 操作在特定大小后返回 ENOSPC
  2. 执行 import
  3. 验证：优雅处理，无崩溃
  4. 清除限制，重新 import 验证恢复

**测试: `test_import_eagain_retry`**
- **目标**: EAGAIN 重试机制
- **步骤**:
  1. 配置前 3 次 read 返回 EAGAIN，第 4 次成功
  2. 验证 svault 是否正确重试

#### 1.3 延迟和超时

**测试: `test_import_slow_read_timeout`**
- **目标**: 慢速读取触发超时
- **步骤**:
  1. 配置每字节 100ms 延迟
  2. 设置 svault 超时参数
  3. 验证超时行为和后续处理

**测试: `test_import_variable_delay`**
- **目标**: 变化的延迟模拟真实慢存储
- **步骤**:
  1. 配置随机 10-500ms 延迟
  2. 大批量导入，验证稳定性

#### 1.4 数据损坏检测

**测试: `test_import_corrupt_at_offset`**
- **目标**: 传输中数据被篡改的检测
- **步骤**:
  1. 创建已知哈希的文件
  2. 配置 FUSE 在特定偏移返回篡改数据
  3. 验证 svault 检测到哈希不匹配

**测试: `test_import_truncated_file`**
- **目标**: 文件被截断的处理
- **步骤**:
  1. 配置返回比 stat 报告的更短的数据
  2. 验证 EOF 处理

### 2. test_recheck_fuse.py - Recheck 中断测试

#### 2.1 校验中途中断

**测试: `test_recheck_pause_at_half_files`**
- **目标**: 校验一半文件时中断
- **步骤**:
  1. 导入 10 个文件
  2. 配置 FUSE 在第 5 个文件校验时 pause
  3. 启动 recheck
  4. 验证前 4 个已校验，第 5 个未标记
  5. 中断并重新 recheck，验证继续

**测试: `test_recheck_source_modified_during_check`**
- **目标**: 校验过程中源文件被修改
- **步骤**:
  1. 导入文件（hardlink 模式）
  2. 配置 FUSE 在校验到一半时修改文件内容
  3. 验证 svault 检测到变化

#### 2.2 Vault 文件读取错误

**测试: `test_recheck_vault_file_eio`**
- **目标**: vault 文件读取失败
- **步骤**:
  1. 正常导入
  2. 配置 FUSE 在读取 vault 文件时返回 EIO
  3. 验证错误报告

**测试: `test_recheck_vault_file_corrupt`**
- **目标**: vault 文件损坏检测
- **步骤**:
  1. 导入文件
  2. 配置 FUSE 返回篡改的数据
  3. 验证哈希校验失败

### 3. test_verify_fuse.py - Verify 中断测试

#### 3.1 验证中断

**测试: `test_verify_pause_resume`**
- **目标**: 验证过程暂停继续
- **步骤**:
  1. 导入大量文件
  2. 配置在特定进度 pause
  3. 验证 resume 后能继续

**测试: `test_verify_partial_failure`**
- **目标**: 部分文件验证失败
- **步骤**:
  1. 导入文件
  2. 配置部分文件返回 EIO
  3. 验证报告区分成功/失败

### 4. test_edge_cases_fuse.py - 极端边界测试

#### 4.1 极小粒度故障

**测试: `test_single_byte_read_failure`**
- **目标**: 单字节读取失败的处理
- **步骤**:
  1. 配置每次只返回 1 字节，第 100 字节失败
  2. 验证正确处理

**测试: `test_alternating_success_failure`**
- **目标**: 交替成功/失败模式
- **步骤**:
  1. 配置奇数次调用成功，偶数次失败
  2. 验证重试机制

#### 4.2 并发和竞争

**测试: `test_concurrent_read_same_file`**
- **目标**: 多线程读取同一文件
- **步骤**:
  1. 配置慢速读取
  2. 多线程 import
  3. 验证无死锁

#### 4.3 文件系统边界

**测试: `test_empty_file_special_handling`**
- **目标**: 空文件处理
- **步骤**:
  1. 创建空文件
  2. 配置各种故障规则
  3. 验证空文件被正确处理

**测试: `test_large_file_4gb_boundary`**
- **目标**: 大文件边界（如果支持）
- **步骤**:
  1. 创建接近/超过 4GB 的稀疏文件
  2. 在边界附近注入故障
  3. 验证偏移量计算正确

### 5. test_corruption_fuse.py - 硬件损坏与静默损坏测试

本文件使用 FUSE 模拟硬件级别的数据损坏场景，这些场景无法通过常规测试实现。

#### 5.1 Fundamental Problem 演示

**测试: `test_corrupted_hash_undetectable_by_verify`**
- **目标**: 演示哈希验证的根本限制
- **场景**: 坏道导致哈希基于损坏数据计算
- **步骤**:
  1. 创建正常文件
  2. FUSE 在 offset=1024 返回 0xFF（模拟坏道）
  3. svault import 读取损坏数据，计算 H_bad
  4. DB 存储 H_bad，vault 存储损坏文件
  5. verify 通过（H_bad == H_bad）
  6. 但 recheck --source 会发现不匹配！
- **结论**: 说明为什么需要外部参考或多会话验证

**测试: `test_silent_corruption_at_specific_offset`**
- **目标**: 特定偏移量的静默位翻转
- **步骤**:
  1. 文件内容 "ORIGINAL_DATA"
  2. FUSE 在 offset=8 将 'D' 改为 'X'（静默）
  3. svault 计算哈希基于损坏数据
  4. verify 无法检测
  5. 验证：需要定期 recheck --source 或外部校验

#### 5.2 不稳定存储模拟

**测试: `test_unstable_read_during_import`**
- **目标**: 多次读取返回不同数据
- **步骤**:
  1. FUSE 配置：第 1 次读取返回 A，第 2 次返回 B
  2. svault 哈希计算用 A，复制用 B
  3. 验证：写入后校验应发现不匹配
  4. 或说明需要单次读取原子性

**测试: `test_bit_rot_detection`**
- **目标**: 随时间推移的数据衰减
- **步骤**:
  1. 正常导入文件
  2. FUSE 后续读取注入 1 bit 翻转（模拟老化）
  3. recheck/verify 应检测到哈希不匹配

#### 5.3 损坏检测策略验证

**测试: `test_post_import_source_recheck_detects_corruption`**
- **目标**: 验证导入后源重检查的有效性
- **步骤**:
  1. 第一次读取（导入）：正常数据
  2. 后续读取（recheck）：损坏数据
  3. recheck --source 对比发现差异
  4. 报告潜在损坏

**测试: `test_multiple_hash_algorithms_detect_corruption`**
- **目标**: 多层哈希提高检测率
- **步骤**:
  1. 使用 CRC32C + XXH3 + SHA256
  2. FUSE 注入精心构造的损坏（可能逃过一种哈希）
  3. 验证至少一种哈希能捕获

#### 5.4 真实世界场景

**测试: `test_aging_hard_drive_simulation`**
- **目标**: 老化硬盘行为模拟
- **行为**:
  - 读取延迟逐渐增加
  - 偶尔需要多次重试
  - 特定区域（坏道）返回损坏数据
- **验证**: svault 能优雅处理并在可能时恢复

**测试: `test_network_storage_interruption`**
- **目标**: NFS/SMB 中断模拟
- **行为**:
  - 读取时返回 EIO 或 ETIMEDOUT
  - 超时后自动恢复
  - 验证重试和恢复机制

## 实现优先级

### P0 - 核心验证（必须先实现）
1. `test_import_pause_at_25_percent` - 基础暂停机制
2. `test_import_pause_at_50_resume` - 继续验证
3. `test_import_eio_at_offset` - 错误处理基础
4. `test_recheck_pause_at_half_files` - recheck 基础
5. `test_corrupted_hash_undetectable_by_verify` - 根本问题演示

### P1 - 重要场景
6. `test_import_enospc_simulation` - 磁盘满
7. `test_import_pause_multiple_files` - 多文件
8. `test_recheck_source_modified_during_check` - 源变化
9. `test_silent_corruption_at_specific_offset` - 静默损坏
10. `test_unstable_read_during_import` - 不稳定读取

### P2 - 深度验证
11. 延迟/超时测试
12. 多种哈希检测策略
13. 老化硬盘模拟
14. 极端边界测试

## 依赖清单

```bash
# 基础依赖
pip install fusepy>=3.0.1
pip install pytest>=7.0.0
pip install psutil>=5.9.0

# 系统依赖 (Ubuntu/Debian)
sudo apt-get install fuse3 libfuse3-dev

# 系统依赖 (Fedora)
sudo dnf install fuse3 fuse3-devel

# 系统依赖 (macOS)
brew install macfuse
```

## 运行命令

```bash
# 运行所有 FUSE 测试
cd tests/e2e/fuse_tests && ./run_fuse.sh

# 只运行 P0 测试
./run_fuse.sh -v -k "pause_at_25 or pause_at_50 or eio_at_offset"

# 调试模式，保留挂载点
./run_fuse.sh --debug --keep-mount -k test_import_pause_at_25_percent
```

## 成功标准

1. 所有 P0 测试通过
2. 测试能在 CI 环境运行（Linux，有 FUSE 支持）
3. 单次测试运行时间 < 5 分钟
4. 内存使用稳定，无泄漏

## 后续扩展

- [ ] NFS 行为模拟（属性缓存、弱一致性）
- [ ] SMB 锁冲突模拟
- [ ] MTP 设备断开模拟
- [ ] 网络存储延迟抖动模拟
