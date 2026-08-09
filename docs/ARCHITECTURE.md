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

pub enum Phase { Scan, Copy, Hash, Insert, Apply, Verify, Compare }

pub enum Event {
    PhaseStarted { phase, total, context },      // 阶段开始（创建进度条）
    PhaseFinished { phase },                     // 阶段结束（清理进度条）
    ScanItem { path, size, mtime_ms, status, error },
    Preflight { source, total, new, duplicate, moved, failed },
    CopyStarted { src, dst, bytes },
    CopyProgress { src, copied, total },
    CopyFinished { src, dst, error },
    HashStarted { path, bytes },
    HashFinished { path, bytes, error },
    RelocateMatched { old_path, new_path, confidence },  // update 命令
    Progress { phase, done, total },             // insert/apply 计数
    ApplyError { path, message },
    VerifyItem { path, result },
    SyncPlan { source_vault, identical, to_copy, .. },   // sync 比对结果
    Summary(Summary),  // import/add/verify/update/clone/sync/album
    Hint(Hint),        // OnlyMoved / MovedHint / NothingToUpdate / DryRunMissing
                       // StagingReconciled / SessionResidue / StagedCommitDeferred
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
| `ops::verify` | DB 全量 | ✗ | ✗ | 只读 |
| `ops::clone` | DB 全量（可过滤） | ✗ | ✓ | 目标 manifest |
| `ops::sync` | 对端 vault DB（只读） | diff 引擎 | ✓ | ✓ + sync plan/manifest |

### 3.1 阶段职责与关键决策

- **Stage A/B（扫描 + CRC 预筛）**：CRC 只读源文件头/尾各 64KB（格式相关，
  见 `media/crc.rs`），是**快速过滤器**——命中 DB 缓存则跳过传输；不参与
  最终身份判定，不作为身份入库。全部命中时早退（`all_cache_hit`）。
- **Lookup（查重）**：串行内联 `ops::check_duplicate`（size+CRC+扩展名），
  在复制前分流重复，避免不必要传输。
- **Stage C（复制，仅 import）**：先原子写 `plan.json`（复制意图，
  **fail-fast**），再复制到会话 staging 子树并 fsync。传输策略链：
  reflink → hardlink → stream copy 无条件兜底（`fs::try_transfer`）。
- **Stage D（强哈希）**：对**暂存副本**算 XXH3-128（必算）/SHA-256
  （`--full-id`/`--force`）；二次去重（DB 跨会话 + DashMap 批内）。
- **Stage E（入库 + 原子提交）**：**整批单事务**——逐条提交比批量慢两个
  数量级（每次 commit 一次 fsync），且全有或全无、中断无中间态；commit
  成功后才把暂存文件逐个 rename 到最终路径；manifest 原子写入会话目录。
- **会话对账（reconcile）**：下次 import 启动时补完成"已入库未 rename"
  的 rename；其余中断残留**只报告不删**（核心原则 1）。

故障语义与中断矩阵的权威描述在 [failure-handling.md](./failure-handling.md)
§3.1/§5/G7。

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
| `status` | vault 统计 |
| `clone` | 单向导出子集到普通目录（§6.1） |
| `sync` | vault 间同步，hash 加速比对（§6.2） |
| `album` | 多级相册 + 成员级评级（Pull 模型 CRUD） |
| `db` | 维护工具（dump） |

### 4.2 Debug-only（仅 debug 构建）

| 命令 | 说明 |
|------|------|
| `scan` | 管线调试（pipe 协议输出） |
| `debug reporter` | reporter 行为模拟 |

### 4.3 已移除（见 docs/PARKED.md）

| 功能 | 移除原因 |
|------|----------|
| `history` | 次要功能；可用 `db dump` 查询 |
| `update --delete` | **违反"永不删除用户文件"核心原则**，是设计漂移的产物 |
| 事件溯源（`events` 表 + `db verify-chain`） | 维护者决策：伪需求（2026-08-09，PARKED §A2） |

（`sync` / `clone` 曾为 `todo!()` stub 被移除，后按 §6 设计正式实现。）

---

## 5. 核心原则（不可妥协）

1. **永不删除用户文件** — 任何命令不得提供删除磁盘文件的路径（svault 只清理本次会话自建的 staging 子树）
2. **会话日志** — import/sync 的意图与结果写入 `.svault/sessions/<kind>/<ts-id>/`（plan/manifest，原子写入）
3. **三层哈希** — CRC32C（预筛）→ XXH3-128（快速身份）→ SHA-256（确定身份）
4. **进程锁** — 写操作必须持有 `.svault/lock`
5. **core 可测试** — 所有用例可用 `NoopSink` 在无终端环境运行

---

## 6. Clone 与 Sync（vault 间复制）

定位：**Beyond Compare 风格，不是 git 风格**。现场比对两侧状态给出差异视图；
不跟踪血统、不合并历史、不需要共同祖先。

### 6.1 Clone（单向导出）

`svault clone --target <dir>` —— 把 vault 的文件子集复制到普通目录（非 vault）。

- 只读源 vault；目标目录不建立任何 vault 结构，只额外写一份 `svault-clone-manifest.json`
- 过滤只提供有数据支撑的维度：`--filter-date`（按 mtime）。
  相机/分组过滤等 media binding 填充后才有意义，暂未提供

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

**各记各的账**：复制前在 dest 落 `sessions/sync/<ts-id>/plan.json`（diff
意图），复制完成后在 dest 追加普通入库记录（`SessionType::Sync` +
manifest），源 vault 的 DB 不被修改。历史不跨 vault 合并——这是避开
git 复杂性的关键简化。

**只比对 `status='imported'` 的记录**：missing 记录代表磁盘上不存在，
不参与比对；dest 的 missing 记录在复制同 hash 文件时按既有 recover 逻辑复活。

### 6.3 turso（最低优先级，未立项）

SQLite 引擎替换（turso）**暂缓**。届时先从稳定后的 `Db` 查询面抽 `Store` trait，
rusqlite 保持默认适配器，turso 以 feature flag 实验接入，用同一测试矩阵验证。

---

## 7. 源码地图（概念 → 代码）

模块粒度对照表，用于按概念定位实现（模块名稳定；行号会腐烂，不写）：

| 概念 | 位置 |
|------|------|
| 管线五阶段（scan/crc/lookup/hash/insert） | `svault-core/src/pipeline/` |
| 用例编排（import/add/update/sync/clone/album） | `svault-core/src/ops/` |
| 相册与评级（多级树、成员引用 files.id） | `svault-core/src/ops/album.rs` + `svault-core/src/db/albums.rs` |
| 会话日志：plan/staging/manifest/对账 | `svault-core/src/session.rs` |
| 事件与交互边界（R3/R4） | `svault-core/src/event.rs`（`Event` / `EventSink` / `Interactor` / `NoopSink` / `YesInteractor`） |
| 文件传输 + 崩溃耐久原语 | `svault-core/src/fs.rs`（`transfer_file` / `atomic_commit` / `atomic_write` / `sync_file_and_dir`） |
| 数据库（SCHEMA / 事务 / 查询 / dump） | `svault-core/src/db/` |
| 三层哈希实现 | `svault-core/src/hash/`、`svault-core/src/media/crc.rs` |
| 媒体格式 / EXIF / RAW ID / 复合媒体绑定 | `svault-core/src/media/` |
| verify / 后台哈希 / hardlink 升级 / manifest 类型 | `svault-core/src/verify/` |
| sync diff 引擎（纯函数，无 IO） | `svault-core/src/sync/diff.rs` |
| vault 发现 / 进程锁 | `svault-core/src/context.rs` / `lock.rs` |
| 终端渲染 / JSON sink / pipe 协议 / 交互确认 | `svault-ui/src/{terminal,json,pipe,interact}.rs` |
| 命令入口 / 参数解析 | `svault-cli/src/{main,cli}.rs` + `svault-cli/src/commands/` |
| E2E 框架（VaultEnv、固件工厂） | `tests/e2e/conftest.py`、`tests/e2e/fixtures/` |
| FUSE 故障注入 | `tests/e2e/fuse_tests/` |
