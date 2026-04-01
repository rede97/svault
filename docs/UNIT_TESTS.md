# Svault 单元测试跟踪文档

> 本文档跟踪所有单元测试和集成测试的状态，随时更新。
> 
> 最后更新：2026-04-02

---

## 测试概览

| 类型 | 数量 | 通过 | 失败 | 跳过 |
|------|------|------|------|------|
| 单元测试 (Unit) | 10 | 10 | 0 | 0 |
| 集成测试 (Integration) | 0 | 0 | 0 | 0 |
| Python E2E 测试 | 66 | 64 | 0 | 2 |
| **总计** | **76** | **74** | **0** | **2** |

---

## 单元测试 (Unit Tests)

单元测试位于源代码文件中（内联测试），或 `src/` 目录下的测试模块。

### hash 模块

| 测试名 | 位置 | 描述 | 状态 | 备注 |
|--------|------|------|------|------|
| *待添加* | `src/hash/mod.rs` | CRC32C 哈希计算 | 🔲 TODO | |
| *待添加* | `src/hash/mod.rs` | XXH3-128 文件哈希 | 🔲 TODO | |
| *待添加* | `src/hash/mod.rs` | SHA-256 文件哈希 | 🔲 TODO | |
| *待添加* | `src/hash/mod.rs` | 大文件流式哈希 | 🔲 TODO | 测试 4MB 分块读取 |

### config 模块

| 测试名 | 位置 | 描述 | 状态 | 备注 |
|--------|------|------|------|------|
| *待添加* | `src/config.rs` | 默认配置生成 | 🔲 TODO | |
| *待添加* | `src/config.rs` | TOML 配置解析 | 🔲 TODO | |
| *待添加* | `src/config.rs` | 配置序列化 | 🔲 TODO | |

### db 模块

| 测试名 | 位置 | 描述 | 状态 | 备注 |
|--------|------|------|------|------|
| *待添加* | `src/db/mod.rs` | 数据库初始化 | 🔲 TODO | |
| *待添加* | `src/db/mod.rs` | 事件追加 | 🔲 TODO | |
| *待添加* | `src/db/mod.rs` | 哈希链验证 | 🔲 TODO | |
| *待添加* | `src/db/files.rs` | CRC32C 查询 | 🔲 TODO | |
| *待添加* | `src/db/files.rs` | 哈希查询 | 🔲 TODO | |
| *待添加* | `src/db/files.rs` | 文件插入 | 🔲 TODO | |
| `test_format_bytes` | `src/db/stats.rs` | 字节格式化 | ✅ PASS | 内联测试 |
| `test_format_count` | `src/db/stats.rs` | 数字千分位格式化 | ✅ PASS | 内联测试 |
| `test_format_value` | `src/db/dump.rs` | SQL 值格式化 | ✅ PASS | 内联测试 |
| `test_list_tables_empty_db` | `src/db/dump.rs` | 空数据库表列表 | ✅ PASS | 内联测试 |
| `test_list_tables_with_data` | `src/db/dump.rs` | 有数据时表列表 | ✅ PASS | 内联测试 |

### import 模块

| 测试名 | 位置 | 描述 | 状态 | 备注 |
|--------|------|------|------|------|
| `test_unix_now_ms_increases` | `src/import/utils.rs` | 时间戳递增测试 | ✅ PASS | 内联测试 |
| `test_session_id_format` | `src/import/utils.rs` | Session ID 格式测试 | ✅ PASS | 内联测试 |
| `test_resolve_dest_path` | `src/import/path.rs` | 路径模板解析 | ✅ PASS | 内联测试 |
| `test_resolve_dest_path_no_device` | `src/import/path.rs` | 无设备路径解析 | ✅ PASS | 内联测试 |
| `test_file_status_equality` | `src/import/mod.rs` | FileStatus 相等性 | ✅ PASS | 内联测试 |
| `test_lock_acquire_and_release` | `src/lock.rs` | Vault 咨询锁获取与释放 | ✅ PASS | 内联测试 |

### vfs 模块

| 测试名 | 位置 | 描述 | 状态 | 备注 |
|--------|------|------|------|------|
| *待添加* | `src/vfs/system.rs` | 文件系统能力探测 | 🔲 TODO | reflink/hardlink |
| *待添加* | `src/vfs/system.rs` | 目录遍历（含 `.svault` 剪枝） | 🔲 TODO | |
| *待添加* | `src/vfs/transfer.rs` | 传输引擎 fallback 链 | 🔲 TODO | reflink→hardlink→stream |

### import 模块

| 测试名 | 位置 | 描述 | 状态 | 备注 |
|--------|------|------|------|------|
| *待添加* | `src/import/mod.rs` | EXIF 日期解析 | 🔲 TODO | |
| *待添加* | `src/import/mod.rs` | 设备名提取 | 🔲 TODO | |
| *待添加* | `src/import/mod.rs` | 路径模板解析 | 🔲 TODO | `$year/$mon` 等 |
| *待添加* | `src/import/mod.rs` | 日期格式转换 | 🔲 TODO | YMD ↔ Unix 时间戳 |
| *待添加* | `src/import/mod.rs` | 去重逻辑 | 🔲 TODO | 三层去重 |

---

## Python E2E 测试

端到端测试位于 `e2e_tests/`，使用 `pytest` + RAMDisk 隔离测试环境。

### 常规场景 (Normal Scenarios)

| ID | 场景 | 描述 | 状态 | 最后验证 |
|----|------|------|------|----------|
| s1 | `s1_normal_apple` | 正常导入：EXIF 日期 + Apple 设备 | ✅ PASS | 2026-03-31 |
| s2 | `s2_no_device` | EXIF 日期存在，无 Make/Model → device=Unknown | ✅ PASS | 2026-03-31 |
| s3 | `s3_no_exif` | 无 EXIF — 路径使用 mtime 回退 | ✅ PASS | 2026-03-31 |
| s4 | `s4_duplicate` | 逐字节重复文件检测为重复 | ✅ PASS | 2026-03-31 |
| s5 | `s5_samsung` | Samsung 设备名称正确提取 | ✅ PASS | 2026-03-31 |
| s6 | `s6_make_in_model` | Model 以 Make 开头时避免重复（如 "Apple Apple iPhone"） | ✅ PASS | 2026-03-31 |

### Chaos 场景 (Chaos Scenarios)

| ID | 场景 | 描述 | 状态 | 备注 |
|----|------|------|------|------|
| c1 | 导入前重命名 | 重命名文件后仍检测为重复 | 🔲 TODO | |
| c2 | 移动到子目录 | 移动到子目录后仍能找到 | 🔲 TODO | |
| c3 | 中断复制 | 截断的 JPEG 文件处理 | 🔲 TODO | 应记录为失败 |
| c4 | 导入中增删文件 | 导入过程中源目录文件变化 | ✅ PASS | `test_concurrent_modification.py` |
| c5 | 重复导入 | 同一目录导入两次，第二次全为缓存命中 | ✅ PASS | `test_chaos.py` |

### Recheck 场景 (Recheck Scenarios)

| ID | 场景 | 描述 | 状态 | 备注 |
|----|------|------|------|------|
| r1 | `test_recheck_detects_corruption_and_reimport_succeeds` | recheck 检测 vault 文件损坏，删除后重新导入 | ✅ PASS | `test_recheck.py` |
| r2 | `test_recheck_no_cache_hits` | 对从未导入的源运行 recheck | ✅ PASS | `test_recheck.py` |
| r3 | `test_recheck_all_ok` | 正常导入后 recheck 全通过 | ✅ PASS | `test_recheck.py` |
| r4 | `test_strategy_copy_no_hardlink` | `--strategy copy` 必须真正二进制复制 | ✅ PASS | 修复了 copy 被忽略的问题 |
| r5 | `test_deleted_file_can_be_reimported_after_verify_failure` | verify 异常/文件删除后可重新导入 | ✅ PASS | `test_recheck.py` |

### Force Import 场景 (Force Import Scenarios)

| ID | 场景 | 描述 | 状态 | 备注 |
|----|------|------|------|------|
| f1 | `test_force_import_duplicate` | `--force` 强制导入重复文件 | ✅ PASS | `test_import_force.py` |
| f2 | `test_force_import_same_name_different_content` | 同名不同内容文件强制导入并重命名 | ✅ PASS | `test_import_force.py` |
| f3 | `test_force_import_recovers_deleted_file` | 删除 vault 文件后 `--force` 恢复 | ✅ PASS | `test_import_force.py` |

### Vault 自我保护场景 (Vault Self-Protection Scenarios)

| ID | 场景 | 描述 | 状态 | 备注 |
|----|------|------|------|------|
| v1 | `test_import_from_ancestor_skips_vault` | 从祖先目录导入时跳过 vault 自身文件 | ✅ PASS | `test_import_ignore.py` |

---

## 测试覆盖率目标

| 模块 | 目标覆盖率 | 当前状态 |
|------|-----------|----------|
| hash | 90% | 🔴 未开始 |
| config | 90% | 🔴 未开始 |
| db | 85% | 🔴 未开始 |
| vfs | 80% | 🔴 未开始 |
| import | 85% | 🟡 部分（通过 E2E） |

---

## 待办测试清单

### 高优先级

- [ ] `hash::crc32c_region` - 测试头部/尾部 CRC32C 计算
- [ ] `hash::xxh3_128_file` - 测试完整文件 XXH3 哈希
- [ ] `hash::sha256_file` - 测试完整文件 SHA-256 哈希
- [ ] `db::append_event` - 测试事件追加和哈希链
- [ ] `db::verify_chain` - 测试哈希链验证
- [ ] `import::resolve_dest_path` - 测试路径模板解析
- [ ] `import::read_exif_date_device` - 测试 EXIF 提取（多种格式）

### 中优先级

- [ ] `config::load` - 测试配置加载和默认值
- [ ] `vfs::probe_capabilities` - 测试文件系统能力探测
- [ ] `vfs::transfer_file` - 测试传输引擎 fallback 链
- [ ] `db::lookup_by_crc32c` - 测试 CRC32C 查询
- [ ] `db::lookup_by_hash` - 测试哈希查询

### 低优先级

- [ ] 并发导入测试
- [ ] 大文件（>4GB）处理测试
- [ ] 各种文件系统（btrfs/xfs/ext4）行为测试
- [ ] 网络文件系统（NFS/SMB）行为测试

---

## 更新记录

| 日期 | 更新内容 | 作者 |
|------|----------|------|
| 2026-03-31 | 初始版本：记录现有测试状态，添加待办清单 | Kimi |
| 2026-03-31 | 修复 EXIF 测试：使用 exiftool 生成测试固件 | Kimi |
| 2026-03-31 | 删除 scratch_exif.rs 临时测试文件，更新测试计数 | Kimi |
| 2026-03-31 | 实现 `svault status` 命令，添加 2 个单元测试 | Kimi |
| 2026-03-31 | 实现 `svault db dump` 命令，添加 3 个单元测试 | Kimi |
| 2026-04-02 | 将 `recheck` 从 `import --recheck` 改为独立命令；修复 `--strategy copy` 未生效问题；添加 vault 进程锁；添加 recheck/re-import E2E 测试 | Kimi |
| 2026-04-02 | VFS 重构：引入 `transfer.rs` 解耦传输策略；`--ignore-duplicate` 重命名为 `--force`；导入扫描自动忽略 `.svault` 和 vault root；E2E 新增 `test_import_force.py`、`test_import_ignore.py`（共 64 passed） | Kimi |

---

## 运行测试

```bash
# 所有单元测试和集成测试
cargo test

# 特定包测试
cargo test -p svault-core
cargo test -p svault-cli

# 特定模块测试
cargo test -p svault-core hash

# E2E 测试（推荐：自动使用 RAMDisk）
cd e2e_tests && bash run.sh --verbose

# 只跑特定测试文件
cd e2e_tests && bash run.sh --verbose test_import_force.py
```

---

## ⚠️ 重要测试规则

### 必须在 RAMDisk 中测试

**永远不要**在项目目录（`/home/mxq/Codes/svault` 或其子目录）中运行 `svault init` 或 `svault import`！

✅ **正确做法** - 使用 RAMDisk:
```bash
# 方法 1: 使用 E2E 测试框架（自动管理 RAMDisk）
cd e2e_tests && bash run.sh

# 方法 2: 手动使用 RAMDisk
cd /tmp/svault-ramdisk/vault    # 或 cd .ramdisk/vault
svault status
svault import /some/source/dir
```

❌ **错误做法** - 在项目目录中:
```bash
cd /home/mxq/Codes/svault
svault init      # 错误！会在项目目录创建 .svault/
svault import    # 错误！会污染项目目录
```

### 为什么使用 RAMDisk？

1. **隔离性** - 测试不会污染项目目录
2. **性能** - tmpfs 内存操作比磁盘快
3. **安全性** - 测试中的 bug 不会删除真实数据
4. **可重复性** - 每次测试从干净状态开始

### RAMDisk 位置

| 路径 | 说明 |
|------|------|
| `/tmp/svault-ramdisk` | 实际挂载点 |
| `.ramdisk` | 项目根目录的软链接（方便访问）|
| `.ramdisk/vault` | 测试 vault 目录 |
| `.ramdisk/source` | 测试源文件目录 |

---

## 添加新测试

添加新测试时，请：

1. **更新本文档** - 在对应模块表格中添加新行
2. **遵循命名规范** - `test_<模块>_<功能>_<条件>`
3. **添加文档注释** - 说明测试目的和预期行为
4. **更新覆盖率** - 如有工具支持，更新覆盖率统计

示例：

```rust
#[test]
fn test_hash_crc32c_region_head() {
    //! 测试 CRC32C 计算文件头部 64KB
    //! 
    //! # 步骤
    //! 1. 创建临时文件，写入 100KB 数据
    //! 2. 计算头部 64KB 的 CRC32C
    //! 3. 验证结果与预期值一致
    
    // ... test code
}
```
