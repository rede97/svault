# Svault 测试跟踪文档

> 本文档跟踪所有单元测试和集成测试的状态，随时更新。
> 
> 最后更新：2026-04-08

---

## 测试概览

| 类型 | 数量 | 通过 | 失败 | 跳过 |
|------|------|------|------|------|
| 单元测试 (Unit) | 117 | 117 | 0 | 0 |
| Python E2E 测试 (Linux) | 242 | — | — | — |
| **总计** | — | — | — | — |

> **注意：** 
> - E2E 测试 `pytest --collect-only` 可收集到 241 个，收集阶段已验证；完整测试运行（full run）未在本轮验证。
> - 此前 Windows 测试数据未经验证，已移除。

---

## 单元测试 (Unit Tests)

单元测试位于源代码文件中（内联测试），或 `src/` 目录下的测试模块。

### hash 模块 (22 tests)

| 测试名 | 位置 | 描述 | 状态 |
|--------|------|------|------|
| `crc32c_region_reads_from_offset` | `src/hash/mod.rs` | CRC32C 从指定偏移读取 | ✅ |
| `crc32c_region_handles_larger_buf_than_file` | `src/hash/mod.rs` | CRC32C 处理缓冲区大于文件 | ✅ |
| `crc32c_region_zero_offset_reads_full` | `src/hash/mod.rs` | CRC32C 偏移 0 读取整个文件 | ✅ |
| `crc32c_tail_reads_last_bytes` | `src/hash/mod.rs` | CRC32C 读取尾部指定字节 | ✅ |
| `crc32c_tail_handles_larger_buf_than_file` | `src/hash/mod.rs` | CRC32C 尾部读取处理大缓冲区 | ✅ |
| `crc32c_region_returns_io_error_for_missing_file` | `src/hash/mod.rs` | CRC32C 文件不存在返回错误 | ✅ |
| `crc32c_tail_returns_io_error_for_missing_file` | `src/hash/mod.rs` | CRC32C 尾部读取文件不存在 | ✅ |
| `xxh3_128_file_is_deterministic` | `src/hash/mod.rs` | XXH3-128 计算确定性 | ✅ |
| `xxh3_128_file_produces_different_hashes_for_different_content` | `src/hash/mod.rs` | XXH3-128 不同内容不同哈希 | ✅ |
| `xxh3_128_file_handles_empty_file` | `src/hash/mod.rs` | XXH3-128 空文件处理 | ✅ |
| `xxh3_128_file_handles_large_file` | `src/hash/mod.rs` | XXH3-128 10MB 大文件分块 | ✅ |
| `xxh3_128_file_returns_io_error_for_missing_file` | `src/hash/mod.rs` | XXH3-128 文件不存在错误 | ✅ |
| `xxh3_digest_to_bytes_little_endian` | `src/hash/mod.rs` | Xxh3Digest 转字节序 | ✅ |
| `xxh3_digest_hex_formatting` | `src/hash/mod.rs` | Xxh3Digest hex 格式 | ✅ |
| `sha256_file_is_deterministic` | `src/hash/mod.rs` | SHA-256 计算确定性 | ✅ |
| `sha256_file_produces_different_hashes_for_different_content` | `src/hash/mod.rs` | SHA-256 不同内容不同哈希 | ✅ |
| `sha256_file_handles_empty_file` | `src/hash/mod.rs` | SHA-256 空文件处理 | ✅ |
| `sha256_file_handles_large_file` | `src/hash/mod.rs` | SHA-256 10MB 大文件分块 | ✅ |
| `sha256_file_returns_io_error_for_missing_file` | `src/hash/mod.rs` | SHA-256 文件不存在错误 | ✅ |
| `sha256_digest_to_hex_format` | `src/hash/mod.rs` | Sha256Digest hex 格式 | ✅ |
| `sha256_digest_display_trait` | `src/hash/mod.rs` | Sha256Digest Display trait | ✅ |
| `sha256_digest_to_bytes_returns_inner_array` | `src/hash/mod.rs` | Sha256Digest 转字节数组 | ✅ |

### config 模块 (23 tests)

| 测试名 | 位置 | 描述 | 状态 |
|--------|------|------|------|
| `default_config_has_expected_values` | `src/config.rs` | 默认配置值验证 | ✅ |
| `default_extensions_include_common_formats` | `src/config.rs` | 默认扩展名列表 | ✅ |
| `config_serializes_to_valid_toml` | `src/config.rs` | 配置序列化为 TOML | ✅ |
| `config_roundtrips_through_toml` | `src/config.rs` | TOML 往返测试 | ✅ |
| `parses_minimal_valid_config` | `src/config.rs` | 解析最小配置 | ✅ |
| `parses_config_with_sync_strategy_list` | `src/config.rs` | 策略列表解析 | ✅ |
| `parses_config_with_sync_strategy_comma_string` | `src/config.rs` | 逗号分隔策略解析 | ✅ |
| `parses_config_with_store_exif_true` | `src/config.rs` | store_exif 选项 | ✅ |
| `parses_config_with_custom_rename_template` | `src/config.rs` | 自定义重命名模板 | ✅ |
| `rejects_unknown_strategy` | `src/config.rs` | 拒绝未知策略 | ✅ |
| `rejects_unknown_strategy_in_string` | `src/config.rs` | 拒绝字符串中的未知策略 | ✅ |
| `rejects_missing_required_import_section` | `src/config.rs` | 拒绝缺少 import 节 | ✅ |
| `rejects_invalid_toml_syntax` | `src/config.rs` | 拒绝无效 TOML 语法 | ✅ |
| `rejects_malformed_strategy_type` | `src/config.rs` | 拒绝错误类型策略 | ✅ |
| `write_and_load_config_roundtrip` | `src/config.rs` | 配置文件写入/加载 | ✅ |
| `load_returns_error_for_missing_file` | `src/config.rs` | 缺失文件错误 | ✅ |
| `load_returns_error_for_invalid_toml` | `src/config.rs` | 无效 TOML 错误 | ✅ |
| `preserves_custom_config_after_roundtrip` | `src/config.rs` | 自定义配置保留 | ✅ |
| `hash_algorithm_display_formats_correctly` | `src/config.rs` | Display trait 格式化 | ✅ |
| `transfer_strategy_arg_converts_correctly` | `src/config.rs` | 策略参数转换 | ✅ |
| `sync_strategy_converts_to_transfer_strategies` | `src/config.rs` | SyncStrategy 转换 | ✅ |
| `transfer_strategy_arg_roundtrips_through_config_toml` | `src/config.rs` | 策略序列化往返 | ✅ |
| `transfer_strategy_case_insensitive_in_config` | `src/config.rs` | 策略大小写不敏感 | ✅ |

### db 模块 (11 tests)

| 测试名 | 位置 | 描述 | 状态 |
|--------|------|------|------|
| `db_open_in_memory_creates_valid_db` | `src/db/mod.rs` | 内存数据库创建 | ✅ |
| `db_open_in_memory_is_isolated` | `src/db/mod.rs` | 内存数据库隔离性 | ✅ |
| `last_event_hash_returns_genesis_for_empty_db` | `src/db/mod.rs` | 空库返回 genesis hash | ✅ |
| `append_event_creates_valid_chain` | `src/db/mod.rs` | 事件追加和链构建 | ✅ |
| `verify_chain_passes_for_valid_chain` | `src/db/mod.rs` | 验证有效链通过 | ✅ |
| `verify_chain_detects_tampering` | `src/db/mod.rs` | 检测篡改事件 | ✅ |
| `get_events_returns_events_in_descending_order` | `src/db/mod.rs` | 事件倒序返回 | ✅ |
| `get_events_filters_by_event_type` | `src/db/mod.rs` | 按事件类型过滤 | ✅ |
| `get_events_respects_limit` | `src/db/mod.rs` | 限制返回数量 | ✅ |
| `compute_event_hash_is_deterministic` | `src/db/mod.rs` | 事件哈希确定性 | ✅ |
| `compute_event_hash_changes_with_input` | `src/db/mod.rs` | 不同输入不同哈希 | ✅ |

### db/dump 模块 (3 tests)

| 测试名 | 位置 | 描述 | 状态 |
|--------|------|------|------|
| `test_format_value` | `src/db/dump.rs` | SQL 值格式化 | ✅ |
| `test_list_tables_empty_db` | `src/db/dump.rs` | 空数据库表列表 | ✅ |
| `test_list_tables_with_data` | `src/db/dump.rs` | 有数据时表列表 | ✅ |

### db/stats 模块 (2 tests)

| 测试名 | 位置 | 描述 | 状态 |
|--------|------|------|------|
| `test_format_bytes` | `src/db/stats.rs` | 字节格式化 | ✅ |
| `test_format_count` | `src/db/stats.rs` | 数字千分位格式化 | ✅ |

### import 模块 (14 tests)

| 测试名 | 位置 | 描述 | 状态 |
|--------|------|------|------|
| `test_unix_now_ms_increases` | `src/import/utils.rs` | 时间戳递增测试 | ✅ |
| `test_session_id_format` | `src/import/utils.rs` | Session ID 格式测试 | ✅ |
| `test_resolve_dest_path` | `src/import/path.rs` | 路径模板解析 | ✅ |
| `test_resolve_dest_path_no_device` | `src/import/path.rs` | 无设备路径解析 | ✅ |
| `test_file_status_equality` | `src/import/mod.rs` | FileStatus 相等性 | ✅ |
| `secs_to_ymd_epoch` | `src/import/exif.rs` | Unix epoch 日期转换 | ✅ |
| `secs_to_ymd_specific_known_dates` | `src/import/exif.rs` | 已知日期转换 | ✅ |
| `secs_to_ymd_year_boundaries` | `src/import/exif.rs` | 跨年日期边界 | ✅ |
| `secs_to_ymd_negative_timestamp` | `src/import/exif.rs` | 负时间戳（1970前） | ✅ |
| `parse_exif_datetime_valid` | `src/import/exif.rs` | EXIF 日期解析 | ✅ |
| `parse_exif_datetime_epoch` | `src/import/exif.rs` | EXIF epoch 日期 | ✅ |
| `parse_exif_datetime_too_short` | `src/import/exif.rs` | 短字符串处理 | ✅ |
| `parse_exif_datetime_handles_edge_cases` | `src/import/exif.rs` | 边界情况处理 | ✅ |
| `ymd_days_round_trip` | `src/import/exif.rs` | YMD ↔ 天数往返 | ✅ |

### fs 模块 (5 tests)

| 测试名 | 位置 | 描述 | 状态 |
|--------|------|------|------|
| `transfer_with_empty_strategy_list_uses_stream_copy_fallback` | `src/fs.rs` | 空策略列表兜底 | ✅ |
| `transfer_creates_parent_directories` | `src/fs.rs` | 自动创建父目录 | ✅ |
| `transfer_preserves_content_integrity` | `src/fs.rs` | 内容完整性保持 | ✅ |
| `empty_source_file_transfers_successfully` | `src/fs.rs` | 空文件传输 | ✅ |
| `large_file_transfers_successfully` | `src/fs.rs` | 大文件传输 (10MB) | ✅ |

### lock 模块 (1 test)

| 测试名 | 位置 | 描述 | 状态 |
|--------|------|------|------|
| `test_lock_acquire_and_release` | `src/lock.rs` | Vault 咨询锁获取与释放 | ✅ |

---

## Python E2E 测试

端到端测试位于 `tests/e2e/`，使用 `pytest` + RAMDisk 隔离测试环境。

### Import E2E 测试分工（5个核心文件 + 环境专用文件）

**核心行为文件（5个，68个用例）：**

| 文件 | 用例数 | 职责范围 | 一句话说明 |
|------|--------|----------|------------|
| `test_import.py` | 20 | 常规导入、EXIF、CLI 语义 | 主流程 + 交互行为 |
| `test_import_recovery.py` | 16 | 幂等性、增量导入、恢复 | 重试与恢复策略 |
| `test_import_interruption.py` | 12 | **信号中断**（strace inject） | 故障注入与一致性 |
| `test_import_dedup.py` | 16 | 去重 + 冲突（已合并 conflict） | 身份判定矩阵 |
| `test_chaos.py` | 4 | 边界情况 | 异常输入与边缘场景 |

**环境专用文件（保持独立，不合并）：**

| 文件 | 职责 | 独立原因 |
|------|------|----------|
| `test_import_disk_full.py` | ENOSPC 处理 | 依赖磁盘容量控制 |
| `test_import_cross_fs.py` | ext4/btrfs 差异 | 依赖多文件系统环境 |
| `test_import_video_metadata.py` | 视频元数据提取 | 独立复杂度，避免主文件膨胀 |
| `test_scan_import_pipeline.py` | scan -> filter -> import 流水线 | 独立接口测试 |
| `test_config_transfer.py` | 传输策略配置 | 配置域，非 import 主行为 |

**合并历史：**
- 2026-04-08: `test_import_conflict.py` → `test_import_dedup.py`（5个用例迁移）

### 其他核心场景

| 类别 | 数量 | 描述 |
|------|------|------|
| Recheck | 6 | 基于 manifest 的源/vault 一致性校验 |
| Add/Reconcile | 6 | 注册已有文件、恢复移动的文件 |
| Verify | 12 | 完整性验证、bit flip 检测、hardlink 升级 |
| 媒体格式 | 19 | JPG/PNG/TIFF/HEIC/DNG/MP4/MOV/MTS |
| Live Photo/RAW+JPEG | 6 | 复合媒体绑定检测与导入 |
| 视频元数据 | 6 | creation_time 提取、设备信息 |
| 磁盘空间 | 4 | ENOSPC 处理、事务一致性 |
| 配置/策略 | 13 | 传输策略 fallback 链验证 |
| 跨文件系统 | 4 | ext4/btrfs 不同组合 |
| History | 6 | 事件查询、过滤、JSON 输出 |
| 并发/锁 | 4 | 进程锁、并发导入 |
| Scan + Filter + Import | 10 | 扫描过滤导入流水线 |

---

## 测试覆盖率目标

| 模块 | 目标 | 当前状态 |
|------|------|----------|
| hash | 90% | 🟢 已达成 (22 tests) |
| config | 90% | 🟢 已达成 (24 tests) |
| db | 85% | 🟢 已达成 (14 tests) |
| fs | 80% | 🟢 已达成 (5 tests) |
| import | 85% | 🟢 已达成 (14 tests) |
| pipeline | 80% | 🟡 待补充 |
| **E2E 测试** | N/A | 🟢 208 passed |

---

## 待办测试清单

### 高优先级

- [ ] `fs::capabilities_for` - 测试文件系统能力探测 (reflink/hardlink 支持检测)
- [x] `pipeline::scan` - 测试目录扫描和 vault 路径过滤 (E2E: test_scan_import_pipeline.py)
- [ ] `pipeline::insert` - 测试批量 DB 插入
- [ ] E2E: `Reporter / output` 语义测试
  说明：锁定 `--output human/json` 的 `stdout/stderr` 边界，避免 reporter 重构污染最终 JSON 输出
  方案文档：[docs/E2E_EXPANSION_PLAN.md](/home/mxq/Codes/svault/docs/E2E_EXPANSION_PLAN.md)
- [ ] E2E: `marker / 测试分组` 自检
  说明：锁定 `dedup/conflict` 等 marker 的收集语义，防止测试合并后分组漂移
  方案文档：[docs/E2E_EXPANSION_PLAN.md](/home/mxq/Codes/svault/docs/E2E_EXPANSION_PLAN.md)

### 中优先级

- [ ] `db::lookup_by_crc32c` - 测试 CRC32C 查询性能
- [ ] `db::lookup_by_hash` - 测试哈希查询
- [ ] 并发导入测试 - 多线程安全验证
- [ ] E2E: `scan -> filter -> import` 流水线补强
  说明：补空输入、全 duplicate、部分失效输入、空格/中文路径等真实用户工作流边界
  方案文档：[docs/E2E_EXPANSION_PLAN.md](/home/mxq/Codes/svault/docs/E2E_EXPANSION_PLAN.md)
- [ ] E2E: `conftest.py` 复用重构
  说明：提取高频场景 helper，减少重复 setup 与重复断言，优先迁移 `test_import_dedup.py`
  方案文档：[docs/E2E_CONFTEST_REFACTOR_PLAN.md](/home/mxq/Codes/svault/docs/E2E_CONFTEST_REFACTOR_PLAN.md)

### 低优先级 (集成测试)

- [ ] 大文件（>4GB）处理测试
- [ ] 各种文件系统（xfs）行为测试
- [ ] 网络文件系统（NFS/SMB）行为测试

---

## 运行测试

### Linux / macOS

```bash
# 所有单元测试
cargo test

# 特定模块测试
cargo test -p svault-core hash
cargo test -p svault-core config

# E2E 测试（推荐：自动使用 RAMDisk）
cd tests/e2e && bash run.sh --verbose

# 只跑特定测试文件
cd tests/e2e && bash run.sh test_import_force.py

# 使用 release 构建跑 E2E
cd tests/e2e && bash run.sh --release --verbose
```

### Windows

```powershell
# 使用 uv 创建虚拟环境并安装依赖
cd tests/e2e
uv venv
uv pip install pytest pillow hypothesis

# 运行 E2E 测试
.venv\Scripts\python -m pytest -v

# 或者使用 PowerShell 脚本
.\run.ps1 -Verbose
```

---

## 更新记录

| 日期 | 更新内容 |
|------|----------|
| 2026-03-31 | 初始版本：记录测试状态 |
| 2026-04-02 | 文件系统模块重构测试；添加 recheck/re-import E2E；E2E 64 passed |
| 2026-04-02 | 添加 `add`/`reconcile` E2E；Verify 统一；E2E 71 passed |
| 2026-04-02 | Windows 适配；E2E 72 passed |
| 2026-04-04 | 策略重构；`history`/`background-hash`；E2E 85 passed |
| 2026-04-04 | 补充 hash/config/fs/import 单元测试；总单元测试 117 |
| 2026-04-04 | 视频元数据、Live Photo/RAW+JPEG、磁盘空间 E2E 测试 |
| 2026-04-05 | E2E 测试参数化重构；删除重复代码 ~110 行 |
| 2026-04-05 | Pipeline 架构实现；CLI 拆分为命令模块；E2E 198 passed |
| 2026-04-06 | 添加 scan + filter + import 流水线 E2E 测试 (10 tests) |
| 2026-04-08 | Import E2E 测试整理：删除重复用例 7 个，conflict 合并至 dedup，明确 5+5 文件分工 |
