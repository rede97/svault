# Svault 架构文档

> 本文档是架构的**单一事实源**。任何代码变更与本文档冲突时，先更新本文档或修正代码。
> 本文档只写**规则与边界**，不写易腐烂的文件清单。

---

## 1. 分层架构

```
┌────────────────────────────────────────────────────────┐
│ L3  Presentation    crate: svault-ui                   │
│     终端渲染 (indicatif/console) · JSON 事件流 · 交互    │
├────────────────────────────────────────────────────────┤
│ L2  Application     crate: svault-cli (bin)            │
│     clap 参数解析 · 组装 sink/interactor · 调用 core    │
├────────────────────────────────────────────────────────┤
│ L1  Domain          crate: svault-core                 │
│     ops/ 用例编排 · pipeline/ 五阶段管线 · 领域模型      │
├────────────────────────────────────────────────────────┤
│ L0  Infrastructure  svault-core::{db, fs, hash, media} │
│     SQLite 适配器 · 文件传输 (reflink/hardlink/copy)    │
└────────────────────────────────────────────────────────┘
```

### 依赖规则（CI 强制执行）

| 规则 | 说明 |
|------|------|
| R1 | `svault-core` **禁止**依赖 `indicatif` / `console` / `rich_rust` / `clap` |
| R2 | `svault-core` **禁止**直接读写 stdin/stdout/stderr（`println!`/`eprintln!`/`read_line`） |
| R3 | core 与外界的一切进度通信通过 `event::Event` + `EventSink` 完成 |
| R4 | 交互确认通过 `event::Interactor` trait，core 不直接提示 |
| R5 | `svault-ui` 依赖 `svault-core`，`svault-cli` 依赖两者；反向禁止 |
| R6 | 一个概念只允许一个抽象（管线只有 `pipeline/` 一套） |

---

## 2. 核心接口（先敲定，再实现）

### 2.1 进度事件（Push 模型）——用于长时间运行的操作

```rust
// svault-core/src/event.rs

pub enum Phase { Scan, Copy, Hash, Insert, Apply, Recheck, Verify, Compare }

pub enum Event {
    PhaseStarted { phase, total, context },      // 阶段开始（创建进度条）
    PhaseFinished { phase },                     // 阶段结束（清理进度条）
    ScanItem { path, size, mtime_ms, status, error },
    Preflight { source, total, new, duplicate, moved, failed },
    CopyStarted / CopyProgress / CopyFinished,
    HashStarted / HashFinished { path, bytes, error },
    RelocateMatched { old_path, new_path, confidence },  // update 命令
    Progress { phase, done, total },             // insert/apply 计数
    ApplyError { path, message },
    VerifyItem { path, result },
    RecheckStarted { total, session_id, source },
    RecheckItem { src, vault, status },
    SyncPlan { source_vault, identical, to_copy, .. },   // sync 比对结果
    Summary(Summary),  // import/add/verify/recheck/update/clone/sync
    Hint(Hint),        // OnlyMoved / MovedHint / NothingToUpdate / ..
    Summary(Summary),                            // 结构化总结（import/add/verify/…）
    Hint(Hint),                                  // OnlyMoved/MovedHint/DryRunMissing/…
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: &Event);
}

pub trait Interactor: Send + Sync {
    fn confirm(&self, message: &str) -> bool;
}
```

**设计决策：**
- 单一 `Event` enum + 单一 `EventSink` trait，替代旧的 12 个 typed reporter trait
- `Event` 派生 `Serialize`：JSON 输出 = 直接序列化，无需手写映射
- 所有用例函数签名统一：`fn run_xxx(opts, db, sink: &dyn EventSink, interactor: &dyn Interactor)`
- `NoopSink` / `YesInteractor` 提供于 core，供测试与自动化使用

### 2.2 查询（Pull 模型）——用于即时返回的命令

`status`、`db dump` 等**不使用事件**，直接返回 `Serialize` 数据结构，由 UI 层格式化：

```rust
let report: StatusReport = svault_core::status::generate_report(root, db, opts)?;
// human → svault_ui::status::render(&report)
// json  → serde_json::to_string(&report)
```

**判断标准**：操作耗时 > 1 秒或需逐文件反馈 → Push；否则 → Pull。

---

## 3. 一条管线，参数化命令

所有写库操作共享 `pipeline/` 五阶段（scan → crc → lookup → hash → insert），
`ops/` 中的用例只是参数化组合：

| 用例 | 输入源 | 去重 | 传输 | 写库 |
|------|--------|------|------|------|
| `ops::import` | 目录遍历 / 文件列表 | ✓ | ✓ | ✓ + manifest |
| `ops::add` | 目录遍历（vault 内） | ✓ | ✗ | ✓ + manifest |
| `ops::update` | DB missing 记录 + 磁盘扫描 | 按 hash 匹配 | ✗ | 路径修正 |
| `ops::recheck` | manifest | ✗ | ✗ | 只写报告 |
| `ops::verify` | DB 全量 | ✗ | ✗ | 只读 |
| `ops::clone` | DB 全量（可过滤） | ✗ | ✓ | 审计事件 + 目标 manifest |
| `ops::sync` | 对端 vault DB（只读） | diff 引擎 | ✓ | ✓ + sync manifest |

---

## 4. 功能分级（骨干 vs 裁减）

### 4.1 骨干功能（第一优先级，必须可靠）

| 命令 | 说明 |
|------|------|
| `init` | 初始化 vault |
| `import` | 导入（含 `--files-from` / `--strategy` / `--force` / `--full-id` / `--dry-run`） |
| `add` | 注册 vault 内已有文件 |
| `update` | 修正被移动/重命名文件的数据库路径 |
| `verify` | 完整性校验（含 `--file` / `--recent` / `--background-hash` / `--upgrade-links`） |
| `recheck` | 基于 manifest 的双侧校验 |
| `status` | vault 统计 |
| `clone` | 单向导出子集到普通目录（§6.1） |
| `sync` | vault 间同步，hash 加速比对（§6.2） |
| `db` | 维护工具（dump / verify-chain） |

### 4.2 Debug-only（仅 debug 构建）

| 命令 | 说明 |
|------|------|
| `scan` | 管线调试（pipe 协议输出） |
| `debug reporter` | reporter 行为模拟 |

### 4.3 已移除（见 docs/PARKED.md）

| 功能 | 移除原因 |
|------|----------|
| `history` | 次要功能；事件日志仍在 DB，可用 `db dump events` 查询 |
| `update --delete` | **违反"永不删除用户文件"核心原则**，是设计漂移的产物 |

（`sync` / `clone` 曾为 `todo!()` stub 被移除，后按 §6 设计正式实现。）

---

## 5. 核心原则（不可妥协）

1. **永不删除用户文件** — 任何命令不得提供删除磁盘文件的路径
2. **事件溯源** — 所有 DB 变更记入 `events` 表
3. **三层哈希** — CRC32C（预筛）→ XXH3-128（快速身份）→ SHA-256（确定身份）
4. **进程锁** — 写操作必须持有 `.svault/lock`
5. **core 可测试** — 所有用例可用 `NoopSink` 在无终端环境运行

---

## 6. Clone 与 Sync（vault 间复制）

定位：**Beyond Compare 风格，不是 git 风格**。现场比对两侧状态给出差异视图；
不跟踪血统、不合并事件日志、不需要共同祖先。

### 6.1 Clone（单向导出）

`svault clone --target <dir>` —— 把 vault 的文件子集复制到普通目录（非 vault）。

- 只读源 vault；目标目录不建立任何 vault 结构，只额外写一份 `svault-clone-manifest.json`
- 过滤只提供有数据支撑的维度：`--filter-date`（按 mtime）。
  相机/分组过滤等 media binding 填充后才有意义，暂未提供
- 源 DB 记一条 `vault.cloned` 审计事件

### 6.2 Sync（vault 间同步）

`svault sync <source_vault>` —— 把源 vault 中本 vault 缺失的文件复制过来。

**比对引擎是纯函数**（`sync/diff.rs`，无 IO，可暴力单测）：

```
diff_vaults(source_records, dest_records) → DiffPlan
  identical   — identity(hash) 相同且路径相同 → 跳过
  only_source — 源有 dest 无                  → 待复制
  only_dest   — dest 有源无                   → 仅报告（永不删除）
  moved       — identity 相同但路径不同       → 仅报告（v1 不改路径）
  conflict    — 路径相同但 identity 不同      → 跳过复制，报告，保留 dest
```

**Hash 加速**：比对基于两侧 DB 记录（sha256 优先，xxh3_128 兜底），
不做全量文件哈希。归档文件 immutable，信任 DB 记录；
源文件若在磁盘上缺失，传输阶段自然报错并计入 failed。

**各记各的账**：复制完成后在 dest 追加普通入库记录
（`SessionType::Sync` + manifest），源 vault 的 DB 不被修改。
事件日志不跨 vault 合并——这是避开 git 复杂性的关键简化。

**只比对 `status='imported'` 的记录**：missing 记录代表磁盘上不存在，
不参与比对；dest 的 missing 记录在复制同 hash 文件时按既有 recover 逻辑复活。

### 6.3 turso（最低优先级，未立项）

SQLite 引擎替换（turso）**暂缓**。届时先从稳定后的 `Db` 查询面抽 `Store` trait，
rusqlite 保持默认适配器，turso 以 feature flag 实验接入，用同一测试矩阵验证。
