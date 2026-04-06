# Svault 单元测试跟踪文档

> 本文档跟踪所有单元测试和集成测试的状态，随时更新。
> 
> 最后更新：2026-04-05

---

## 测试概览

| 类型 | 数量 | 通过 | 失败 | 跳过 |
|------|------|------|------|------|
| 单元测试 (Unit) | 117 | 117 | 0 | 0 |
| 集成测试 (Integration) | 0 | 0 | 0 | 0 |
| Python E2E 测试 (Linux) | 198 | 190 | 0 | 8 |
| Python E2E 测试 (Windows) | 198 | 190 | 0 | 8 |
| **总计** | **513** | **505** | **0** | **16** |

---

## 单元测试 (Unit Tests)

单元测试位于源代码文件中（内联测试），或 `src/` 目录下的测试模块。

### hash 模块

| 测试名 | 位置 | 描述 | 状态 | 备注 |
|--------|------|------|------|------|
| `crc32c_region_reads_from_offset` | `src/hash/mod.rs` | CRC32C 从指定偏移读取 | ✅ PASS | |
| `crc32c_region_handles_larger_buf_than_file` | `src/hash/mod.rs` | CRC32C 处理缓冲区大于文件 | ✅ PASS | |
| `crc32c_region_zero_offset_reads_full` | `src/hash/mod.rs` | CRC32C 偏移 0 读取整个文件 | ✅ PASS | |
| `crc32c_tail_reads_last_bytes` | `src/hash/mod.rs` | CRC32C 读取尾部指定字节 | ✅ PASS | |
| `crc32c_tail_handles_larger_buf_than_file` | `src/hash/mod.rs` | CRC32C 尾部读取处理大缓冲区 | ✅ PASS | |
| `crc32c_region_returns_io_error_for_missing_file` | `src/hash/mod.rs` | CRC32C 文件不存在返回错误 | ✅ PASS | |
| `crc32c_tail_returns_io_error_for_missing_file` | `src/hash/mod.rs` | CRC32C 尾部读取文件不存在 | ✅ PASS | |
| `xxh3_128_file_is_deterministic` | `src/hash/mod.rs` | XXH3-128 计算确定性 | ✅ PASS | |
| `xxh3_128_file_produces_different_hashes_for_different_content` | `src/hash/mod.rs` | XXH3-128 不同内容不同哈希 | ✅ PASS | |
| `xxh3_128_file_handles_empty_file` | `src/hash/mod.rs` | XXH3-128 空文件处理 | ✅ PASS | |
| `xxh3_128_file_handles_large_file` | `src/hash/mod.rs` | XXH3-128 10MB 大文件分块 | ✅ PASS | |
| `xxh3_128_file_returns_io_error_for_missing_file` | `src/hash/mod.rs` | XXH3-128 文件不存在错误 | ✅ PASS | |
| `xxh3_digest_to_bytes_little_endian` | `src/hash/mod.rs` | Xxh3Digest 转字节序 | ✅ PASS | |
| `xxh3_digest_hex_formatting` | `src/hash/mod.rs` | Xxh3Digest hex 格式 | ✅ PASS | |
| `sha256_file_is_deterministic` | `src/hash/mod.rs` | SHA-256 计算确定性 | ✅ PASS | |
| `sha256_file_produces_different_hashes_for_different_content` | `src/hash/mod.rs` | SHA-256 不同内容不同哈希 | ✅ PASS | |
| `sha256_file_handles_empty_file` | `src/hash/mod.rs` | SHA-256 空文件处理 | ✅ PASS | |
| `sha256_file_handles_large_file` | `src/hash/mod.rs` | SHA-256 10MB 大文件分块 | ✅ PASS | |
| `sha256_file_returns_io_error_for_missing_file` | `src/hash/mod.rs` | SHA-256 文件不存在错误 | ✅ PASS | |
| `sha256_digest_to_hex_format` | `src/hash/mod.rs` | Sha256Digest hex 格式 | ✅ PASS | |
| `sha256_digest_display_trait` | `src/hash/mod.rs` | Sha256Digest Display trait | ✅ PASS | |
| `sha256_digest_to_bytes_returns_inner_array` | `src/hash/mod.rs` | Sha256Digest 转字节数组 | ✅ PASS | |

### config 模块

| 测试名 | 位置 | 描述 | 状态 | 备注 |
|--------|------|------|------|------|
| `default_config_has_expected_values` | `src/config.rs` | 默认配置值验证 | ✅ PASS | |
| `default_extensions_include_common_formats` | `src/config.rs` | 默认扩展名列表 | ✅ PASS | |
| `config_serializes_to_valid_toml` | `src/config.rs` | 配置序列化为 TOML | ✅ PASS | |
| `config_roundtrips_through_toml` | `src/config.rs` | TOML 往返测试 | ✅ PASS | |
| `parses_minimal_valid_config` | `src/config.rs` | 解析最小配置 | ✅ PASS | |
| `parses_config_with_sync_strategy_list` | `src/config.rs` | 策略列表解析 | ✅ PASS | |
| `parses_config_with_sync_strategy_comma_string` | `src/config.rs` | 逗号分隔策略解析 | ✅ PASS | |
| `parses_config_with_store_exif_true` | `src/config.rs` | store_exif 选项 | ✅ PASS | |
| `parses_config_with_custom_rename_template` | `src/config.rs` | 自定义重命名模板 | ✅ PASS | |
| `rejects_unknown_hash_algorithm` | `src/config.rs` | 拒绝未知哈希算法 | ✅ PASS | 错误处理 |
| `rejects_unknown_strategy` | `src/config.rs` | 拒绝未知策略 | ✅ PASS | 错误处理 |
| `rejects_unknown_strategy_in_string` | `src/config.rs` | 拒绝字符串中的未知策略 | ✅ PASS | 错误处理 |
| `rejects_missing_required_import_section` | `src/config.rs` | 拒绝缺少 import 节 | ✅ PASS | 错误处理 |
| `rejects_invalid_toml_syntax` | `src/config.rs` | 拒绝无效 TOML 语法 | ✅ PASS | 错误处理 |
| `rejects_malformed_strategy_type` | `src/config.rs` | 拒绝错误类型策略 | ✅ PASS | 错误处理 |
| `write_and_load_config_roundtrip` | `src/config.rs` | 配置文件写入/加载 | ✅ PASS | |
| `load_returns_error_for_missing_file` | `src/config.rs` | 缺失文件错误 | ✅ PASS | |
| `load_returns_error_for_invalid_toml` | `src/config.rs` | 无效 TOML 错误 | ✅ PASS | |
| `preserves_custom_config_after_roundtrip` | `src/config.rs` | 自定义配置保留 | ✅ PASS | |
| `hash_algorithm_display_formats_correctly` | `src/config.rs` | Display trait 格式化 | ✅ PASS | |
| `transfer_strategy_arg_converts_correctly` | `src/config.rs` | 策略参数转换 | ✅ PASS | |
| `sync_strategy_converts_to_transfer_strategies` | `src/config.rs` | SyncStrategy 转换 | ✅ PASS | |
| `transfer_strategy_arg_roundtrips_through_config_toml` | `src/config.rs` | 策略序列化往返 | ✅ PASS | |
| `transfer_strategy_case_insensitive_in_config` | `src/config.rs` | 策略大小写不敏感 | ✅ PASS | |

### db 模块

| 测试名 | 位置 | 描述 | 状态 | 备注 |
|--------|------|------|------|------|
| `db_open_in_memory_creates_valid_db` | `src/db/mod.rs` | 内存数据库创建 | ✅ PASS | |
| `db_open_in_memory_is_isolated` | `src/db/mod.rs` | 内存数据库隔离性 | ✅ PASS | |
| `last_event_hash_returns_genesis_for_empty_db` | `src/db/mod.rs` | 空库返回 genesis hash | ✅ PASS | |
| `append_event_creates_valid_chain` | `src/db/mod.rs` | 事件追加和链构建 | ✅ PASS | |
| `verify_chain_passes_for_valid_chain` | `src/db/mod.rs` | 验证有效链通过 | ✅ PASS | |
| `verify_chain_detects_tampering` | `src/db/mod.rs` | 检测篡改事件 | ✅ PASS | |
| `get_events_returns_events_in_descending_order` | `src/db/mod.rs` | 事件倒序返回 | ✅ PASS | |
| `get_events_filters_by_event_type` | `src/db/mod.rs` | 按事件类型过滤 | ✅ PASS | |
| `get_events_respects_limit` | `src/db/mod.rs` | 限制返回数量 | ✅ PASS | |
| `compute_event_hash_is_deterministic` | `src/db/mod.rs` | 事件哈希确定性 | ✅ PASS | |
| `compute_event_hash_changes_with_input` | `src/db/mod.rs` | 不同输入不同哈希 | ✅ PASS | |
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
| `secs_to_ymd_epoch` | `src/import/exif.rs` | Unix epoch 日期转换 | ✅ PASS | 内联测试 |
| `secs_to_ymd_specific_known_dates` | `src/import/exif.rs` | 已知日期转换 | ✅ PASS | 内联测试 |
| `secs_to_ymd_year_boundaries` | `src/import/exif.rs` | 跨年日期边界 | ✅ PASS | 内联测试 |
| `secs_to_ymd_negative_timestamp` | `src/import/exif.rs` | 负时间戳（1970前） | ✅ PASS | 内联测试 |
| `parse_exif_datetime_valid` | `src/import/exif.rs` | EXIF 日期解析 | ✅ PASS | 内联测试 |
| `parse_exif_datetime_epoch` | `src/import/exif.rs` | EXIF epoch 日期 | ✅ PASS | 内联测试 |
| `parse_exif_datetime_too_short` | `src/import/exif.rs` | 短字符串处理 | ✅ PASS | 内联测试 |
| `parse_exif_datetime_handles_edge_cases` | `src/import/exif.rs` | 边界情况处理 | ✅ PASS | 内联测试 |
| `ymd_days_round_trip` | `src/import/exif.rs` | YMD ↔ 天数往返 | ✅ PASS | 内联测试 |
| `ymd_to_days_behavioral_test` | `src/import/exif.rs` | ymd_to_days 行为 | ✅ PASS | 内联测试 |
| `exif_ascii_first_extracts_string` | `src/import/exif.rs` | EXIF ASCII 提取 | ✅ PASS | 内联测试 |
| `exif_ascii_first_trims_nulls` | `src/import/exif.rs` | EXIF 空字符修剪 | ✅ PASS | 内联测试 |
| `exif_ascii_first_empty_vec` | `src/import/exif.rs` | 空向量处理 | ✅ PASS | 内联测试 |
| `exif_ascii_first_non_ascii` | `src/import/exif.rs` | 非 ASCII 值处理 | ✅ PASS | 内联测试 |

### vfs 模块

| 测试名 | 位置 | 描述 | 状态 | 备注 |
|--------|------|------|------|------|
| *待添加* | `src/vfs/system.rs` | 文件系统能力探测 | 🔲 TODO | reflink/hardlink |
| *待添加* | `src/vfs/system.rs` | 目录遍历（含 `.svault` 剪枝） | 🔲 TODO | |
| `transfer_uses_first_strategy_when_it_succeeds` | `src/vfs/transfer.rs` | 首策略成功时使用 | ✅ PASS | fallback 链 |
| `transfer_falls_back_to_second_strategy_when_first_fails` | `src/vfs/transfer.rs` | 首策略失败时回退 | ✅ PASS | fallback 链 |
| `transfer_falls_back_to_stream_copy_when_all_else_fails` | `src/vfs/transfer.rs` | 全部失败时回退 stream_copy | ✅ PASS | fallback 链 |
| `stream_copy_is_always_final_fallback` | `src/vfs/transfer.rs` | stream_copy 始终兜底 | ✅ PASS | fallback 链 |
| `transfer_with_empty_strategy_list_uses_stream_copy_fallback` | `src/vfs/transfer.rs` | 空策略列表兜底 | ✅ PASS | fallback 链 |
| `transfer_creates_parent_directories` | `src/vfs/transfer.rs` | 自动创建父目录 | ✅ PASS | |
| `transfer_preserves_content_integrity` | `src/vfs/transfer.rs` | 内容完整性保持 | ✅ PASS | 二进制数据 |
| `empty_source_file_transfers_successfully` | `src/vfs/transfer.rs` | 空文件传输 | ✅ PASS | |
| `large_file_transfers_successfully` | `src/vfs/transfer.rs` | 大文件传输 (10MB) | ✅ PASS | |

### lock 模块

| 测试名 | 位置 | 描述 | 状态 | 备注 |
|--------|------|------|------|------|
| `test_lock_acquire_and_release` | `src/lock.rs` | Vault 咨询锁获取与释放 | ✅ PASS | 内联测试 |

---

## Python E2E 测试

端到端测试位于 `tests/e2e/`，使用 `pytest` + RAMDisk 隔离测试环境。

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

## 集成测试计划 (Integration Tests)

> 集成测试验证多个模块协同工作的场景，需要真实或模拟的外部依赖。
> 与单元测试不同，集成测试通常需要特殊环境准备，运行时间较长。

### 测试范围定义

| 类别 | 描述 | 依赖要求 | 运行频率 |
|------|------|----------|----------|
| **文件系统集成** | 验证在不同文件系统上的行为一致性 | 需要挂载多种文件系统 | 每周/发布前 |
| **多 Vault 同步** | 验证 vault 间数据迁移和同步 | 多个 vault 实例 | 每次发布 |
| **MTP 设备** | 验证从物理设备导入 | 需要真实 MTP 设备 | 手动触发 |
| **网络文件系统** | 验证 NFS/SMB 上的行为 | 需要网络存储 | 每周 |

### 1. 文件系统集成测试

#### 目标文件系统

| 文件系统 | 特性 | 测试重点 | 状态 |
|----------|------|----------|------|
| ext4 | Linux 默认，无 reflink | hardlink/copy 回退 | ✅ 参数化测试 |
| btrfs | 支持 reflink | reflink 原子复制 | ✅ 参数化测试 |
| xfs | 企业级，支持 reflink (v5) | reflink/copy 混合 | 🔲 待设计 |
| tmpfs | 内存文件系统 | 已用于 E2E | ✅ 在用 |

#### 测试场景设计

**已实现：跨文件系统导入测试** (`test_import_cross_fs.py`)

```python
@pytest.mark.parametrize(
    "source_fs,vault_fs,strategy",
    [
        ("ext4", "ext4", "copy"),  # X1: ext4 → ext4 使用 copy
        ("btrfs", "btrfs", "reflink,copy"),  # X2: btrfs → btrfs 使用 reflink
    ],
)
def test_cross_fs_import(...):
    """X1-X2: 参数化跨文件系统导入测试"""
```

**待实现单元测试：**
```rust
// 示例：test_integration_fs_relink.rs
#[test]
#[ignore = "requires btrfs mount at /mnt/btrfs-test"]
fn test_reflink_on_btrfs() {
    // 1. 在 btrfs 上创建 vault
    // 2. 导入文件
    // 3. 验证使用 reflink（检查文件共享相同物理块）
    // 4. 修改源文件，验证 CoW 行为
}

#[test]
#[ignore = "requires ext4 mount at /mnt/ext4-test"]
fn test_hardlink_fallback_on_ext4() {
    // 1. 在 ext4 上创建 vault
    // 2. 导入文件（策略: reflink,hardlink,copy）
    // 3. 验证 reflink 失败后使用 hardlink
    // 4. 验证同一源文件多次导入使用 hardlink
}
```

#### 环境准备脚本 (img + loopback 方案)

**方案优势：**
- 无需额外磁盘分区
- 可在 CI/CD 环境中运行（需要 privileged 模式）
- 精确控制文件系统大小和特性
- 测试完成后自动清理

```bash
#!/bin/bash
# tests/setup_fs_integration.sh

set -e

# 配置
TEST_BASE="/tmp/svault-fs-test"
IMG_SIZE_MB=512

cleanup() {
    echo "Cleaning up..."
    for fs in ext4 xfs btrfs; do
        mountpoint -q "$TEST_BASE/$fs" && sudo umount "$TEST_BASE/$fs" || true
        rm -f "$TEST_BASE/$fs.img"
    done
    rm -rf "$TEST_BASE"
}

setup_ext4() {
    echo "=== Setting up ext4 (no reflink) ==="
    mkdir -p "$TEST_BASE/ext4"
    dd if=/dev/zero of="$TEST_BASE/ext4.img" bs=1M count=$IMG_SIZE_MB status=progress
    mkfs.ext4 "$TEST_BASE/ext4.img"
    sudo mount -o loop "$TEST_BASE/ext4.img" "$TEST_BASE/ext4"
    sudo chown $(id -u):$(id -g) "$TEST_BASE/ext4"
    echo "ext4 ready at $TEST_BASE/ext4"
}

setup_xfs() {
    echo "=== Setting up XFS (with reflink) ==="
    mkdir -p "$TEST_BASE/xfs"
    dd if=/dev/zero of="$TEST_BASE/xfs.img" bs=1M count=$IMG_SIZE_MB status=progress
    # XFS v5 supports reflink (since Linux 4.9)
    mkfs.xfs -m reflink=1 "$TEST_BASE/xfs.img"
    sudo mount -o loop "$TEST_BASE/xfs.img" "$TEST_BASE/xfs"
    sudo chown $(id -u):$(id -g) "$TEST_BASE/xfs"
    echo "XFS ready at $TEST_BASE/xfs"
}

setup_btrfs() {
    echo "=== Setting up btrfs (with reflink) ==="
    mkdir -p "$TEST_BASE/btrfs"
    dd if=/dev/zero of="$TEST_BASE/btrfs.img" bs=1M count=$IMG_SIZE_MB status=progress
    mkfs.btrfs "$TEST_BASE/btrfs.img"
    sudo mount -o loop "$TEST_BASE/btrfs.img" "$TEST_BASE/btrfs"
    sudo chown $(id -u):$(id -g) "$TEST_BASE/btrfs"
    echo "btrfs ready at $TEST_BASE/btrfs"
}

# 跨文件系统导入测试：不同文件系统作为 source 和 vault
setup_cross_fs() {
    echo "=== Setting up cross-filesystem test ==="
    # Source on ext4, vault on xfs
    setup_ext4
    mv "$TEST_BASE/ext4" "$TEST_BASE/source"
    mv "$TEST_BASE/ext4.img" "$TEST_BASE/source.img"
    setup_xfs
    mv "$TEST_BASE/xfs" "$TEST_BASE/vault"
    mv "$TEST_BASE/xfs.img" "$TEST_BASE/vault.img"
    echo "Cross-fs ready: source (ext4) -> vault (xfs)"
}

# 主入口
case "${1:-all}" in
    ext4) setup_ext4 ;;
    xfs) setup_xfs ;;
    btrfs) setup_btrfs ;;
    cross) setup_cross_fs ;;
    cleanup) cleanup ;;
    all)
        cleanup 2>/dev/null || true
        mkdir -p "$TEST_BASE"
        setup_ext4
        setup_xfs
        setup_btrfs
        echo ""
        echo "All filesystems ready:"
        df -h "$TEST_BASE"/* 2>/dev/null || true
        ;;
    *)
        echo "Usage: $0 [ext4|xfs|btrfs|cross|cleanup|all]"
        exit 1
        ;;
esac
```

**Docker CI 支持：**

```dockerfile
# Dockerfile.fs-test
FROM ubuntu:22.04

RUN apt-get update && apt-get install -y \
    btrfs-progs xfsprogs e2fsprogs \
    util-linux mount \
    && rm -rf /var/lib/apt/lists/*

# 需要 privileged 模式运行
# docker run --privileged -v $(pwd):/workspace svault-fs-test
```

### 2. 多 Vault 同步测试

#### 测试场景

| ID | 场景 | 描述 | 验证点 |
|----|------|------|--------|
| mv1 | vault-to-vault 迁移 | 将文件从 vault A 迁移到 vault B | DB 记录、文件内容完整性 |
| mv2 | 增量同步 | vault A 更新后同步到 vault B | 仅传输差异文件 |
| mv3 | 冲突解决 | 同一文件在两个 vault 被不同修改 | 冲突检测、手动/自动解决 |
| mv4 | 哈希链验证跨 vault | 导出的 manifest 在目标 vault 验证 | 事件哈希链完整性 |

#### 技术方案

```rust
// tests/integration/multi_vault.rs
use svault_core::db::Db;
use tempfile::TempDir;

struct VaultEnv {
    root: TempDir,
    db: Db,
}

fn test_vault_migration() {
    // Setup source vault
    let source = create_vault_with_files(&["a.jpg", "b.png"]);
    
    // Setup target vault
    let target = create_empty_vault();
    
    // Export manifest from source
    let manifest = source.export_manifest();
    
    // Import to target
    target.import_from_manifest(&manifest);
    
    // Verify
    assert_eq!(target.file_count(), 2);
    assert!(target.verify_all_hashes().is_ok());
}
```

### 3. MTP 设备集成测试

#### 测试矩阵

| 设备类型 | 连接方式 | 测试重点 | 状态 |
|----------|----------|----------|------|
| Android 手机 | USB | 大文件传输稳定性 | 🔲 待实施 |
| iPhone | USB | 特殊路径处理 | 🔲 待实施 |
| 数码相机 | USB PTP/MTP | EXIF 保留 | 🔲 待实施 |

#### 测试前提条件

```rust
// tests/integration/mtp_device.rs
#[test]
#[ignore = "requires physical MTP device"]
fn test_mtp_import_real_device() {
    // 1. 检测设备连接
    let devices = list_mtp_devices();
    assume!(!devices.is_empty(), "No MTP device connected");
    
    // 2. 导入测试
    let result = import_from_mtp(&devices[0], "DCIM/Camera");
    
    // 3. 验证
    assert!(result.imported > 0);
    assert!(result.failed == 0);
}
```

### 4. 网络文件系统测试

#### 4.1 测试矩阵（单系统同构）

| 类型 | 服务端 | 客户端 | 关注点 |
|------|--------|--------|--------|
| NFS v4 | Linux server | Linux client | 锁行为、大文件 |
| SMB/CIFS | Samba | Linux | 权限映射、文件名编码 |
| SSHFS | OpenSSH | sshfs | 延迟容忍 |

#### 4.2 测试矩阵（跨系统异构）⭐ 新增

| 场景 | 服务端 | 客户端 | 测试重点 | 风险等级 |
|------|--------|--------|----------|----------|
| **NFS Linux→Linux** | Ubuntu/Debian | CentOS/RHEL | 兼容性、默认配置差异 | 🟡 中 |
| **NFS Linux→macOS** | Linux NFS | macOS (NFSv3/4) | 锁机制、扩展属性 | 🔴 高 |
| **Samba Windows→Linux** | Windows 10/11/Server | Linux (cifs) | 权限映射、文件名编码、符号链接 | 🔴 高 |
| **Samba Windows→macOS** | Windows | macOS (SMB2/3) | 资源分支、元数据 | 🔴 高 |
| **Samba Linux→Windows** | Samba | Windows 10/11 | ACL 映射、长路径 | 🟡 中 |

#### 4.3 跨系统测试重点

**NFS 跨系统场景：**

| 测试项 | Linux→Linux | Linux→macOS | 说明 |
|--------|-------------|-------------|------|
| 文件锁 (flock) | ✅ 支持 | ⚠️ 有限支持 | macOS NFS 客户端锁行为不同 |
| 硬链接 | ✅ 支持 | ✅ 支持 | 但计数可能不一致 |
| 符号链接 | ✅ 支持 | ✅ 支持 | 相对/绝对链接 |
| 扩展属性 (xattr) | ⚠️ 有限 | ❌ 不支持 | macOS 不保留 Linux xattr |
| 文件名编码 | UTF-8 | UTF-8 NFD | macOS 使用 NFD 规范化 |
| 时间戳精度 | nanosecond | second | macOS 仅秒级精度 |

**Samba 跨系统场景：**

| 测试项 | Windows→Linux | Windows→macOS | 说明 |
|--------|---------------|---------------|------|
| 权限映射 | ⚠️ 复杂 | ⚠️ 复杂 | Windows ACL ↔ Unix mode |
| 文件名大小写 | 保留但不敏感 | 保留但不敏感 | 与 Linux 默认不同 |
| 保留字符 | `:"\|<>*?` | `:"\|<>*?` | Linux 允许，Windows 不允许 |
| 流/资源分支 | ❌ 不支持 | ✅ 支持 | macOS 扩展属性通过 SMB 流传输 |
| 符号链接 | ⚠️ 有限 | ⚠️ 有限 | Windows 符号链接需要特殊权限 |

#### 4.4 测试场景代码示例

```rust
// tests/integration/cross_system_nfs.rs

/// Linux NFS Server + macOS Client 测试
#[test]
#[ignore = "requires NFS mount from Linux server at /mnt/nfs-linux-mac"]
fn test_nfs_linux_to_macos_import() {
    // 重点：macOS NFS 客户端使用 NFD 规范化，可能引发文件名不匹配
    let vault = Vault::init("/mnt/nfs-linux-mac/vault");
    
    // 创建包含组合字符的文件名（如 é = e + ́）
    let filename_nfc = "caf\u{e9}.jpg";  // NFC: é 是单个字符
    let filename_nfd = "caf\u{65}\u{301}.jpg";  // NFD: e + ́ 两个字符
    
    // 在 Linux 端创建 NFC 文件名
    create_source_file(&format!("/nfs/server/{}", filename_nfc));
    
    // 在 macOS 端导入 - 需要处理 NFD 规范化
    let result = vault.import("/nfs/server/");
    
    // 验证：文件名应正确处理，不会重复导入
    assert_eq!(result.imported, 1);
    assert_eq!(result.duplicate, 0);
}

/// Windows Samba + Linux Client 测试
#[test]
#[ignore = "requires Samba mount from Windows at /mnt/smb-windows"]
fn test_smb_windows_to_linux_import() {
    // 重点：Windows 保留字符在 Linux 端的行为
    let vault = Vault::init("/mnt/smb-windows/vault");
    
    // Windows 不允许的文件名在 Linux Samba 客户端创建
    // 实际测试应使用 Windows 端创建文件
    
    // 导入测试
    let result = vault.import("/mnt/smb-windows/source/");
    
    // 验证：权限映射正确性
    let file_stat = std::fs::metadata("/mnt/smb-windows/vault/...").unwrap();
    // Windows ACL 映射到 Unix mode 的合理性
}

/// Windows Samba + macOS Client 测试
#[test]
#[ignore = "requires Samba mount from Windows at /Volumes/smb-windows"]
fn test_smb_windows_to_macos_import() {
    // 重点：macOS 扩展属性 (xattr) 通过 SMB 流传输
    let vault = Vault::init("/Volumes/smb-windows/vault");
    
    // 导入带资源分支的文件
    let result = vault.import("/Volumes/smb-windows/source/");
    
    // 验证：资源分支/扩展属性是否保留
}
```

#### 4.5 跨系统测试环境准备

**方案 A：多虚拟机（推荐用于 CI）**

```yaml
# docker-compose.integration.yml
version: '3'
services:
  nfs-server:
    image: erichough/nfs-server
    privileged: true
    volumes:
      - nfs-data:/exports
    environment:
      - NFS_EXPORT_0=/exports *(rw,sync,no_subtree_check)
  
  nfs-client-linux:
    image: ubuntu:22.04
    depends_on:
      - nfs-server
    volumes:
      - nfs-mount:/mnt/nfs
    command: |
      apt-get update && apt-get install -y nfs-common
      mount -t nfs nfs-server:/exports /mnt/nfs
      # 运行测试
  
  # macOS 客户端需要物理机或特殊 CI runner
```

**方案 B：单机多挂载（适用于本地开发）**

```bash
#!/bin/bash
# tests/setup_cross_system_fs.sh

# 模拟 Linux→Linux NFS（使用 loopback）
setup_nfs_linux_linux() {
    # 服务端
    sudo apt-get install -y nfs-kernel-server
    echo "/tmp/nfs-export *(rw,sync,no_subtree_check)" | sudo tee /etc/exports
    sudo exportfs -a
    
    # 客户端（本地挂载）
    sudo mkdir -p /mnt/nfs-local
    sudo mount -t nfs localhost:/tmp/nfs-export /mnt/nfs-local
}

# Windows Samba 测试需要 Windows 主机或虚拟机
# 建议使用 Windows 10/11 虚拟机作为 Samba 服务器
setup_smb_windows_linux() {
    # 在 Windows 虚拟机中设置共享文件夹
    # 在 Linux 主机中挂载
    sudo mount -t cifs //windows-vm/svault-share /mnt/smb-windows \
        -o username=testuser,password=testpass,vers=3.0
}
```

**方案 C：云服务（适用于定期回归测试）**

| 云服务商 | NFS | SMB | 说明 |
|----------|-----|-----|------|
| AWS | EFS | FSx | 全托管，按需付费 |
| Azure | Files (NFS) | Files (SMB) | 支持多种协议 |
| GCP | Filestore | - | NFS 原生支持 |

#### 4.6 跨系统测试实施策略

| 优先级 | 场景 | 实施难度 | 建议方案 | 里程碑 |
|--------|------|----------|----------|--------|
| P0 | NFS Linux→Linux | 🟢 低 | Docker Compose | 阶段 2 |
| P1 | SMB Windows→Linux | 🟡 中 | Windows VM + Linux | 阶段 4 |
| P2 | NFS Linux→macOS | 🔴 高 | 物理 Mac 或 AWS macOS | 阶段 5 |
| P2 | SMB Windows→macOS | 🔴 高 | 物理 Mac 或 CI runner | 阶段 5 |
| P3 | SMB Linux→Windows | 🟡 中 | Samba + Windows VM | 阶段 4 |

#### 测试场景
#[test]
#[ignore = "requires NFS mount at /mnt/nfs-test"]
fn test_nfs_import_with_locks() {
    // NFS 锁行为可能与本地不同
    let vault = Vault::init("/mnt/nfs-test/vault");
    
    // 并发导入测试
    let handles: Vec<_> = (0..4)
        .map(|i| thread::spawn(move || vault.import(...)))
        .collect();
    
    // 所有导入应成功，无锁冲突
    for h in handles {
        assert!(h.join().is_ok());
    }
}
```

### 实施路线图

#### 阶段 1: 基础设施 (2 周)

- [ ] 创建 `tests/integration/` 目录结构
- [ ] 实现 `VaultEnv` 测试辅助结构
- [ ] 添加 CI 条件执行标签（`#[cfg(integration_test)]`）
- [ ] 编写文件系统环境准备脚本

#### 阶段 2: 单系统文件系统集成 (2 周)

- [ ] ext4 回退测试
- [ ] btrfs reflink 测试
- [ ] xfs reflink 测试
- [ ] **NFS Linux→Linux (同构)** 基础测试

#### 阶段 3: 多 Vault (2 周)

- [ ] vault 导出/导入 manifest 测试
- [ ] 增量同步逻辑测试
- [ ] 冲突检测测试

#### 阶段 4: 跨系统网络文件系统 (4 周) ⭐ 重点

- [ ] **SMB Windows→Linux** 权限映射和编码测试
- [ ] **SMB Linux→Windows** 长路径和 ACL 测试
- [ ] NFS 锁行为对比测试（Linux vs 差异）
- [ ] 文件名编码规范化测试（NFC/NFD）

#### 阶段 5: 高级跨系统场景 (4 周)

- [ ] **NFS Linux→macOS** 扩展属性和时间戳测试
- [ ] **SMB Windows→macOS** 资源分支测试
- [ ] 跨系统 vault 迁移测试（如 Linux vault → Windows 访问）
- [ ] 延迟模拟测试（模拟 WAN 延迟下的导入行为）

#### 阶段 6: MTP 设备 (2 周)

- [ ] MTP 设备发现测试
- [ ] MTP 真实设备导入测试（Android、iPhone）

### 跨系统测试特殊注意事项

| 问题 | 影响 | 缓解措施 |
|------|------|----------|
| **macOS 需要物理机/特殊 CI** | NFS/macOS 测试难以自动化 | 使用 AWS EC2 macOS 实例（按需）或本地定期手动测试 |
| **Windows 许可** | Windows Server VM 需要许可 | 使用 Windows 10/11 评估版（90天）或 Azure DevOps Windows runner |
| **权限差异** | root 权限可能需要在 CI 中特殊配置 | 使用容器特权模式或预配置 runner |
| **网络不稳定** | 跨系统测试可能因网络波动失败 | 增加重试机制，区分网络错误和产品错误 |
| **文件系统差异** | 某些特性在跨系统场景不支持 | 明确文档化已知限制，测试降级行为 |

### 运行集成测试

```bash
# 运行所有集成测试（需要完整环境）
cargo test --features integration-tests

# 仅运行文件系统集成测试
cargo test --features integration-tests fs::

# 跳过需要物理设备的测试
cargo test --features integration-tests -- --skip mtp

# 使用环境变量控制测试范围
INTEGRATION_TEST_FS=ext4,xfs cargo test fs::
```

---

## 测试覆盖率目标

| 模块 | 目标覆盖率 | 当前状态 |
|------|-----------|----------|
| hash | 90% | 🟢 已补充 (22 tests) |
| config | 90% | 🟢 已补充 (24 tests) |
| db | 85% | 🟡 部分 (11 tests) |
| vfs | 80% | 🟡 部分 (9 tests) |
| import | 85% | 🟢 已补充 (14 tests) |
| **集成测试** | N/A | 🟡 已规划（单系统） |
| **跨系统集成** | N/A | 🔴 已规划（待实施） |

---

## 待办测试清单

### 高优先级（已完成 ✅）

- [x] `hash::crc32c_region` - 测试头部/尾部 CRC32C 计算
- [x] `hash::xxh3_128_file` - 测试完整文件 XXH3 哈希
- [x] `hash::sha256_file` - 测试完整文件 SHA-256 哈希
- [x] `db::append_event` - 测试事件追加和哈希链
- [x] `db::verify_chain` - 测试哈希链验证
- [x] `import::read_exif_date_device` - 测试 EXIF 提取（多种格式）

### 中优先级

- [x] `config::load` - 测试配置加载和默认值 ✅
- [x] `vfs::transfer_file` - 测试传输引擎 fallback 链 ✅
- [ ] `vfs::probe_capabilities` - 测试文件系统能力探测
- [ ] `db::lookup_by_crc32c` - 测试 CRC32C 查询
- [ ] `db::lookup_by_hash` - 测试哈希查询

### 低优先级

- [ ] 并发导入测试
- [ ] 大文件（>4GB）处理测试
- [ ] 各种文件系统（btrfs/xfs/ext4）行为测试
- [ ] 网络文件系统（NFS/SMB）行为测试

### 性能测试计划（来自质量报告）

> 性能测试用于建立基准线和检测回归

| 测试项 | 目标 | 方法 | 状态 |
|--------|------|------|------|
| 大文件导入 | 1GB+ 视频文件 | 测量导入时间和内存使用 | 🔲 待实施 |
| 并发导入 | 多进程并行 | 测试 4/8/16 进程并发 | 🔲 待实施 |
| 哈希计算基准 | CRC32C vs XXH3 vs SHA-256 | criterion.rs 基准测试 | 🔲 待实施 |
| 数据库查询性能 | 10万+ 文件规模 | 测量查询响应时间 | 🔲 待实施 |
| 内存使用测试 | 批量导入 | valgrind/massif 分析 | 🔲 待实施 |

### 模糊测试计划（Fuzzing）

> 使用 cargo-fuzz 测试边界条件和异常输入

| 目标 | 测试内容 | 工具 | 状态 |
|------|----------|------|------|
| EXIF 解析 | 损坏的 EXIF 数据 | cargo-fuzz + libfuzzer | 🔲 待实施 |
| 文件格式 | 畸形 JPEG/PNG/MP4 | cargo-fuzz | 🔲 待实施 |
| 路径处理 | 非法路径字符、路径遍历 | cargo-fuzz | 🔲 待实施 |
| TOML 配置 | 随机配置突变 | cargo-fuzz | 🔲 待实施 |

**实施步骤：**
1. 添加 `cargo-fuzz` 依赖和模糊测试目标
2. 定义种子语料库（有效文件样本）
3. 设置 CI 每日运行（30 分钟）
4. 集成 crash 报告到 GitHub Issues

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

### 媒体绑定测试 (Media Binding Tests)

> 测试 Live Photo 和 RAW+JPEG 的绑定检测与导入行为
> 
> **测试文件**: `test_binding.py`

| ID | 测试场景 | 描述 | 状态 | 测试函数 |
|----|---------|------|------|----------|
| F1 | Live Photo 检测 | `.heic` + `.mov` 同基础名检测为 Live Photo | ✅ DONE | `test_live_photo_detection_with_ffmpeg` |
| F2 | Live Photo 导入 | 导入时绑定文件使用相同时间/设备路径 | ✅ DONE | `test_live_photo_same_device_path` |
| F3 | RAW+JPEG 检测 | `.dng`/`.arw` + `.jpg` 同基础名检测为 RAW+JPEG | ✅ DONE | `test_raw_jpeg_detection` |
| F4 | RAW+JPEG 导入 | RAW 和 JPEG 导入到同一路径层次 | ✅ DONE | `test_raw_jpeg_same_organization` |
| F5 | Burst 序列检测 | `IMG_0001.jpg` ~ `IMG_0005.jpg` 检测为连拍 | ✅ DONE | `test_burst_detection` |
| F6 | 绑定分离场景 | 部分绑定文件缺失时的导入行为 | ✅ DONE | `test_partial_binding_import` |

**固件生成函数：**
- `create_live_photo_pair()` - 创建 JPG + MOV 对，使用 ffmpeg 设置 creation_time
- `create_raw_jpeg_pair()` - 创建 DNG + JPG 对，使用 PIL 设置 EXIF
- `create_burst_sequence()` - 创建连拍序列 IMG_0001 ~ IMG_000N

### 空间不足测试 (Disk Full Tests)

> 测试磁盘空间不足时的导入行为和错误处理
> 
> CLI 退出码定义：`4` = 目标空间不足
> 
> **测试文件**: `test_import_disk_full.py`

| ID | 测试场景 | 描述 | 状态 | 测试方法 |
|----|---------|------|------|----------|
| D1 | 小容量 RAMDisk 导入 | 创建 2MB RAMDisk，导入大文件，验证优雅失败 | ✅ DONE | `test_import_fails_with_exit_code_4_on_disk_full` |
| D2 | 部分导入后空间不足 | 导入成功几个文件后空间耗尽，验证事务一致性 | ✅ DONE | `test_no_partial_files_after_disk_full` |
| D3 | 大文件预留检查 | 导入前检查文件大小是否超过可用空间 | 🔲 TODO | 待实现 |
| D4 | 空间不足后恢复 | 清理空间后再次导入，验证可以正常继续 | ✅ DONE | `test_can_import_after_cleanup` |

**实现说明：**
- 使用 `mount -t tmpfs -o size=2m` 创建小型 RAMDisk
- 需要 root 权限或 `CAP_SYS_ADMIN` 能力
- 测试在无法挂载时自动跳过

### 视频元数据提取测试 (Video Metadata Tests)

> 测试视频文件的元数据提取功能（`media/video.rs`）
> 
> **单元测试**: `src/media/video.rs`
> **E2E 测试**: `test_import_video_metadata.py`

| ID | 测试场景 | 描述 | 状态 | 测试函数 |
|----|---------|------|------|----------|
| V1-V2 | MP4/MOV creation_time | 参数化测试 MP4 (32/64-bit) 和 MOV 时间戳解析 | ✅ DONE | `test_video_creation_time_extraction[mp4\|mov]` |
| V3 | 时间戳优先级 | creation_time 优先于 mtime | ✅ DONE | `test_video_creation_time_over_mtime` |
| V4 | 视频设备信息 | 从 udta/meta box 提取设备名 | 🚫 SKIP | 需要高级元数据注入 |
| V5 | MTS 时间戳 | AVCHD 格式时间戳提取 | 🔲 TODO | 待实现 |
| V6 | 视频导入路径 | 视频按 creation_time 组织到 `$year/$mon/$day` | ✅ DONE | `test_video_organized_by_year_month_day` |

**辅助函数：**
- `_create_video_with_timestamp()` - 统一函数，使用 ffmpeg 创建带 creation_time 的视频（支持 mp4/mov 格式参数）
- `verify_video_timestamp()` - 使用 ffprobe 验证时间戳
- `create_mov_with_device_info()` - 创建设备信息元数据视频
- `verify_video_device_info()` - 验证设备信息元数据

**附加测试：**
- `test_multiple_videos_different_dates` - 多视频按日期分别组织
- `test_video_device_model_extraction` - 设备模型提取（Apple）
- `test_video_device_model_samsung` - 设备模型提取（Samsung）

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
| 2026-04-04 | **质量报告处理**：修复 `test_property.py` 硬编码路径；添加 `exiftool_available` fixture；补充 hash 模块 22 个单元测试；补充 import/exif 模块 14 个单元测试；补充 db 模块 11 个单元测试；更新 UNIT_TESTS.md | Kimi |
| 2026-04-04 | **集成测试计划**：添加集成测试详细规划章节，包括文件系统测试（ext4/btrfs/xfs）、多 Vault 同步、MTP 设备、网络文件系统（NFS/SMB）的测试设计和实施路线图 | Kimi |
| 2026-04-04 | **跨系统集成测试扩展**：详细规划跨系统挂载场景（NFS Linux→macOS、SMB Windows→Linux/macOS），包括测试矩阵、风险分析、环境准备方案（Docker/VM/云）和实施策略 | Kimi |
| 2026-04-04 | **config 模块单元测试**：添加 24 个单元测试，覆盖配置加载、TOML 解析/序列化、错误处理（未知哈希算法、无效策略、缺少必填项等）| Kimi |
| 2026-04-04 | **vfs/transfer 模块单元测试**：添加 9 个单元测试，覆盖 fallback 链逻辑（reflink→hardlink→stream_copy）、自动创建父目录、内容完整性、空文件/大文件传输 | Kimi |
| 2026-04-04 | **EXIF 回退场景 E2E 测试**：添加 `test_import.py::TestExifFallback`，覆盖无 EXIF、部分 EXIF、损坏 EXIF 等场景 | Kimi |
| 2026-04-04 | **配置和传输策略 E2E 测试**：添加 `test_config_transfer.py`，覆盖配置加载、传输策略 fallback 链（13 个测试）| Kimi |
| 2026-04-04 | **多媒体格式 E2E 测试**：添加 `test_media_formats.py`，覆盖 PNG/TIFF/HEIC/DNG/MP4/MOV 格式（19 个测试），支持 ffmpeg 检测 | Kimi |
| 2026-04-04 | **性能测试和模糊测试计划**：将质量报告中的未完成项添加到测试计划，包括性能基准、cargo-fuzz 模糊测试路线图 | Kimi |
| 2026-04-04 | **删除 review-report.md**：质量报告中的待办事项已同步到 UNIT_TESTS.md，原文件删除 | Kimi |
| 2026-04-04 | **视频元数据提取**：实现 MP4/MOV `creation_time` 解析（`media/video.rs`），支持 ISO BMFF 格式；新增 3 个单元测试；E2E 待补充 | Kimi |
| 2026-04-04 | **Live Photo/RAW+JPEG 测试计划**：添加待办测试项到功能规划章节（F1-F6）| Kimi |
| 2026-04-04 | **空间不足测试计划**：添加磁盘满（ENOSPC）场景的测试设计（D1-D4）| Kimi |
| 2026-04-04 | **实现 E2E 测试套件**（15 个测试）：
- `test_import_disk_full.py`: 空间不足测试（D1,D2,D4）
- `test_binding.py`: Live Photo/RAW+JPEG/ Burst 测试（F1-F6）
- `test_import_video_metadata.py`: 视频元数据提取测试（V1-V3,V6）| Kimi |
| 2026-04-05 | **E2E 测试清理与参数化重构**：
- `test_import_video_metadata.py`: 合并 MP4/MOV 测试为参数化 `test_video_creation_time_extraction`，删除冗余辅助函数，节省 ~60 行
- `test_import_cross_fs.py`: 合并 ext4/btrfs 测试为参数化 `test_cross_fs_import`，节省 ~50 行
- `test_add.py`: 删除重复的 `TestAddRawId` 类（已在 `test_raw_id.py` 覆盖）
- `test_import.py`: 迁移 `TestDuplicateDetection` 到 `test_import_dedup.py`
- **总计**: 减少 ~110 行代码，测试数量保持 190 passed, 8 skipped | Kimi |

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
cd tests/e2e && bash run.sh --verbose

# 只跑特定测试文件
cd tests/e2e && bash run.sh --verbose test_import_force.py

# 使用 release 构建跑 E2E
cd tests/e2e && bash run.sh --release --verbose
```

### Windows

```powershell
# 使用 uv 创建虚拟环境并安装依赖
cd tests/e2e
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
cd tests/e2e && bash run.sh

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
