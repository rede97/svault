# 2026-04 架构重构与 Clone/Sync 实现 — 决策记录

> 本文档记录本轮工作的完整脉络：**用户提出的问题与需求 → 对应的决策 → 实际修改 → 最终结果**。
> 目的是让后续维护者（人类或 AI）理解每个决策的上下文，避免重复讨论已否决的方案。
>
> 日期：2026-04-16 · 执行：Pi（编码代理）· 指导：项目所有者

---

## 背景

项目此前由多个 AI 模型（Claude / GPT / DeepSeek 早期版本）接力维护，
架构逐渐失控。所有者要求重新评估代码质量，并以"良好可靠的架构"为第一优先级重整。

**重构前状态诊断**（勘察所得，非推测）：

| 症状 | 证据 |
|------|------|
| 同一概念两套抽象并存 | `import/mod.rs`（953 行）自带管线编排，同时 `pipeline/` 又抽象了一套五阶段 |
| 报告接口面失控 | 12 个 typed reporter trait + builder 上 10 个关联类型；每加一个命令要改 4 处 |
| 分层泄漏 | core 依赖 `rich_rust`（终端渲染库）和 optional `clap` |
| 文档与代码漂移 | AGENTS.md 列出已删除的模块、不存在的命令；`refactor_task.md`（任务卡）遗留在仓库根目录 |
| 死代码 | `sync`/`clone` 是 `todo!()` 却出现在 CLI help；`staging.rs` 的 `write_pending` 无人调用；`db/schema.rs` 是未挂载的孤儿文件 |
| 自相矛盾 | AGENTS.md 声称"永不删除文件"，`update --delete` 却提供物理删除（且为未接线死代码） |
| 巨石文件 | terminal.rs 1608 行、import/mod.rs 953 行、config.rs 794 行 |

**根因判断**：不是某个模型能力问题，而是缺少被执行的架构约束——
每次 AI 会话拿到局部任务卡，做局部最优补丁，无人对全局一致性负责。

---

## 决策记录

### Q1：代码质量如何？应从顶层如何指定架构？

**决策**：不推倒重写，采用三 crate 分层 + 六条硬性依赖规则。

```
svault-core  纯库（领域 + 用例编排）   —— 禁止终端依赖、禁止直接读写 stdin/stdout
svault-ui    展现层（indicatif/console/rich_rust）
svault-cli   薄入口（clap 解析 + 组装 + 调用）
```

关键规则（写入 `docs/ARCHITECTURE.md`，CI 可强制）：
- R3：core 的进度通信只有一条通道——单一 `event::Event` 枚举 + `EventSink` trait
- R6：一个概念只允许一个抽象（管线只有 `pipeline/` 一套）

**接口设计策略**：用户授权"先敲定接口还是迭代优化可以自己决定"——
选择**先敲定**（Event 枚举 + Push/Pull 双模型），因为它同时约束 core 和 ui 两侧，
防止重构中接口漂移。Push（事件）用于长耗时操作，Pull（返回 Serialize 数据）用于查询。

**修改**：
- 12 个 reporter trait → 单一 `Event`（20 个变体，派生 Serialize）+ `EventSink`
- `import/` → `ops/`（import/add/update/recheck 统一为用例编排器）
- core 移除 rich_rust / clap 依赖；`TransferStrategyArg` 改用 `FromStr`
- 新建 `svault-ui` crate：TerminalSink（状态机）/ JsonSink / PipeSink / SuspendingInteractor

**结果**：core 10,161 行 / ui 1,505 行 / cli 1,076 行；净删 2,791 行；
`--output json` 成为零成本的自描述事件流（serde 直接序列化，删除 672 行手写映射）。

---

### Q2：以骨干功能为核心；次要功能删除或降级；CLI 展现独立成模块

**功能分级决策**（原则：只保留"内容寻址归档 + 可验证完整性"的直接支撑）：

| 功能 | 处置 | 理由 |
|------|------|------|
| init / import / add / update / verify / recheck / status / db | **骨干保留** | 核心价值的直接组成 |
| `history` | **移除** | AI 想当然造出的次要功能；数据仍可用 `db dump` 查询 |
| `sync` / `clone` | **移除 stub** | `todo!()` 却出现在 help 中误导用户（后按 Q4 正式实现） |
| `update --delete` | **移除** | 违反"永不删除文件"核心原则；且是未接线的死代码 |
| `scan` | **降级 debug-only** | 管线调试工具，非用户功能；E2E 用 debug 二进制不受影响 |
| 死代码 `staging.rs` / `db/schema.rs` | **删除** | 无任何调用方 |

所有移除均记录在 `docs/PARKED.md`（原因 + 恢复路径），防止后续 AI 会话"顺手复活"。

**结果**：release 构建从 12 个命令（含 2 个 stub）收敛为 8 个真实骨干命令。

---

### Q3：为什么删了进度条库？架构考虑过 Clone/Sync 吗？用 turso 替代 SQLite？

**回答与决策**：

1. **进度条库没有删除，是物理搬迁**。`indicatif` 从 core/cli 移到 `svault-ui`，
   tty 下的动画进度条完全保留；唯一行为变化是管道/CI 环境从"完全静默"（旧 bug）
   修复为输出完整文本。用户看到的"删除"是依赖从 Cargo.toml 挪到了正确的 crate。

2. **Clone/Sync 架构考虑了一半**。地基已备（管线复用、Transfer 策略、事件接口），
   但 `Db` 仍是 rusqlite 具体类型，跨 vault 比对与事件合并语义未设计。
   当时将 sync/clone 记入 PARKED，要求"先设计再实现"。

3. **turso：建议端口先行，不建议直接替换**。核心冲突：Svault 的存在意义是数据可信
   （事件溯源 + 哈希链），而 turso 仍是快速迭代的未成熟引擎。
   提出 Store trait + 双适配器（rusqlite 默认，turso feature-gated 实验）的渐进路径。

---

### Q4：SQLite 替换放到最低优先级，优先实现 Clone/Sync？

**决策**：完全同意，并修正了自己上一轮的过度设计。

原方案称"Store trait 是 Sync 前置条件"——**不是**。Sync 只需要
`Db::open_readonly(path)` 一个具体函数；trait 只有到真换引擎那天才有意义。
当时 `Db` 查询面尚未稳定（Sync 开发中确实新增了 `open_readonly`），
提前抽象会在开发中反复返工。正确顺序：**Sync 让查询面长齐 → turso 立项时再抽 trait**。

**Clone/Sync 设计**（Beyond Compare 风格，明确排除 git 风格）：

- **Clone**：单向导出。`--filter-date` 是唯一过滤维度——media_groups 表存在但
  相机数据未填充，拒绝提供半吊子的 `--filter-camera`（正是要杜绝的"想当然功能"）
- **Sync 比对引擎是纯函数**（`sync/diff.rs`，零 IO）：DB 记录 vs DB 记录，
  SHA-256 优先 identity，mtime/size 信任 DB（归档文件 immutable 假设）
- **各记各的账**：dest 追加普通入库记录（SessionType::Sync + manifest），
  **不合并事件日志**——这是避开 git 合并复杂性的关键一刀
- **永不删除**：only_dest 仅报告；conflict 保留本地；moved 仅报告（v1）

**修改**：新增 `sync/diff.rs`（11 单测）、`ops/clone.rs`（5 单测）、
`ops/sync.rs`（3 单测）、`Db::open_readonly`、`SessionType::Sync/Clone`、
事件变体 `Phase::Compare` / `SyncPlan` / `Summary::{Clone,Sync}`、
CLI 两个命令、E2E 两个文件（17 个测试）。

**结果**：全部场景验证通过——幂等重同步、冲突保留本地、moved 检测、
源 vault 只读（E2E 断言 sync 前后源 events 不变）、审计事件、JSON 事件流。

---

### Q5：相关文档补齐了吗？

**文档审计发现的问题**（比代码漂移更严重的是文档在描述"想象中的系统"）：

| 文档 | 问题 | 处置 |
|------|------|------|
| `ARCHITECTURE.md` | §4.3 说 sync/clone 已移除，§6 却说已实现（自相矛盾） | 修正分级表 + 补 §3/§2.1 |
| `docs/cli.md` | 记录不存在的选项（`--progress`/`--config`/`--vault`/`-H`）、虚构的退出码 2-6、错误的 manifest 路径 | **全文档重写**，与实现对齐 |
| `docs/database-schema.md` | 描述从未实现的 `import_sessions` 表、`files.import_session_id` 字段 | **全文档重写**，以 SCHEMA 常量为准 |
| `docs/import-pipeline.md` | `.pending` 续传文件是"核心设计"——实际从未实现（staging.rs 死代码） | 重写恢复语义章节：真实机制是 CRC32C 缓存幂等重跑 |
| `README.md` | 架构图两层（实际三层）、命令表含 stub、Roadmap 过时 | 更新 |
| `tests/e2e/README.md` | 测试文件表缺 12 个文件 | 补齐 |

**审计中发现的两个真实代码 bug**：

1. `crc32c_epoch()` 查询从未在 SCHEMA 中创建的 `metadata` 表——死代码，已删除
2. import 早退路径（全部重复/用户拒绝/dry-run）不发 Summary 事件——
   JSON 消费者没有流结束标记。已修复：**所有操作保证以 Summary 事件收尾**

**E2E 适配**：conftest 的 `parse_json_summary` 更新为新 schema
（`event=summary, kind=import`），兼容旧调用签名；
`test_path_compatibility.py` 从已删除的 `history` 命令迁移到 `db dump` 查询。

---

## 最终结果

### 验证矩阵

| 验证项 | 结果 |
|--------|------|
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ 零警告 |
| Rust 单元测试 | ✅ 153 通过 / 0 失败（重构后新增 26 个） |
| Python E2E（Windows 全量） | ✅ 206 通过 / 39 跳过；1 failed + 4 errors 均为 Linux 专属（ext4/btrfs/strace），**与重构前基线逐项比对完全相同——零回归** |
| E2E 新增（clone 8 + sync 9） | ✅ 全部通过 |
| 真机冒烟 | ✅ import 全流程 / 去重 / update 移动修正 / sync 冲突与幂等 / clone 过滤 / JSON 流合法性 / `--quiet` |
| Release 命令集 | ✅ 10 个真实命令，无 stub |

### 量化结果

- 三轮累计 81 个文件变更，+5,853 / −6,730 行
- `svault-core` 10,161 行（纯库）· `svault-ui` 1,505 行 · `svault-cli` 1,076 行
- 报告接口：12 trait + 10 关联类型 → 1 个 Event 枚举 + 1 个 EventSink trait
- 新增命令接口成本：旧模式改 4 处 → 新模式加事件变体即可（sync/clone 验证）

### 防再失控机制（本轮最重要的产出）

1. `docs/ARCHITECTURE.md` — 架构规则单一事实源（R1–R6 可被 CI grep 强制）
2. `docs/PARKED.md` — 每个移除功能的原因与恢复路径
3. `AGENTS.md` — 只写流程规范与不可妥协原则，不写会腐烂的模块清单
4. 文档与实现不一致时以代码为准并**当场修正文档**（本轮已修正 7 份）

### 明确的遗留事项（按优先级）

1. **Linux CI 全套 E2E**——cross_fs / interruption / fuse 测试需在真 ext4/btrfs 验证
2. `sync --fix-moves`——v1 对 moved 文件仅报告，后续可加路径修正选项
3. `verify --background-hash` 的 Summary 事件类型统一（当前用 messages 输出）
4. turso 适配器——**最低优先级**；立项时先从稳定后的 `Db` 查询面抽 `Store` trait，
   rusqlite 保持默认，turso feature-gated 接入并用同一测试矩阵验证
