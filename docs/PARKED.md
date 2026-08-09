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
- **数据未丢失**：逐文件历史在会话清单中，等价查询可用 `svault db dump`

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

## 6. `svault recover` 损坏恢复（未实现设计，暂缓）

**设计**：vault 文件损坏（bit rot/坏道）但 DB 完好时，从健康备份 vault
拉取完好副本替换。损坏原件移到 `.svault/corrupted/`（不删除），
逐文件确认 `[y/N/a/q]`。同时暂缓的还有 sync health pre-flight
（源端一致性检查阻止 sync）与 sync_journal 断点续传。

**暂缓原因**：仅为设计稿（原 `docs/sync-design.md` §第三种模式），
从未实现；且该设计稿基于已移除的 reporter trait 体系
（HealthReporter/RecoverReporter），直接复活会引入第二套报告抽象。

**现状记录**：现行损坏检测能力边界见
[failure-handling.md](./failure-handling.md) §4。
若立项实现，交互必须走 `Event`/`Interactor`，不得恢复 reporter trait。
原设计稿可从 git 历史恢复：`git show f34d53b:docs/sync-design.md`。

> **2026-08-09 部分复活**：原 sync-design 的 tmp→rename 原子提交思路已就
> **import** 立项实现为会话日志模型（`.svault/sessions/import/<ts-id>/`：
> plan.json → staging/ → fsync → hash → 整批入库 → rename + 启动对账，
> 见 `session` 模块与 failure-handling.md G7）。sync/clone 仍直达最终路径；
> sync_journal 断点续传与 recover 维持暂缓。

## 7. 已知小遗留（2026-08-04 Linux 验证确认，非 bug）

1. `verify --background-hash` 的 Summary 事件未统一（用 messages 输出）
2. clone 重复导出会重新复制（path+size 跳过优化未采纳，可作后续小改进）
3. diff 引擎边缘：dest 同 identity 多路径时只保留一个索引项——v1 接受

## 8. 事件溯源（`events` 表 + 哈希链 + `db verify-chain`）

**功能**：所有 DB 变更先写 append-only 事件（含 prev_hash/self_hash 链），
物化视图由事件重放得到；`db verify-chain` 校验链完整性。

**移除原因**（维护者决策，2026-08-09）：**伪需求**。
- 工具定位是帮助用户的比对/归档工具，不为"用户擅自修改数据库"无限兜底
- 哈希链无外部锚点（无签名/异地副本），蓄意篡改者可重建全链——
  防篡改场景不成立，只能防"随手编辑"，而那不是本工具的职责
- 实际只有 3 种事件（batch.imported / file.sha256_resolved / vault.cloned），
  写协议已在漏（update 直接 UPDATE），且从不被重放——write-mostly 死重

**替代**：逐文件历史在 `.svault/sessions/<kind>/<ts-id>/` 的
plan.json / manifest.json（原子写入）；DB 的 `files` 表是运行时状态索引，
整批单事务写入。`db dump` 仍可导出任意表。

**恢复路径**：git 历史（移除提交见 2026-08-09 `refactor: 移除 events
事件溯源表`）。若恢复，先回答"防谁"——无外部锚点的链不防蓄意篡改。
