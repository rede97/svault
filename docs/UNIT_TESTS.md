# Svault 测试跟踪文档

> 本文档跟踪所有单元测试和集成测试的状态，随时更新。
>
> 最后更新：2026-08-09（事件溯源移除 + 会话日志布局）

---

## 测试概览

| 类型 | 数量 | 通过 | 失败 | 跳过 |
|------|------|------|------|------|
| 单元测试 (svault-core) | 154 | 154 | 0 | 0 |
| 单元测试 (svault-ui / svault-cli) | 6 | 6 | 0 | 2 ignored |
| Python E2E 测试 (Linux) | 165 执行 | 165 | 0 | 环境性 skip |
| FUSE 故障注入 (Linux, `fuse_tests/`) | 21 | 17 | 0 | 4（P2 待设施） |

> **2026-08-09（二）事件溯源移除 + 会话日志：**
> - 删 events 表/verify-chain（伪需求，PARKED §8）：-9 单测、-4 E2E
> - 会话日志布局 `.svault/sessions/<kind>/<ts-id>/`：plan.json fail-fast +
>   staging/ + manifest 原子写（BUG-3/4 修复）；reconcile 只报告不删
> - 单测：session 模块 6（路径/原子写/reconcile 四态）+ session_id 唯一性
> - E2E：disk_full D1/D2 重写（复制 ENOSPC=逐文件失败 exit 0）、
>   TestStagingReconcile 改写、recheck/path_compatibility 迁新布局

> **2026-08-09 import staging 原子提交（OPEN-3 闭环）：**
> - import 改走 staging → fsync → hash → 整批入库 → commit 后 rename；
>   不变量"最终路径可见 ⟹ 已完整复制+哈希+入库"
> - 启动对账：补 rename（已入库）/ 清残留（未入库）
> - +7 单测、+2 E2E（TestStagingReconcile）
> - 判据更新：failure-handling.md G1 精确化 / G7 重写 / §3.1 / §5

> **2026-08-06 E2E 套件重设计（220 → 167）：**
> - 删除 60 个冗余/零价值用例（五组，评估记录见会话与提交 `test: redesign`）；
>   修复 `test_import_dedup.py` 同名类遮蔽（3 个从不运行的死测试）
> - 合并：`test_chaos.py` → `test_import.py`（边界组）、`test_background_hash.py` → `test_verify.py`
> - 加固 6 个弱断言（config 错误信息 ×2、空格路径真实往返、hardlink inode、
>   中断后事件链校验、截断 JPEG 退出码）
> - **发现并修复真实产品缺陷**：scan 协议转义与 files-from 分词不对称——
>   空格文件名无法经 `scan | import --files-from` 导入（`pipe.rs`/`import.rs`），+3 单测 +1 E2E
> - 套件宪法（一测试一契约 / 禁止弱断言 / 覆盖归属唯一）见 tests/e2e/README.md

> **2026-08-05 FUSE 故障注入落地（08-06 精简）：**
> - 判据单一事实源：[docs/failure-handling.md](./failure-handling.md) §8
> - 基础设施：corrupt action / 运行时规则管理 / corrupt_sequence（INFRA-1/2/4）+ 6 个自验测试
> - 场景测试 11 个：P0×5（暂停中断、EIO 隔离、recheck 中断、损坏不可检出演示）、
>   P1×4（多文件暂停、源中途修改、不稳定读取、EAGAIN 内核重试）、
>   P2×2（空文件边界×2）
> - 08-06 删除 16 个无必要测试（永不实现 4 / 已有等价覆盖 7 / 冗余 5），理由见 §8.3
> - 修复 BUG-1：哈希 IO 错误误计 duplicate（`insert.rs`），+2 回归单测（分类 + manifest Failed 契约）

> **2026-04-16 架构重构变更：**
> - 移除 `test_history.py`（history 命令已移除，见 docs/PARKED.md）
> - `test_path_compatibility.py` 改用 `db dump` 查询（替代 history）
> - core 单元测试以 `cargo test -p svault-core -- --list` 为准；
>   下方逐测试表格冻结于重构前，仅保留 hash/config 等未变动模块的参考价值。

> **注意：** E2E 需在 Linux/macOS 运行（RAMDisk）。

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

> **2026-08-06 套件重设计**（220 → 167 用例，-24%）：删除 60 个冗余/零价值用例、
> 修复 `test_import_dedup.py` 同名类遮蔽（3 个从不运行的死测试）、
> 合并 `test_chaos.py`→`test_import.py`、`test_background_hash.py`→`test_verify.py`、
> 加固 6 个弱断言。套件宪法见 [tests/e2e/README.md](../tests/e2e/README.md) §测试文件说明。
> 本轮还发现并修复**真实产品缺陷**：scan 协议转义与 files-from 分词不对称
> （空格文件名无法经管线导入），见提交记录。

### 文件分工（19 个文件，158 测试函数 / 167 执行）

> 用例数为测试函数数；参数化展开后实际执行 167（如 media_formats 的 jpeg 三参数）。

| 文件 | 用例数 | 职责 |
|------|--------|------|
| `test_import.py` | 19 | 主流程、EXIF/设备/回退路径组织、CLI 交互、force、show-dup、边界（chaos 并入） |
| `test_import_dedup.py` | 12 | 身份判定矩阵：去重、冲突重命名、CRC 碰撞（已修复同名类遮蔽） |
| `test_import_recovery.py` | 9 | 幂等重跑、增量导入、修改识别、vault 内移动 |
| `test_import_interruption.py` | 7 | strace 信号中断恢复、并发修改、不可读/伪装文件 |
| `test_import_disk_full.py` | 3 | ENOSPC（loopback ext4 真实满盘） |
| `test_import_cross_fs.py` | 2 | 跨文件系统（ext4/btrfs） |
| `test_import_video_metadata.py` | 7 | 视频 creation_time、设备信息、路径组织 |
| `test_scan_import_pipeline.py` | 8 | scan → files-from 管线、空格路径往返契约 |
| `test_config_transfer.py` | 9 | 配置创建/错误处理、传输策略、hardlink 升级 |
| `test_verify.py` | 18 | verify（损坏检测/算法/摘要）、recheck、background-hash（并入）、db verify-chain |
| `test_update.py` | 6 | update 路径修正、missing 标记、dry-run |
| `test_add.py` | 10 | add 注册、去重、vault 内移动检测 |
| `test_clone.py` | 8 | clone 导出、过滤、审计事件 |
| `test_sync.py` | 9 | sync 双 vault 同步 |
| `test_media_formats.py` | 16 | 格式矩阵（base/别名/大小写）、过滤、路径模板 |
| `test_binding.py` | 6 | 复合媒体绑定 |
| `test_raw_id.py` | 10 | RAW 唯一 ID |
| `test_path_compatibility.py` | 4 | 跨平台路径格式 |
| `test_property.py` | 4 | Hypothesis 属性测试 |

### 其他核心场景

| 类别 | 描述 |
|------|------|
| Recheck | 基于 manifest 的源/vault 一致性校验（含 recheck 版恢复流程） |
| Verify | 损坏检测（bit flip/截断/missing/批量）、算法选择、摘要、恢复 |
| 配置/策略 | 配置创建与错误处理、策略链、hardlink inode 验证、升级 |

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
| **E2E 测试** | N/A | 🟢 167 passed (2026-08-06 重设计后) |

---

## 待办测试清单

### 高优先级

- [ ] `fs::capabilities_for` - 测试文件系统能力探测 (reflink/hardlink 支持检测)
- [x] `pipeline::scan` - 测试目录扫描和 vault 路径过滤 (E2E: test_scan_import_pipeline.py)
- [ ] `pipeline::insert` - 测试批量 DB 插入
- [ ] E2E: `Reporter / output` 语义测试
  说明：锁定 `--output human/json` 的 `stdout/stderr` 边界，避免 reporter 重构污染最终 JSON 输出
  方案文档：[docs/E2E_EXPANSION_PLAN.md](./E2E_EXPANSION_PLAN.md)
- [ ] E2E: `marker / 测试分组` 自检
  说明：锁定 `dedup/conflict` 等 marker 的收集语义，防止测试合并后分组漂移
  方案文档：[docs/E2E_EXPANSION_PLAN.md](./E2E_EXPANSION_PLAN.md)

### 中优先级

- [ ] `db::lookup_by_crc32c` - 测试 CRC32C 查询性能
- [ ] `db::lookup_by_hash` - 测试哈希查询
- [ ] 并发导入测试 - 多线程安全验证
- [ ] E2E: `scan -> filter -> import` 流水线补强
  说明：补空输入、全 duplicate、部分失效输入、空格/中文路径等真实用户工作流边界
  方案文档：[docs/E2E_EXPANSION_PLAN.md](./E2E_EXPANSION_PLAN.md)
- [ ] E2E: `conftest.py` 复用重构
  说明：提取高频场景 helper，减少重复 setup 与重复断言，优先迁移 `test_import_dedup.py`
  方案文档：[docs/E2E_CONFTEST_REFACTOR_PLAN.md](./E2E_CONFTEST_REFACTOR_PLAN.md)

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
