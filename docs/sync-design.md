# Sync 设计文档

> 本文档覆盖三个命令的设计，它们共用同一套 diff + journal + transfer 引擎：
>
> | 命令 | 源 → 目标 | 用途 |
> |------|----------|------|
> | `svault sync` | vault → vault | 增量同步，把源端有而目标端没有的文件复制过去 |
> | `svault clone` | vault → 普通目录 | 导出文件子集（sync --export 的别名） |
> | `svault recover` | vault → vault | 从健康备份 vault 拉取完好副本替换本地损坏文件 |

## 目标

实现单向同步（pull/push），两个 vault 之间按 sha256 差集传输文件。不是分布式共识，不是 CRDT，不是 git merge。

## 核心原理

```
源端 sha256 集合  -  目标端 sha256 集合  =  传输清单
```

两边各一次 SELECT，不扫描文件系统。rsync 要做 rolling checksum 比对才知道差什么，svault 两边查 DB 就知道。

## 整体流程

```
0. 源端 pre-flight 健康检查（见下方）
1. 源端: SELECT sha256, xxh3_128, path, size FROM files WHERE status='imported'
2. 目标: SELECT sha256 FROM files
3. 差集 = { 源有 且 目标没有 的 sha256 }
4. 目标端写 sync_journal（差集全部标记 pending）
5. 源端遍历差集 → copy 文件到目标 objects/ + 文件路径
6. 目标每完成一个 → rename 到位 → insert DB → journal 标记 done
7. 中断后重连 → 读 journal → done 跳过，pending 继续
8. 全部完成 → 删除 journal
```

## 步骤 0：源端 pre-flight 健康检查

在计算差集之前，验证源端 DB 和磁盘状态的一致性。**任何不一致都阻止 sync 继续**，因为传播不一致的 DB 到备份端会放大问题。

### 检查项

| 检查 | 方法 | 问题 |
|------|------|------|
| DB 事件链完整性 | `db verify-chain` | 事件链被篡改，DB 不可信 |
| imported 文件存在 | os::exists + 大小比对 | 文件被删除但 DB 未标记 missing |
| 文件大小匹配 | fs::metadata.len() == row.size | 文件被替换但 DB 未更新 |
| xxh3 匹配（快速抽检） | xxh3_file == row.xxh3_128 | 内容被改变但 DB 未感知 |

### 策略

```
全部通过 → sync 继续
有 imported 缺失 → 报错退出，提示用户先运行 svault recheck
                 → 列出具体路径和错误
                 → sync 不继续（不能传播脏数据）
````

### 实现

不重新实现 `verify`/`recheck` 的全部逻辑。从 DB 中取 imported 文件列表，逐个检查文件是否存在和大小是否一致即可。xxh3 校验作为可选抽检（`--check-full` 开启），默认只做存在性 + 大小检查。

如果用户想跳过检查（风险自负），`svault sync --no-verify` 可强行继续。

## 传输清单 / sync_journal

放在目标端 `.svault/sync_journal.json`：

```json
{
  "source_id": "vault-uuid-...",
  "started_at_ms": 1700000000000,
  "files": [
    {
      "sha256": "abc123...",
      "path": "2024/03-15/DSC_1234.dng",
      "size": 45678901,
      "status": "done"
    },
    {
      "sha256": "def456...",
      "path": "2024/03-15/DSC_1235.jpg",
      "size": 2345678,
      "status": "pending"
    }
  ]
}
```

- 独立 JSON 文件，不进入事件溯源链
- 传输完成后删除
- 中断续传：读 journal → pending 的继续，done 的跳过
- checkpoint：每传完一个文件写一次（简单但可靠；大文件传输不频繁）

## Reporting 接口设计

sync 模块遵循和 import pipeline 相同的 reporting 架构：**core 定义 trait，CLI/GUI 实现渲染，sync 代码不直接输出**。

### 架构原则

```
svault-core/src/reporting/mod.rs   ← 新增 sync/recover 相关的 reporter trait
svault-core/src/sync/              ← 通过泛型接收 reporter，只调用 trait 方法
svault-cli/src/reporting/          ← 实现终端/JSON 渲染
```

sync 内部永远不调用 `println!`、`eprintln!`、`ProgressBar`。所有输出都走 reporter trait，所有用户确认都走 `Interactor` trait。

### 新增 ReporterBuilder 关联类型

在已有的 `ReporterBuilder` trait 中追加三组 sync 相关类型：

```rust
pub trait ReporterBuilder: Send + Sync {
    // ... 已有的 import/add/recheck/update/verify/history ...

    // ── sync / clone ─────────────────────────────────────────────────────────
    type SyncDiff: SyncDiffReporter;
    type SyncHealth: HealthReporter;

    fn sync_diff_reporter(&self) -> Self::SyncDiff;
    fn sync_health_reporter(&self, vault_root: &Path) -> Self::SyncHealth;
    // sync_transfer 复用已有的 CopyReporter（语义一致）

    // ── recover ──────────────────────────────────────────────────────────────
    type SyncRecover: RecoverReporter;

    fn sync_recover_reporter(&self) -> Self::SyncRecover;
}
```

### 三个新增 trait

**SyncDiffReporter** — 差集计算阶段：

```rust
pub trait SyncDiffReporter: Send + Sync {
    /// 开始比对，source_count / target_count 为两端 sha256 集合大小。
    fn started(&self, source_count: usize, target_count: usize);
    /// 差集结果：new_count 个文件待传输，total_bytes 待复制。
    fn diff_computed(&self, new_count: usize, total_bytes: u64);
    /// 目标端已经是最新，无需同步。
    fn nothing_to_sync(&self);
    fn finish(&self);
}
```

**HealthReporter** — 源端 pre-flight 检查阶段：

```rust
pub struct HealthIssue {
    pub issue_type: HealthIssueType,
    pub path: String,
    pub sha256: String,
}

pub enum HealthIssueType {
    FileMissing,
    SizeMismatch { expected: u64, actual: u64 },
    EventChainTampered,
}

pub trait HealthReporter: Send + Sync {
    fn started(&self, total: u64);
    /// 单个文件通过检查。
    fn item_ok(&self, path: &Path);
    /// 全量通过，无问题。
    fn all_clear(&self);
    /// 发现问题，sync 被阻止。CLI 据此渲染错误列表。
    fn blocked(&self, issues: &[HealthIssue]);
    fn finish(&self);
}
```

**RecoverReporter** — 损坏恢复阶段（专用于 recover 命令）：

```rust
pub struct DamagedFile {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub issue: String,
}

pub trait RecoverReporter: Send + Sync {
    /// 扫描发现 count 个损坏文件。
    fn damaged_found(&self, damaged: &[DamagedFile]);
    /// 在源 vault 找到了匹配的完好副本。
    fn match_found(&self, path: &str, source_vault: &Path);
    /// 所有源 vault 都没有此文件的副本。
    fn match_not_found(&self, path: &str);
    /// 开始替换流程。
    fn replace_started(&self, total: u64);
    /// 正在替换某个文件（损坏原件已移走）。
    fn item_replacing(&self, path: &str);
    /// 替换完成，corrupted_path 是损坏原件的新位置。
    fn item_replaced(&self, path: &str, corrupted_path: &Path);
    /// 用户选择跳过此文件。
    fn item_skipped(&self, path: &str);
    /// 最终摘要。
    fn summary(&self, replaced: usize, skipped: usize, unmatched: usize);
    fn finish(&self);
}
```

### 复用 vs 新增

| 已有，直接复用 | 理由 |
|----------------|------|
| `CopyReporter` | sync 文件复制 = import 的 Stage C，item_started → item_progress → item_finished 完全一致 |
| `Interactor::confirm()` | recover 的逐文件确认通过已有的 Interactor trait，GUI 模式不需要改 |

| 新增 | 理由 |
|------|------|
| `SyncDiffReporter` | import 没有"差集计算"这个独立阶段 |
| `HealthReporter` | import 没有对已有 vault 做 pre-flight 检查的概念 |
| `RecoverReporter` | 损坏恢复是新交互模型：损坏清单展示、匹配结果、替换进度 |

### JSON 输出示例

```
{"event":"sync_diff_started","source_total":1500,"target_total":1450}
{"event":"sync_diff_result","new_files":50,"total_bytes":1073741824}
{"event":"sync_diff_finished"}

{"event":"sync_health_started","total":1500}
{"event":"sync_health_item_ok","path":"2024/03-15/DSC_1234.dng"}
{"event":"sync_health_all_clear"}

{"event":"sync_health_blocked","issues":[
  {"type":"file_missing","path":"2024/03-15/DSC_9999.cr3","sha256":"..."}
]}

{"event":"recover_damaged_found","damaged":[
  {"path":"2024/03-15/DSC_1234.dng","sha256":"abc123","size":45678901,"issue":"hash_mismatch"}
]}
{"event":"recover_match_found","path":"2024/03-15/DSC_1234.dng","source":"/mnt/healthy-vault"}
{"event":"recover_item_replaced","path":"2024/03-15/DSC_1234.dng","corrupted_path":".svault/corrupted/DSC_1234.dng.1700000000"}
{"event":"recover_summary","replaced":2,"skipped":0,"unmatched":1}
```

### sync 代码调用方式

```rust
// sync 代码只调用 reporter trait 方法，不打印任何输出
fn run_sync<R: ReporterBuilder, I: Interactor>(
    opts: &SyncOptions,
    reporter_builder: &R,
    interactor: &I,
) -> anyhow::Result<SyncSummary> {
    // Phase 1: health check
    let health = reporter_builder.sync_health_reporter(&opts.source_root);
    health.started(total);
    // ... 检查每个文件 ...
    if issues.is_empty() { health.all_clear(); } else { health.blocked(&issues); return Err(...); }
    health.finish();
    drop(health);

    // Phase 2: diff
    let diff = reporter_builder.sync_diff_reporter();
    diff.started(src_count, dst_count);
    let manifest = compute_diff(...);
    diff.diff_completed(manifest.len(), manifest.total_bytes());
    diff.finish();
    drop(diff);

    // Phase 3: transfer — 复用 CopyReporter
    let transfer = reporter_builder.sync_transfer_reporter(&src, &dst, total);
    for file in &manifest {
        transfer.item_started(&file.src, &file.dest, file.size);
        // copy ...
        transfer.item_finished(&file.src, &file.dest, &result);
    }
    transfer.finish();

    Ok(summary)
}
```

## 模块边界

全部逻辑在 `svault-core/src/sync/` 中，**clone 和 sync 共用同一套引擎**，不分别实现。

| 模块 | 职责 |
|------|------|
| `mod.rs` | 公开入口 `SyncOptions::run()`，clone 和 sync 两个路径都进这里 |
| `health.rs` | 源端 pre-flight 一致性检查，任一项失败则阻止 sync |
| `diff.rs` | 比对两个 sha256 集合 → 差集 |
| `journal.rs` | sync_journal 读写、中断恢复，clone 和 sync 共用 |
| `transfer.rs` | 按差集传输文件（reflink → hardlink → copy fallback），共用 |

CLI 层只负责打开两端 vault → 调用 `SyncOptions::run(source_db, target_db, options)`。

## clone 与 sync 合并 —— 同一套代码的两个入口

clone 和 sync 不是两个独立实现。它们走完全相同的 diff → journal → transfer 管线，唯一区别在于目标的类型。

```
                  SyncOptions::run()
                        │
                  ┌─────┴──────┐
                  │            │
              vault→vault   vault→dir
              (sync)       (clone / sync --export)
                  │            │
                  └─────┬──────┘
                        │
              ┌─────────▼──────────┐
              │  diff.rs            │
              │  源: SELECT sha256  │
              │  目标 vault: SELECT │
              │  目标 dir: 空集     │
              │  → 差集 = 传输清单  │
              ├────────────────────┤
              │  journal.rs         │  ← 同一份 journal 逻辑
              │  写 sync_journal   │     中断续传 clone/sync 都适用
              │  标记 done/pending │
              ├────────────────────┤
              │  transfer.rs        │  ← 同一份传输逻辑
              │  遍历差集           │     reflink→hardlink→copy
              │  原子复制到目标     │
              │  tmp → rename      │
              ├────────────────────┤
              │  收尾               │
              │  vault 目标: 更新 DB│
              │  dir 目标:   跳过   │
              │  删除 journal       │
              └────────────────────┘
```

**两种模式共用的部分**：
- journal 中断续传（文件级 done/pending）
- transfer 复制策略（reflink → hardlink → copy）
- 原子写入（tmp → rename）
- 验校（xxh3 比对待确认文件）

**两种模式不同的部分**：

| | sync (vault → vault) | clone/sync --export (vault → dir) |
|---|---|---|
| 目标类型 | vault（有 DB） | 普通目录（无 DB） |
| 差集计算 | 源 sha256 - 目标 DB sha256 | 全量 = 源 sha256（目标无 DB 可查） |
| 目标路径 | 保持源路径 | 保持源路径 |
| DB 更新 | append event + insert file row | 跳过 |
| 过滤支持 | 无（全量同步） | --filter-date, --filter-camera |
| pre-flight 检查 | 源端一致性检查（带 --no-verify 可跳过） | 同 sync，入口相同 |

**当前 `clone.rs` 的处理方式**：
- 删掉 `svault-cli/src/commands/clone.rs`
- clone CLI 命令转为 `sync --export` 的别名，参数映射：`--target` → 目标路径, `--filter-date` / `--filter-camera` 保留
- 过滤逻辑从当前 clone.rs 移到 `sync::diff` 中，作为可选的 `SyncFilter` 参数

## 第三种模式：损坏恢复（repair）

一块硬盘部分数据损坏但 DB 完好，用另一块健康硬盘的对应文件来替换。这是 sync 的第三个入口，和 clone/sync 共用 diff + journal + transfer 引擎。

### 场景

```
本地 vault:  DB 完好，但部分文件被 flip bits / 坏扇区损坏
远程 vault:  健康的副本（同一批照片的另外一个备份）
```

### 和普通 sync 的本质区别

| | sync | repair |
|---|---|---|
| 目标文件状态 | 文件缺失（DB 有记录但路径无文件） | 文件存在但内容损坏（hash 和 DB 不匹配） |
| 操作 | 新增目标不存在的文件 | 替换目标已损坏的文件 |
| 坏文件处理 | 无 | 移到 `.svault/corrupted/` **不删除** |
| 用户确认 | 整体确认 Proceed? | **每个损坏文件逐条确认** |
| 源端选择 | 唯一源 | 可以选择多个源 vault 去查 |

### 流程

```
0. 在目标端运行 verify → 找出所有损坏文件（hash 不匹配）的 sha256 列表
1. 连接到源 vault → SELECT sha256 FROM files（健康库的文件索引）
2. 对每个损坏的 sha256：
   a. 在源端 DB 查找 → 存在 → 源端有这个文件的完好副本
   b. 在源端 DB 查找 → 不存在 → 标记 unmatched，跳过
3. 生成修复清单（只含源端有对应副本的损坏文件）
4. 逐文件向用户确认：
   "file.dng is corrupted (sha256 mismatch). Replace from source? [y/N/a/q]"
   y=替换  N=跳过  a=全部替换  q=退出
5. 确认的文件：
   a. 损坏文件 → mv 到 .svault/corrupted/file.dng.{timestamp}
   b. 从源端 → copy 到目标端原路径
   c. 验校新文件的 sha256
   d. journal 标记 done
6. 中断续传：和 sync 一样，journal 记录修复进度
7. 完成后输出报告：
   - 修复了哪些文件
   - 哪些文件源端也没有（仍然损坏）
   - 损坏的原件在 .svault/corrupted/ 中
```

### 为什么不删除损坏文件

- 用户可能想自行尝试恢复（hex 比较、工具修复）
- 损坏文件本身也可能包含部分可恢复的像素数据
- 安全原则：svault 从不静默删除

### 逐条确认的设计

```
$ svault recover --from /mnt/healthy-vault
Scanning... 3 files damaged, 2 can be recovered from source.

  2024/03-15/DSC_1234.dng (45MB, sha256 mismatch)
  → Good copy found in /mnt/healthy-vault
  Replace? [y/N/a/q] y

  2024/03-15/DSC_1235.jpg (8MB, sha256 mismatch)
  → Good copy found in /mnt/healthy-vault
  Replace? [y/N/a/q] y

  2024/06-20/IMG_8888.cr3 (32MB, sha256 mismatch)
  → NOT found in /mnt/healthy-vault (source doesn't have this file)
  Skipped.

Recovery summary:
  Replaced: 2  (damaged originals moved to .svault/corrupted/)
  Skipped:  0
  Unmatched: 1  (source vault does not have a copy)
```

### 多源查询

```bash
# 从多个备份 vault 查找缺失的文件
svault recover --from /mnt/hdd-backup --from /mnt/nas-backup
# 任一源有对应 sha256 就可以恢复
```

### 实现

和 sync 共用 diff/transfer/journal，新增 `recovery.rs` 专门处理：
- 损坏清单（来自本地 verify 结果）
- 源端匹配查询
- 损坏文件 mv → `.svault/corrupted/`
- 逐条交互确认（复用已有的 Interactor trait）

## 复制策略

源端 objects/ 文件到目标端 objects/：

```
本地同文件系统: reflink → hardlink → copy
跨文件系统:     copy only
```

目标端文件路径保持和源端一致（`path` 字段不变）。

## 数据库集成

目标端收到一个新文件后，需要插入 files 表。**复用已有的 `insert_file_row`**，但需要通过事件机制写入：

1. 构造 `file_imported` event
2. `append_event()` + `insert_file_row()` in transaction
3. 事件链完整——sync 行为也被审计

如果目标端是纯目录（非 vault），跳过 DB 步骤，只有文件复制 + journal。

## 不做什么

- 不做双向同步 / 冲突解决
- 不做文件删除同步
- 不做分块续传（文件级别的原子操作足够）
- 不做远程传输（传输层独立于同步逻辑，当前只做本地）
- 不修改源端 DB
- 不合并事件链（各 vault 保持独立的事件链）

## CLI 命令设计

```bash
# vault → vault 增量同步（全量，不做过滤）
svault sync /mnt/backup/vault

# vault → 普通目录（继承当前 clone 的过滤能力）
svault sync /mnt/backup/work --export --filter-date 2024-05-01..2024-05-31

# clone 保留为 sync --export 的别名
svault clone --target /mnt/backup --filter-date 2024-03-01..2024-03-31

# 损坏恢复：从健康的备份 vault 拉取完好副本替换损坏文件
svault recover --from /mnt/healthy-vault
svault recover --from /mnt/hdd-backup --from /mnt/nas-backup    # 多源
```

## 实现优先级

1. `sync::health` — 源端 pre-flight 一致性检查（文件存在性 + 大小匹配）
2. `sync::diff` — sha256 差集计算（纯内存操作，不涉及 IO）
3. `sync::journal` — JSON journal 读写、中断恢复
4. `sync::transfer` — 按差集复制文件
5. `sync::run` — 串联 health → diff → journal → transfer 全流程
6. CLI wiring — 替换 clone，新增 sync 命令

## 关键设计决策

- **journal 是独立 JSON 文件**，不像 DB 事件那样哈希链保护。journal 是临时清单，丢了重建即可
- **单向 sync 不合并事件链**，目标端生成自己的 `file_imported` 事件
- **只比 sha256**，如果源端两个文件 sha256 相同（内容相同），目标只需要一个。内容寻址天然 dedup
- **传输清单不需要 xxh3**，因为 sha256 已经是终局身份。xxh3 在 sync 阶段不参与比对
