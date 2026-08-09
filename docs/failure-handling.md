# 故障处理决策矩阵（Failure Handling Decisions）

> **本文档是 Svault 故障处理行为与故障注入测试判据的单一事实源。**
>
> - 任何故障注入测试的判据 MUST 以本文档为准；`tests/e2e/fuse_tests/VALIDATION_PLAN.md`
>   中的测试场景只有与本文档判据一致的部分有效。
> - 本文档与代码冲突时，按仓库惯例**当场修正其一**（修代码或修文档），
>   不允许两者长期不一致（见 docs/REFACTOR-2026-04.md §遗留规则）。
> - 其他文档（import-pipeline.md / cli.md / database-schema.md）
>   中与故障行为相关的段落，凡与本文档冲突，以本文档为准并应回改。
> - 每条决策标注代码证据（`文件:行号`）。行号会腐烂，仅作定位参考；符号名优先。
> - 状态标记：**[VERIFIED]** = 已对照代码核实；**[OPEN-n]** = 待维护者拍板（见 §9）。

适用范围：svault-core / svault-cli 当前工作树（2026-08-05 核实；2026-08-09 import staging 模型更新）。

---

## 1. 故障分类（Taxonomy）

| 类别 | 说明 | 典型注入手段 |
|------|------|--------------|
| F1 IO 错误 | 读/写系统调用返回 errno（EIO、ENOSPC、EAGAIN、EXDEV…） | FUSE `error` action、strace inject |
| F2 中断 | 进程被信号杀死或机器断电，发生在任意阶段 | SIGTERM/SIGKILL/SIGINT、strace 信号注入 |
| F3 静默损坏 | 读返回错误数据但不报错（bit rot、坏道） | FUSE `corrupt` action（**未实现**，见 §8.4） |
| F4 不稳定存储 | 同一文件多次读取返回不同数据 | FUSE 动态规则（**未实现**，见 §8.4） |
| F5 延迟 | 读写极慢或抖动 | FUSE `delay` action |
| F6 并发冲突 | 多进程操作同一 vault | 多进程并发测试 |

## 2. 全局原则（不可妥协）

### G1 永不删除用户文件 [VERIFIED；2026-08-09 精确化]

任何命令不得删除**用户**磁盘文件。唯一例外：`.svault/staging/import/`
内由 svault **自己创建**的导入暂存残留（中断半成品、Stage-D 重复副本），
允许由 svault 自己清理（`pipeline/staging.rs::reconcile` 对账清除 +
`ops/import.rs` Stage E 提交后清空 session 目录）——这些不是用户文件，
用户源文件从不被触碰。故障路径上其余 `remove_file` 仅 `fs.rs` hardlink
能力探测的临时文件。`update --delete` 因违反本原则被移除
（docs/PARKED.md §3）。

### G2 无重试、无超时、无信号处理 [VERIFIED]

全 svault-core / svault-cli **不存在**：
- 任何重试逻辑（无 retry / EAGAIN / EINTR 显式处理；仅依赖 std 库对
  `ErrorKind::Interrupted` 的内部重试）
- 任何超时 / deadline 机制
- 任何信号处理器（无 ctrlc / signal-hook 依赖；SIGINT/SIGTERM 即硬杀，
  无清理钩子）

**推论**：故障的应对策略只有两种——**跳过该文件继续**（逐文件隔离，G3），
或**整个命令报错退出**（致命错误，G4）。恢复一律靠**幂等重跑**（§6），
不靠断点续传状态（`.pending` 设计从未实现且已移除，
docs/import-pipeline.md §原子性与可恢复性）。staging 对账
（`pipeline::staging::reconcile`，2026-08-09 起）只做两件事：补完成
"已入库未 rename"的 rename、清除"未入库"的暂存残留——**不利用**已复制
未入库的暂存文件续传（无法低成本区分完整与半截副本，见 §5 注）。

### G3 逐文件失败隔离 [VERIFIED]

单文件 IO 错误 NEVER 中止整个命令：该文件计入失败/跳过，其余文件继续处理。
各阶段证据：

| 阶段 | 行为 | 证据 |
|------|------|------|
| 扫描/遍历 | Err 项流过 channel，发 `ScanItem{status:Failed}`，`failed_files+=1`，continue | `ops/import.rs:290-301` |
| 复制 | `transfer_file` 失败发 `CopyFinished{error}`，`filter_map` 丢弃，继续 | `ops/import.rs:616-621` |
| 哈希 | 失败发 `HashFinished{error}`，继续（**但有 BUG-1**，见 §7） | `pipeline/hash.rs:47-55` |
| sync 传输 | `summary.failed+=1`，continue | `ops/sync.rs:224-226` |
| clone 传输 | `failed+=1`，continue | `ops/clone.rs:131-133` |
| verify 校验 | 逐文件发 `VerifyItem`，不中止 | `verify/mod.rs:183-190` |

### G4 退出码契约 [VERIFIED]

`main.rs:104-116`：`run()` 返回 `Err` → 友好化错误信息到 stderr + **exit 1**；
否则 **exit 0**。**逐文件失败不产生 `Err`**，因此：

| 命令 | 部分文件失败 | 致命错误（锁冲突 / DB 写失败 / manifest 写失败 / 遍历根失败） |
|------|:---:|:---:|
| `import` / `add` | **0** | 1 |
| `sync` / `clone` | **0** | 1 |
| `verify` | **1**（missing/size mismatch/hash mismatch/IO error 任一类计数 > 0，`commands/verify.rs:73-80`） | 1 |
| `recheck` | **0**（恒返回 Ok，不一致只写报告，`ops/recheck.rs`） | 1（manifest 加载失败等） |
| `update` | 0 | 1 |

注：`import` 唯一对逐文件失败 bail 的例外是 debug-only 的 `scan` 命令
（`failed_files>0` 即 Err，`ops/import.rs:412-414`）。
import 的 `ImportSummary.failed` 不含扫描阶段失败（只含复制+哈希，
`ops/import.rs:721`）——统计口径判据见 §8。

import 与 verify 对"部分失败"的退出码语义不同（0 vs 1），登记为 **[OPEN-6]**。

### G5 事件溯源 + WAL 事务 [VERIFIED]

- DB 以 WAL 模式打开（`db/mod.rs:55`）。
- 写协议：状态变更应经 `append_event` 在同一事务内写事件 + 物化视图
  （`db/mod.rs:7-13`）。**例外：`update` 命令直接 UPDATE 绕过协议（BUG-2，§7）。**
- import/add/sync 的入库是**整批单事务**（`pipeline/insert.rs:204`），
  失败整体回滚（`db/mod.rs:151-158`）；事务提交后追加单条 `batch.imported`
  汇总事件（`insert.rs:278-284`）。
- events 表**不记录逐文件错误**；逐文件错误只出现在 Event 流（UI/JSON 输出）
  和 manifest 中（且 manifest 只覆盖进入 Stage E 的文件——扫描失败和
  复制失败的文件不进 manifest）。

### G6 进程锁 [VERIFIED]

- 所有经 `VaultContext::open_at` 的命令（含只读的 verify/recheck）对
  `<vault>/.svault/lock` 取 flock 排他咨询锁（`lock.rs:19-33`、
  `context.rs:84-85`）。
- 锁冲突 → `Err("another svault process is already running…")` → exit 1，
  **不重试不等待**。
- sync 的**源** vault 用 `Db::open_readonly`（`SQLITE_OPEN_READ_ONLY`，
  `db/mod.rs:80-86`），**不取锁、不迁移**。

### G7 传输策略链与 import staging [VERIFIED；2026-08-09 重写]

- 按 `--strategy` 列表顺序尝试（默认仅 `reflink`）：Reflink（FICLONE ioctl，
  失败静默降级）→ Hardlink（失败静默降级）→ 循环结束后 **stream copy 无条件
  兜底**（`fs.rs` `try_transfer`；空策略列表也兜底）。
- Stream copy 是终态：其错误**不再降级**，直接向上传播。
- 跨文件系统：reflink 跨设备 ioctl 失败归一化为"不支持"→ 静默降级 copy，
  不区分具体 errno。
- **import 走 staging 原子提交**：复制目标是
  `.svault/staging/import/<session_id>/`（镜像最终相对路径，
  `pipeline/staging.rs`），传输成功后 `fs::sync_file_and_dir` 落盘，
  Stage D 在**暂存副本**上算哈希，Stage E 整批事务入库（记录最终路径）
  成功后才逐个 `fs::atomic_commit`（rename + 目录 fsync）搬到最终路径。
  **不变量：最终路径可见 ⟹ 已完整复制 + 哈希 + 入库**；半成品永不进入
  用户可见目录树。rename 失败非致命：发 `Hint::StagedCommitDeferred`，
  下次 import 对账补齐。reflink 失败的空文件、copy 写失败的截断文件
  只可能残留在 staging 内，由对账/Stage E 后清理清除（G1 例外）。
- **sync/clone 不在 staging 范围内**：仍直达最终路径复制，中断残留
  （半成品/孤儿）语义维持原状。
- 死代码：`capabilities_for` / `best_strategy`（`fs.rs`）无调用者，
  sync/clone 不做传输预检。

---

## 3. 逐命令故障语义

### 3.1 `import`（`add` 共享管线，差异注明）

管线：Stage A 扫描（jwalk 线程→mpsc）→ Stage B CRC（100 条攒批 + rayon）
→ Lookup 查重（串行内联 `ops::check_duplicate`）→ Preflight + 用户确认
→ Stage C 复制到 **staging**（串行 EXIF 路径解析 + rayon 并行传输 +
fsync，目标 `.svault/staging/import/<session>/`）→ Stage D 强哈希
（对**暂存副本**计算 + 二次去重）→ Stage E 整批入库 + manifest +
**commit 后 rename 到最终路径**（G7）。

| 故障场景 | 行为 [VERIFIED] | 可观测证据 |
|----------|----------------|------------|
| 源文件读取 EIO（Stage B） | 该文件 `ScanItem{Failed}`，跳过继续 | Preflight 事件 failed 计数；退出码 0 |
| 源文件读取 EIO（Stage C） | 该文件 `CopyFinished{error}`，跳过继续 | 退出码 0；manifest 无此文件 |
| vault 目标哈希 EIO（Stage D） | 计入 **failed**，跳过继续（BUG-1 已于 `d970fd0` 修复：结构化 `hash_error` 字段替代前缀匹配） | 退出码 0；manifest status=Failed |
| 进程在 Stage A–D 被杀死 | DB 无任何记录；已复制部分留在 `.svault/staging/import/<session>/`，**最终路径不可见** | 下次 import 对账清除 staging 残留后重跑幂等（§5） |
| 进程在 Stage E 事务中被杀死 | 事务回滚，DB 无记录；已复制文件留在 staging（不再产生最终路径孤儿） | 同上 |
| 事务提交后、rename 前被杀死 | DB 有记录，文件在 staging | 下次 import 对账补 rename，零重复制自愈 |
| 事务提交后、manifest 写入前被杀死 | DB 有记录，无 manifest，文件在 staging | 对账补 rename；recheck --session 找不到该会话 |
| manifest 写入失败（ENOSPC） | Err → exit 1；DB 已提交不回滚 | stderr 报错 |
| DB 写入失败（ENOSPC/锁） | 整批回滚，Err → exit 1 | 已复制文件成孤儿 |
| 源路径在 vault root 内 | 拒绝导入（自保护） | Err → exit 1 |
| 0 字节文件 | 无特判，正常导入；同扩展名 0 字节文件互判 duplicate（size=0 且 CRC 相同） | manifest |

### 3.2 `verify` [VERIFIED]

- 哈希选择：DB 有 SHA-256 用 SHA-256，否则 XXH3-128（`verify/mod.rs:93-128`）。
- 结果六态：Ok / Missing / SizeMismatch / HashMismatch / IoError / （Skipped）。
- 只读命令：**不写 events 表、不改文件**（唯一例外：`--background-hash`
  补算 SHA-256 并写 `file.sha256_resolved` 事件，`verify/background_hash.rs:69-77`）。
- 逐文件 IO 错误：计入 io_error，继续后续文件；四类失败任一 > 0 → exit 1。
- `--upgrade-links`：hardlink 升级走"同目录临时文件 + fsync + 原子 rename"
  （`verify/hardlink_upgrade.rs:44-73`）；中途失败**原文件不变**，临时文件
  残留；升级失败仅 warn，不影响退出码（`commands/verify.rs:96-122`）。

### 3.3 `recheck` [VERIFIED]

- 唯一的**源-vs-vault 显式哈希比对**层（`ops/recheck.rs:60-123`），基于
  manifest 记录分别重算两侧哈希。
- 七态 RecheckStatus（`ops/recheck.rs:24-41`）。
- 只读 DB、不改任何文件；报告写 `.svault/staging/recheck_<新时间戳>.json`
  （`ops/recheck.rs:207-252`；status 字段为 Rust Debug 格式，非 snake_case）。
- **恒 exit 0**：不一致由用户读报告后自行决策，这是有意设计
  （`cli.rs:93`：“No files are imported or deleted”）。
- source 参数与 manifest 的 source_root 不符 → Err → exit 1。

### 3.4 `update` [VERIFIED]

- moved 匹配依据：XXH3-128 + size 初筛，SHA-256 确认（`ops/update.rs:139-171`）。
- 找不到的文件标记 `status='missing'`，**永不删除**（`ops/update.rs:261-269`）。
- 确认默认 No；用户拒绝时**连 missing 标记都跳过**（`ops/update.rs:215-228`）。
- 非终端输出（JSON / 管道）回退 YesInteractor 自动确认（`commands/update.rs:27-39`）。
- **BUG-2**：路径修正与 missing 标记均为直接 SQL UPDATE，绕过 `append_event`
  写协议，events 表无记录（`db/files.rs:237-253`）。
- missing 记录可被同哈希再导入"复活"（`Recover` 路径，`ops/mod.rs:90-95`、
  `insert.rs:225-238`：事务内 UPDATE path + status='imported'）。

### 3.5 `sync` [VERIFIED]

- 前置检查仅有：源路径存在、源≠目标、源有 `.svault/vault.db`。
  **无 health pre-flight**（该设计未实现，已随 sync-design.md 删除，
  见 PARKED §6）。
- 源 DB 只读打开（G6）；比对只取 `status='imported'` 记录
  （`ops/sync.rs:110,116`）。
- diff 依据 identity = SHA-256 优先、XXH3-128 兜底（`sync/diff.rs:37-39`）；
  五分类：Identical / OnlySource / OnlyDest / Moved / Conflict；
  同路径不同 hash → Conflict，保留目标不复制；双方无 hash 的 OnlySource
  计入 skipped_hashless 不复制（`sync/diff.rs:146-212`）。
- 单文件传输失败：failed+=1 继续；退出码仍 0（G4）。
- **无 journal**：全部复制完成后才整批 batch_insert（SessionType::Sync +
  manifest）。中断 → "文件已复制但 DB 无记录"，重跑靠 diff 重新复制覆盖
  （文件级重传，非断点续传）。
- 完成后按 `--verify`（默认 norm）做同步后校验。
- dest missing 记录在复制同 hash 文件时按既有 recover 逻辑复活
  （`insert.rs:95-99`）。

### 3.6 `clone` [VERIFIED]

- 独立实现（**不是** sync --export 别名，`ops/clone.rs:65-177`）。
- 选 `status='imported'`（可 `--filter-date`）→ 逐文件 transfer_file
  （失败 failed+=1 继续）→ 目标目录写 `svault-clone-manifest.json`
  → 源 DB 写 `vault.cloned` 审计事件。
- 拒绝目标在 vault 内（`clone.rs:76-81`）。
- CLI 层对本地 vault 持排他锁 + 读写打开（因要写审计事件）——
  ARCHITECTURE.md §6.1 "只读源 vault" 措辞不准确，实际是"不修改文件记录"。

### 3.7 `db verify-chain` [VERIFIED]

事件链防篡改校验；`events` 表被篡改 → 校验失败 → exit 1
（E2E `AI-DB-001` 场景，docs/ai-user-testing.md §C）。

---

## 4. 损坏与静默损坏（F3/F4）的检测能力边界

这是归档工具的**核心局限**，判据必须如实锁定，不得虚构防御能力：

| 场景 | 现行行为 [VERIFIED] | 理由 |
|------|--------------------|------|
| 导入时源数据已损坏（坏道返回错数据） | **无法检测**：哈希基于损坏数据计算并入库，verify 永远通过（H_bad == H_bad） | 无外部参考 |
| 导入后 vault 文件 bit rot | **可检测**：verify 报 hash_mismatch，exit 1 | DB 哈希是导入时的参考 |
| 源不稳定（多次读返回不同数据） | **无法检测**：Stage B CRC 用一次读、Stage C 复制用一次读、Stage D 哈希 vault 副本——三处可能各读各的，导入路径**无源-目标哈希比对** | 写后校验只是"vault 副本自洽"（隐式读回） |
| 上述场景的兜底手段 | `svault recheck`：按 manifest 重算**源侧**哈希比对，能发现源侧变化/损坏 | `ops/recheck.rs:60-123` |

**决策**：F3/F4 类测试的判据 MUST 断言"现行检测能力边界"（如上表），
并在测试名/注释中明确这是**局限性确认测试**而非缺陷。真正的修复手段
（多副本比对、外部校验、`svault recover` 损坏恢复）属于已暂缓的设计
（docs/PARKED.md §6；另见 §7 DRIFT-2、[OPEN-5]）。

---

## 5. 恢复语义（幂等重跑矩阵）

中断后**唯一的恢复路径是重新执行同一命令**（import 重跑前会先经
`pipeline::staging::reconcile` 对账 staging 残留）。逐中断点的保证：

| 中断点 | 系统状态 | 重跑结果 [VERIFIED] |
|--------|----------|--------------------|
| import Stage A–D | DB 无记录；已复制部分在 staging，最终路径不可见 | 对账清除 staging 残留；全部重新处理 |
| import Stage E 事务中 | 事务回滚 | 同上 |
| import commit 后、rename 前 | DB 有记录；文件在 staging | 对账补 rename，零重复制 |
| import 完成后 | DB + manifest 完整；staging 已清空 | CRC32C 短路：秒级跳过，全部 duplicate |
| 重跑时 CRC 命中 | — | 不复制、不入库（`db/files.rs` 查 size+CRC+扩展名） |
| 重跑时 CRC 未命中但哈希相同 | — | 复制到 staging 后 Stage D 查 `files.xxh3_128` 判重复、**不入库**；暂存副本由 Stage E 后清理/对账清除，**不再在最终路径残留第二份物理副本**（2026-08-09 起） |
| sync 复制中途 | 部分文件已复制，DB 无记录 | diff 重算，重新复制覆盖 |
| sync 入库后 | DB + manifest 完整 | diff 全 Identical，空计划 |

**注（为何不利用 staging 残留续传）**：中断点任意，staging 内的无记录
文件无法低成本区分"完整副本"与"写了一半"——唯一可靠判别是源/暂存两侧
重算哈希比对（各读一遍），省下的仅一次写 IO；且 staged→source 反向映射
需要持久化元数据（即已暂缓的 sync_journal 范畴，PARKED §6）。因此一律
清除后重复制。

**已知边界（非 bug 但须锁定）**：
- ~~"已复制未入库"的孤儿文件无法被任何去重层识别，重跑改名重复制~~
  **已闭环（2026-08-09，[OPEN-3]）**：staging 模型下孤儿只存在于
  `.svault/staging/`，对账清除，最终路径不再有不可识别副本。
  docs/import-pipeline.md "由 Stage D 去重识别"的说法仍然错误。
- `--force` 是有意破坏幂等的开关：跳过 CRC 短路 + 跳过二次去重 +
  跳过按路径检查 → 重复内容会复制并插入第二条 DB 记录
  （`lookup.rs`、`import.rs`、`insert.rs`）。

---

## 6. 命名与统计口径（判据用词必须精确）

- **"CRC32C" 实为 CRC-32/ISO-HDLC**：导入管线用 `crc32fast`
  （`media/mod.rs:127-129`）；真正的 crc32c crate 仅用于 verify 的
  Stage 3 probe（`hash/mod.rs:26-28`）。测试断言 CRC 值时注意。
- **ImportSummary.failed = 复制失败 + 哈希失败**，不含扫描阶段失败
  （`ops/import.rs:721`）；扫描失败只见于 Preflight 事件。
- **manifest 只覆盖进入 Stage E 的文件**；扫描失败、复制失败的文件
  不在 manifest 中。
- **manifest 非原子写入**（直接 `fs::write`，`manifest.rs:142-145`）；
  session_id 为 Unix 秒，同秒同类会话 manifest 互相覆盖。登记 **[OPEN-4]**。
- files 表**无唯一约束**（`idx_files_xxh3_128` 等均为普通索引，
  `db/mod.rs:365-368`），import-pipeline.md "层 3 唯一约束兜底"不存在。

---

## 7. 缺陷与漂移登记

### 代码缺陷

| 编号 | 描述 | 证据 | 影响 |
|------|------|------|------|
| BUG-1 | ~~`insert.rs:121` 用 `reason.starts_with("hash error")` 判定哈希失败，但实际消息前缀永不匹配 → 哈希 IO 错误被计为 duplicate~~ **已修复（`d970fd0`，2026-08-05）**：`HashResult.hash_error` 结构化字段替代字符串前缀匹配；回归单测 `batch_insert_classifies_hash_error_as_failed_not_duplicate` | — | 已闭环 |
| BUG-2 | `update` 的路径修正/missing 标记直接 UPDATE，绕过 append_event 写协议，events 表无记录 | `db/files.rs:237-253` | 事件溯源完整性受损 |
| BUG-3 | manifest 非原子写入，写一半中断留截断 JSON | `manifest.rs:142-145` | recheck 加载失败 |
| BUG-4 | session_id 为 Unix 秒，同秒同类会话 manifest 互相覆盖 | `ops/utils.rs:14-19` | manifest 丢失 |

### 文档漂移（以本文档为准，应回改原文档）

| 编号 | 文档 | 漂移内容 |
|------|------|----------|
| DRIFT-1 | import-pipeline.md | 11 处不符：lookup_stream 不存在（串行内联）；层 3 唯一约束不存在；无 500 条分批提交（整批单事务）；crc32c 只在 Stage E 落库；duplicate 从不写 files 表；无 import_sessions 表；manifest 是 JSON 非文本；孤儿文件 Stage D 无法识别；无归档文件清理；CRC32C 命名 |
| DRIFT-2 | ~~sync-design.md~~（2026-08-05 已删除） | 原设计稿大量未实现且基于已移除的 reporter trait 体系：health pre-flight、sync_journal 断点续传、tmp→rename、clone=sync --export、"只比 sha256"、`svault recover` 命令。recover 等暂缓设计已抢救至 PARKED §6；原文见 `git show f34d53b:docs/sync-design.md` |
| DRIFT-3 | ARCHITECTURE.md §6.1 | "只读源 vault" 措辞：clone 实际读写打开源 DB 并写 vault.cloned 审计事件 |
| DRIFT-4 | cli.md / database-schema.md | 事件名示例（file.imported / file.path_updated）与实际（batch.imported / file.sha256_resolved / vault.cloned）不符；recheck 报告 session 实为新时间戳；recheck 报告 status 为 Debug 格式 |

---

## 8. 故障注入测试判据

`VALIDATION_PLAN.md` 中每项计划的处置。**判据用词 MUST 用本文档 §6 的
精确口径**；每项判据 MUST 锁定可观测证据（退出码 / Event 流 / manifest /
DB 查询 / 文件系统状态）。

### 8.1 P0（先行实现）

| 计划测试 | 处置 | 判据（按现行行为） |
|----------|------|-------------------|
| `test_import_pause_at_25_percent` | ✅ 实现 | pause 于源读取；SIGTERM 后：DB 无该文件记录；**最终路径无半成品**（staging 模型，G7；残留只在 `.svault/staging/`，下次 import 对账清除）；故障解除后重跑 import 完成；`verify` 通过 |
| `test_import_pause_at_50_resume` | ✅ 实现 | 释放 pause 后 import 自行完成；manifest 完整；vault 副本哈希与源一致 |
| `test_import_eio_at_offset` | ✅ 实现 | 该文件 ScanItem/CopyFinished 报 error；**退出码 0**；其余文件正常导入；manifest 不含该文件；解除故障重跑后该文件成功导入 |
| `test_recheck_pause_at_half_files` | ✅ 实现（改写） | recheck 无状态可续——判据改为：中断后**无报告或报告截断**；重跑生成完整新报告；两侧比对结果正确 |
| `test_corrupted_hash_undetectable_by_verify` | ✅ 实现（依赖 corrupt action，§8.4） | **局限性确认测试**（§4）：import 损坏数据 → verify 通过（H_bad==H_bad）→ recheck 报 mismatch。断言三者 |

### 8.2 P1

| 计划测试 | 处置 | 判据要点 |
|----------|------|----------|
| `test_import_enospc_simulation` | ⏭ **不重复建设，占位已删除**（2026-08-05）：`test_import_disk_full.py` 已用 loopback ext4 覆盖等价场景（copy 路径 ENOSPC + DB 写失败 exit 1 + 清理后恢复）。判据以该文件实际断言为准（其"exit code 4"注释陈旧，实际断言 `[4, 1]` 兼容现行 exit 1） | — |
| `test_import_pause_multiple_files` | ✅ 实现 | 暂停点前的文件已完成入库（如已过 Stage E）或未入库（未过）；重跑后全部一致 |
| `test_recheck_source_modified_during_check` | ✅ 实现 | 源侧校验值与 manifest 不符 → 报告中对应状态；exit 0 |
| `test_silent_corruption_at_specific_offset` | ✅ 实现（依赖 corrupt action） | 同 §4 局限性确认 |
| `test_unstable_read_during_import` | ✅ 实现（依赖动态规则，§8.4） | **局限性确认**：断言现行行为=无法检测（导入成功，crc32c 与 vault 内容可能不一致）；recheck 可发现 |
| `test_import_eagain_retry` | ⚠️ **改写并按实测修正**（2026-08-05）：svault 无重试层（G2）结论不变，但实测 **FUSE 内核客户端对 EAGAIN 透明重试**——瞬态 EAGAIN 被内核吸收，导入正常完成。改名 `test_import_eagain_error`，判据：注入 3 次 EAGAIN（error_count==3）→ import exit 0 全入库 |
| `test_import_slow_read_timeout` | ❌ **删除** | 无超时机制（G2），计划前提不成立。可改为 `test_import_slow_read_completes`：慢速读取只是慢，最终正常完成（归 P2 稳定性） |

### 8.3 P2（深度验证）——实施状态（2026-08-05 已全部处置；2026-08-06 冗余测试经评估删除）

- ✅ `test_edge_cases_fuse.py`：已创建——空文件 × 故障规则 2 例
  （corrupt 规则对空文件不触发；同扩展名空文件互判 duplicate，§3.1）。
- 🔧 保留待实现（需新设施）：`test_corruption_during_copy_to_vault`、
  `test_recheck_vault_file_eio`（dm-flakey）；`test_import_corrupt_at_offset`；
  `test_import_truncated_file`（需 truncate action）。
- 🗑 **已删除——永不实现（前提证伪）**：`test_verify_pause_resume`（verify 不读源，
  源侧 FUSE 无注入面）、`test_verify_across_different_storage`（"带重试"假设
  违反 G2）、`test_parity_verification_detects_corruption`（无 parity 功能）、
  `test_multiple_hash_algorithms_detect_corruption`（三层哈希是串联身份链，
  非并行冗余）。
- 🗑 **已删除——已有等价覆盖**：`test_import_enospc_simulation`（loopback 满盘）、
  `test_verify_partial_failure` / `test_recheck_vault_file_corrupt` /
  `test_bit_rot_detection`（test_verify.py 系列）、`test_intermittent_corruption` /
  `test_post_import_source_recheck_detects_corruption`（P0-5/unstable 已含）、
  `test_bad_sector_during_import`（= eio_at_offset）。
- 🗑 **已删除——冗余通过项**：`test_silent_corruption_at_specific_offset`
  （与 P0-5 同构，XOR 路径由设施自验 `test_corrupt_xor_flip` 锁定）、
  `test_empty_file_eio_rule_never_fires`（与 corrupt 版同机制）、
  `test_import_variable_delay`（无独有故障判据）、
  `test_aging_hard_drive_simulation` / `test_network_storage_interruption`
  （组合场景，组件已被单项覆盖）。

### 8.4 测试基础设施需求（先于测试实现）

| 编号 | 需求 | 现状 | 服务测试 |
|------|------|------|----------|
| INFRA-1 | `corrupt` action 落地：read 返回后按 `corrupt_data` 改写字节 | ✅ **已实现**（2026-08-05）：post-read 缓冲区改写 + XOR 0xFF 默认路径 | 全部 corruption 测试 |
| INFRA-2 | 运行时规则变更 API（测试中途启用/停用/修改规则，线程安全） | ✅ **已实现**：`enabled` 字段 + `set_rules`/`clear_rules`/`enable_rule`/`disable_rule` | bit_rot、unstable_read、aging |
| INFRA-3 | ~~vault 侧挂载 fixture~~ **已评估取消**（2026-08-05）：ENOSPC 已被 `test_import_disk_full.py`（loopback ext4 真实满盘）覆盖；vault 侧 EIO/损坏在 P2 用 dm-flakey 或直接字节翻转实现。完整读写 FUSE 挂 vault（SQLite over FUSE）风险大于收益 | 现有 fixture 仅挂 source 侧 | （已覆盖，见 §8.2） |
| INFRA-4 | "每次读返回不同数据"规则（per-read 内容序列） | ✅ **已实现**：`corrupt_sequence` 字段 | unstable_read |

### 8.5 已覆盖、不重复建设

- 信号中断恢复：`tests/e2e/test_import_interruption.py`（strace 信号注入；2026-08-06 套件重设计后 7 例，其中 strace 中断 3 例：SIGTERM 重跑、write 阶段中断、SIGKILL 后 DB/链一致性）
- 幂等/增量恢复：`test_import_recovery.py`（重设计后 9 例，幂等/增量/修改识别/vault 内移动各有代表）
- 手动破坏 vault 后 verify 失败、events 表篡改检测：ai-user-testing §C（AI-VERIFY-002 / AI-DB-001），E2E 对应 `test_verify.py` 损坏检测族 + `TestDbVerifyChain`

---

## 9. 待拍板事项（OPEN）

以下各项影响"判据锁行为还是先修代码"，**由维护者核对本文档时决策**：

| 编号 | 问题 | 选项 |
|------|------|------|
| OPEN-1 | ~~BUG-1：哈希 IO 错误误计 duplicate~~ **已决策执行（2026-08-05）**：选 A——修代码（`HashResult.hash_error` 结构化字段替代字符串前缀匹配），+1 回归单测 `batch_insert_classifies_hash_error_as_failed_not_duplicate` | — |
| OPEN-2 | BUG-2：update 绕过事件溯源写协议 | A. 修代码走 append_event；B. 接受现状并更新 database-schema.md |
| OPEN-3 | ~~半成品/孤儿文件不清理 + 重跑改名重复制~~ **已决策执行（2026-08-09）**：选 B 的 staging 变体——import 改走 `.svault/staging/import/` 原子提交（tmp→fsync→hash→入库→rename），半成品不进入最终路径，对账清除残留；G1 精确化为"不删除**用户**文件"。+7 单测（staging 对账 ×5、import 端到端 ×2） | — |
| OPEN-4 | manifest 非原子 + session_id 秒冲突 | A. 接受；B. tmp+rename + 纳秒/UUID session_id |
| OPEN-5 | PARKED §6 暂缓设计（recover / health pre-flight / sync_journal） | A. 维持暂缓；B. 立项实现（recover 涉及 .svault/corrupted/ 与逐文件确认交互，必须走 Event/Interactor，不得复活 reporter trait） |
| OPEN-6 | 部分失败退出码不一致：import/sync/clone=0，verify=1 | A. 接受（verify 是审计命令，语义不同）；B. 统一 |
| OPEN-7 | DRIFT-1/3/4 原文档回改 | A. 本次一并回改；B. 仅保留本文档权威声明，原文档加指向 |

---

*本文档由三轮代码事实核查（import 管线 / verify-recheck-update / 传输-sync-信号）
生成，全部 [VERIFIED] 结论均有 `文件:行号` 证据。核对通过后，故障注入测试
按 §8 执行。*

*2026-08-09 更新：import 引入 staging 原子提交（G7 重写、G1 精确化、§3.1/§5
中断语义更新、[OPEN-3] 闭环）。sync/clone 的中断残留语义不变。*
