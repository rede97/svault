# 已移除 / 暂缓的功能（PARKED）

> 本文档记录从代码库中移除的功能、移除原因、以及恢复路径。
> 所有代码均保留在 git 历史中；本文档指向移除前的最后一个提交。

移除提交点：2026-04 架构重构（三 crate 分层：core / ui / cli）。

---

## 1. `svault history`（命令 + core::history 模块）

**功能**：查询导入会话（sessions）和单文件记录（items），支持分页/过滤，terminal 表格 + JSON 输出。

**移除原因**：
- 次要功能——早期 AI 会话按"归档工具应该有历史浏览"的假设造出，并非用户核心需求
- 给 core 的报告接口增加了 4 个 DTO + 2 个 reporter trait，是接口面膨胀的典型
- **数据未丢失**：事件溯源 DB 仍在，等价查询可用 `svault db dump events|files --format json`

**恢复路径**：git 历史中 `svault-core/src/history/`、CLI `commands/history.rs`、
E2E `tests/e2e/test_history.py`。若恢复，应改为 Pull 模型（返回数据，UI 格式化），
不要恢复 reporter trait 版本。

## 2. ~~`svault sync` / `svault clone`~~（已实现）

2026-04 架构重构后正式实现，设计见 [ARCHITECTURE.md](./ARCHITECTURE.md) §6。

- **Clone**：单向导出，v1 仅 `--filter-date`；相机/分组过滤等 media binding 完成后再加
- **Sync**：DB-vs-DB hash 加速比对 + 方向性复制 + 各记各的账（不合并事件日志）
- Sync v1 暂不移正 moved 文件路径（仅报告），后续可加 `--fix-moves`

## 3. `svault update --delete`

**功能**：物理删除 vault 中 missing 的文件。

**移除原因**：**违反核心原则"永不删除用户文件"**。
且实际为死代码——core 从未读取 `UpdateOptions.delete` 字段。

**恢复路径**：不恢复。missing 标记 + `db dump` 查询已足够，
物理删除应由用户手动执行。

## 4. `svault scan`（降级为 debug-only）

**功能**：扫描目录并输出 pipe 协议（`SCAN:` / `new:` / `dup:` / `fail:`）。

**降级原因**：这是管线调试工具，不是用户功能；
`--files-from` 管道组合仍在 import 中保留。

**现状**：debug 构建中可用（`cargo build` 默认）；release 构建不含。
E2E 测试使用 debug 二进制，因此 `test_scan_import_pipeline.py` 不受影响。

## 5. 12 个 typed reporter trait（旧 reporting 系统）

**移除原因**：接口面过大（12 trait + 10 个 builder 关联类型），
每加一个命令需改 4 处。已由单一 `Event` enum + `EventSink` 替代
（见 [ARCHITECTURE.md](./ARCHITECTURE.md) §2.1）。
