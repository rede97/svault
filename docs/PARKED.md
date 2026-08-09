# 已移除 / 暂缓功能登记（PARKED）

> **用途**：防止未来会话把已否决的设计当新点子重新发明。**新增功能前先查本文件。**
> 所有代码与设计稿均保留在 git 历史中；每节给出移除/暂缓原因与恢复路径。

---

## A. 已移除——不恢复

### A1. `svault update --delete`（2026-04）

**功能**：物理删除 vault 中 missing 的文件。

**移除原因**：违反核心原则"永不删除用户文件"（failure-handling.md G1）。
且实际为死代码——core 从未读取 `UpdateOptions.delete` 字段。

**恢复路径**：不恢复。missing 标记 + `db dump` 查询已足够，
物理删除应由用户手动执行。

### A2. 事件溯源（`events` 表 + 哈希链 + `db verify-chain`）（2026-08-09）

**功能**：所有 DB 变更先写 append-only 事件（含 prev_hash/self_hash 链），
物化视图由事件重放得到；`db verify-chain` 校验链完整性。

**移除原因**（维护者决策）：**伪需求**。
- 工具定位是帮助用户的比对/归档工具，不为"用户擅自修改数据库"无限兜底
- 哈希链无外部锚点（无签名/异地副本），蓄意篡改者可重建全链——
  防篡改场景不成立，只能防"随手编辑"，而那不是本工具的职责
- 实际只有 3 种事件（batch.imported / file.sha256_resolved / vault.cloned），
  写协议已在漏（update 直接 UPDATE），且从不被重放——write-mostly 死重

**替代**：逐文件历史在 `.svault/sessions/<kind>/<ts-id>/` 的
plan.json / manifest.json（原子写入）；DB 的 `files` 表是运行时状态索引，
整批单事务写入。`db dump` 仍可导出任意表。

**恢复路径**：git 历史（提交 `refactor: remove the events event-sourcing
table`）。若恢复，先回答"防谁"——无外部锚点的链不防蓄意篡改。

### A3. 12 个 typed reporter trait（旧 reporting 系统）（2026-04）

**移除原因**：接口面过大（12 trait + 10 个 builder 关联类型），
每加一个命令需改 4 处。已由单一 `Event` enum + `EventSink` 替代
（见 ARCHITECTURE.md §2.1）。

**恢复路径**：不恢复。

---

## B. 已移除/降级——可有条件恢复

### B1. `svault history`（命令 + core::history 模块）（2026-04）

**功能**：查询导入会话（sessions）和单文件记录，分页/过滤，表格 + JSON。

**移除原因**：次要功能——早期 AI 会话按"归档工具应该有历史浏览"的假设
造出，并非用户核心需求；给 core 增加 4 DTO + 2 reporter trait，
是接口面膨胀的典型。逐文件历史在会话清单中，等价查询可用 `svault db dump`。

**恢复路径**：git 历史中 `svault-core/src/history/`、CLI `commands/history.rs`、
E2E `tests/e2e/test_history.py`。**若恢复，应改为 Pull 模型**（返回数据，
UI 格式化），不要恢复 reporter trait 版本。

### B2. `svault scan`（降级为 debug-only，2026-04）

**功能**：扫描目录并输出 pipe 协议（`SCAN:` / `new:` / `dup:` / `fail:`）。

**现状**：管线调试工具，debug 构建可用（release 不含）；
`--files-from` 管道组合仍在 import 中保留。

---

## C. 暂缓设计（从未实现，立项前须先更新设计文档）

### C1. `svault recover` 损坏恢复 + sync health pre-flight + sync_journal 断点续传

**设计**：vault 文件损坏（bit rot/坏道）但 DB 完好时，从健康备份 vault
拉取完好副本替换。损坏原件移到 `.svault/corrupted/`（不删除），逐文件确认
`[y/N/a/q]`。同时暂缓：sync 源端一致性 pre-flight 检查、sync_journal
断点续传。

**暂缓原因**：仅为设计稿（原 `docs/sync-design.md`），从未实现；
且基于已移除的 reporter trait 体系，直接复活会引入第二套报告抽象。
现行损坏检测能力边界见 failure-handling.md §4。

**2026-08-09 部分复活**：tmp→rename 原子提交思路已就 **import** 落地为
会话日志模型（`session` 模块：plan.json → staging/ → fsync → hash →
整批入库 → rename + 启动对账）。sync_journal 断点续传维持暂缓——
plan.json 提供 source↔dest 映射但只用于事后剖析，不参与恢复决策
（判据：failure-handling.md §5 注）。

**若立项**：交互必须走 `Event`/`Interactor`，不得恢复 reporter trait。
原设计稿：`git show f34d53b:docs/sync-design.md`。

---

## D. 已实现（原暂缓，登记备查）

### D1. `svault sync` / `svault clone`（2026-04 设计，已实现）

Beyond Compare 风格（ARCHITECTURE.md §6）。Clone 单向导出（`--filter-date`）；
Sync 为 DB-vs-DB hash 加速比对 + 方向性复制 + 各记各的账（不合并历史）。
sync 不落 `--fix-moves`（moved 仅报告）。

---

## E. 已知小遗留（确认非 bug）

1. `verify --background-hash` 的 Summary 事件未统一（用 messages 输出）
2. clone 重复导出会重新复制（path+size 跳过优化未采纳，可作后续小改进）
3. diff 引擎边缘：dest 同 identity 多路径时只保留一个索引项——v1 接受
