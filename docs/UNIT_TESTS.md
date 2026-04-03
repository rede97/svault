# Svault 单元测试跟踪文档

> 本文档跟踪所有单元测试和集成测试的状态，随时更新。
> 
> 最后更新：2026-04-04

---

## 测试概览

| 类型 | 数量 | 通过 | 失败 | 跳过 |
|------|------|------|------|------|
| 单元测试 (Unit) | 35 | 32 | 0 | 3 |
| 集成测试 (Integration) | 0 | 0 | 0 | 0 |
| Python E2E 测试 (Linux) | 87 | 85 | 0 | 2 |
| Python E2E 测试 (Windows) | 87 | 85 | 0 | 2 |
| **总计** | **209** | **201** | **0** | **7** |

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
| *待添加* | `src/vfs/transfer.rs` | 传输引擎 fallback 链 | 🔲 TODO | 按策略列表顺序尝试，`copy` 始终兜底 |

### import 模块

| 测试名 | 位置 | 描述 | 状态 | 备注 |
|--------|------|------|------|------|
| *待添加* | `src/import/mod.rs` | EXIF 日期解析 | 🔲 TODO | |
| *待添加* | `src/import/mod.rs` | 设备名提取 | 🔲 TODO | |
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
| c1 | 导入前重命名 | 重命名文件后仍检测为重复 | ✅ PASS | `test_chaos.py::test_renamed_before_import` |
| c2 | 移动到子目录 | 移动到子目录后仍能找到 | ✅ PASS | `test_chaos.py::test_moved_subdirectory` |
| c3 | 中断复制 | 截断的 JPEG 文件处理 | ✅ PASS | `test_chaos.py::test_truncated_jpeg_handling` |
| c4 | 导入中增删文件 | 导入过程中源目录文件变化 | ✅ PASS | `test_concurrent_modification.py` |
| c5 | 重复导入 | 同一目录导入两次，第二次全为缓存命中 | ✅ PASS | `test_chaos.py` |

### Recheck 场景 (Recheck Scenarios)

| ID | 场景 | 描述 | 状态 | 备注 |
|----|------|------|------|------|
| r1 | `test_recheck_detects_corruption_and_reimport_succeeds` | recheck 检测 vault 文件损坏，删除后重新导入 | ✅ PASS | `test_recheck.py` |
| r2 | `test_recheck_all_ok` | 正常导入后 recheck 全通过 | ✅ PASS | `test_recheck.py` |
| r3 | `test_recheck_source_mismatch` | 提供不匹配的 source 路径时 recheck 报错 | ✅ PASS | `test_recheck.py` |
| r4 | `test_recheck_with_matching_source` | 提供匹配的 source 路径时 recheck 正常 | ✅ PASS | `test_recheck.py` |
| r5 | `test_strategy_copy_no_hardlink` | `--strategy copy` 必须真正二进制复制 | ✅ PASS | 修复了 copy 被忽略的问题 |
| r6 | `test_deleted_file_can_be_reimported_after_verify_failure` | verify 异常/文件删除后可重新导入 | ✅ PASS | `test_recheck.py` |

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

### Add / Reconcile 场景

| ID | 场景 | 描述 | 状态 | 备注 |
|----|------|------|------|------|
| a1 | `test_add_tracks_existing_files` | `add` 注册已存在于 vault 内的文件 | ✅ PASS | `test_add.py` |
| a2 | `test_add_skips_already_tracked` | `add` 对已跟踪文件无重复写入 | ✅ PASS | `test_add.py` |
| a3 | `test_add_detects_duplicates` | `add` 检测并跳过重复内容 | ✅ PASS | `test_add.py` |
| rc1 | `test_reconcile_finds_moved_file` | `reconcile` 恢复 vault 内被重命名的文件路径 | ✅ PASS | `test_reconcile.py` |
| rc2 | `test_reconcile_dry_run_no_changes` | `reconcile` dry-run 不修改数据库 | ✅ PASS | `test_reconcile.py` |
| rc3 | `test_reconcile_no_missing_files` | 无缺失文件时 `reconcile` 正常报告 | ✅ PASS | `test_reconcile.py` |

### Verify 场景

| ID | 场景 | 描述 | 状态 | 备注 |
|----|------|------|------|------|
| vy1 | `test_verify_all_ok` | 默认 hash 配置下全部通过 | ✅ PASS | `test_verify.py` |
| vy2 | `test_verify_detects_bit_flip` | 检测单字节损坏 | ✅ PASS | `test_verify.py` |
| vy3 | `test_verify_detects_truncation` | 检测文件截断 | ✅ PASS | `test_verify.py` |
| vy4 | `test_verify_detects_missing_file` | 检测文件缺失 | ✅ PASS | `test_verify.py` |
| vy5 | `test_verify_with_sha256` | 使用 `secure` 算法验证 | ✅ PASS | `test_verify.py` |
| vy6 | `test_verify_with_xxh3_128` | 使用 `fast` 算法验证 | ✅ PASS | `test_verify.py` |

### History 场景

| ID | 场景 | 描述 | 状态 | 备注 |
|----|------|------|------|------|
| h1 | `test_history_shows_import_events` | `history` 显示 `file.imported` 事件 | ✅ PASS | `test_history.py` |
| h2 | `test_history_json_output` | `history --output=json` 输出有效 JSON | ✅ PASS | `test_history.py` |
| h3 | `test_history_filter_by_event_type` | `--event-type` 过滤特定事件 | ✅ PASS | `test_history.py` |
| h4 | `test_history_filter_by_file_path` | `--file` 过滤特定文件事件 | ✅ PASS | `test_history.py` |
| h5 | `test_history_limit` | `--limit` 限制返回事件数 | ✅ PASS | `test_history.py` |
| h6 | `test_history_filter_by_date_range` | `--from`/`--to` 时间范围过滤 | ✅ PASS | `test_history.py` |

### Background Hash 场景

> 注：`background-hash` 已并入 `verify` 命令，通过 `svault verify --background-hash` 调用。

| ID | 场景 | 描述 | 状态 | 备注 |
|----|------|------|------|------|
| bh1 | `test_background_hash_computes_missing_sha256` | `verify --background-hash` 补齐 fast hash 导入后缺失的 SHA-256 | ✅ PASS | `test_background_hash.py` |
| bh2 | `test_background_hash_no_pending_files` | 无 pending 文件时返回 0 | ✅ PASS | `test_background_hash.py` |
| bh3 | `test_background_hash_limit` | `--background-hash-limit` 限制处理数量 | ✅ PASS | `test_background_hash.py` |
| bh4 | `test_background_hash_nice_does_not_fail` | `--background-hash-nice` 低优先级运行不报错 | ✅ PASS | `test_background_hash.py` |

### Hardlink Upgrade 场景

| ID | 场景 | 描述 | 状态 | 备注 |
|----|------|------|------|------|
| hl1 | `test_upgrade_hardlink_during_verify` | `verify --upgrade-links` 将 hardlink 升级为独立副本 | ✅ PASS | `test_hardlink_upgrade.py` |
| hl2 | `test_upgrade_links_no_op_for_regular_files` | 普通文件不触发升级 | ✅ PASS | `test_hardlink_upgrade.py` |

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

### 功能/命令规划待办（暂不实现）

> 以下功能已纳入设计，但暂时不修改代码，仅作为后续开发路线图记录。

- [ ] **`svault scan <source>`** — 仅执行 Stage A/B 扫描（目录遍历 + CRC32C 缓存查询），输出 `likely-new` 文件列表（每行一个相对路径）。支持的扫描/判断参数与 `import` 对齐，包括 `--hash`、`--strategy`、`--show-dup`、`--force`、`--target` 等。不执行复制、不写数据库、不写 manifest。
- [ ] **`svault import --files-from <path>`** — 从文本文件（或 `-` 表示标准输入）读取相对路径列表，跳过完整目录扫描，仅对列表中指定的文件执行后续导入流程（Stage C/E）。路径格式为相对于 `<source>` 的相对路径。
- [ ] **管道工作流（Pipeline workflow）** — 结合 `scan` 与 `--files-from` 支持 Unix 管道式筛选：
  ```bash
  svault scan /mnt/card > candidates.txt
  exiftool -p '$Directory/$FileName' -if '$Model eq "iPhone 15"' /mnt/card > iphone.txt
  svault import /mnt/card --files-from iphone.txt
  ```
- [ ] **MTP 导入完整实现** — 当前 `svault mtp ls` 和 `svault mtp tree` 已实现并可用，但 `svault import mtp://...` 存在已知缺陷（如 `MtpFs::create_dir_all` 返回 `Unsupported`、单流传输稳定性不足等），**暂定为 browse-only，从 MTP 设备直接导入的功能尚未完成**。

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
| 2026-04-02 | `recheck` 改为基于 manifest 工作；`verify-source` 合并入 `recheck`；导入时写入 JSON manifest；E2E 更新至 65 passed | Kimi |
| 2026-04-02 | 实现 `svault add` / `reconcile`；Verify 统一使用全局 hash 配置、统一进度条和输出风格；CLI hash 参数简化为 `fast`/`secure`；E2E 新增 `test_add.py`、`test_reconcile.py`，更新 `test_verify.py`；全部 71 passed | Kimi |
| 2026-04-02 | 修复 Windows 构建错误（替换 `GetVolumeInformationW`/`CopyFileExW`）；适配 E2E 测试到 Windows（72 passed）；添加 `run.ps1` 脚本；更新测试文档 | Kimi |
| 2026-04-04 | `--strategy` 重构：移除 `auto`，默认 `reflink`，支持逗号组合；同步更新文档和测试覆盖记录；补充 Chaos 场景状态 | Kimi |
| 2026-04-04 | 新增 `history`、`background-hash`、`verify --upgrade-links` E2E 测试；修复 `conftest.py` 中 `db_query` 缺少 `commit()` 的问题；E2E 更新至 85 passed | Kimi |

---

## 运行测试

### Linux / macOS

```bash
# 所有单元测试和集成测试
cargo test

# 特定包测试
cargo test -p svault-core
cargo test -p svault-cli

# 特定模块测试
cargo test -p svault-core hash

# E2E 测试（推荐：自动使用 RAMDisk，默认 debug 构建）
cd e2e_tests && bash run.sh --verbose

# 只跑特定测试文件
cd e2e_tests && bash run.sh --verbose test_import_force.py

# 使用 release 构建跑 E2E
cd e2e_tests && bash run.sh --release --verbose
```

### Windows

```powershell
# 使用 uv 创建虚拟环境并安装依赖
cd e2e_tests
uv venv
uv pip install pytest pillow hypothesis

# 运行 E2E 测试（Windows 使用临时目录代替 RAMDisk）
.venv\Scripts\python -m pytest -v

# 或者使用 PowerShell 脚本
.\run.ps1 -Verbose

# 运行特定测试
.\run.ps1 -TestName "test_import" -Verbose
```

**Windows 测试注意事项：**
1. **不需要 RAMDisk** - Windows 上自动使用临时目录代替
2. **依赖安装** - 必须使用 `uv` 创建虚拟环境（`uv venv` + `uv pip install`）
3. **exiftool** - 需要手动安装并添加到 PATH（用于生成带 EXIF 的测试图片）
4. **编码问题** - 已修复控制台 UTF-8 编码处理
5. **路径分隔符** - 已适配 Windows 路径格式

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
