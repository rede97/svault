# Import Pipeline Design

> 导入流水线架构设计方案

---

## 设计目标

- 以最小 IO 代价完成文件导入，避免重复传输
- 在文件复制发生之前发现重复，而非之后
- **CRC32C 用于快速反馈**：网络/慢速源仅读 64KB，快速告知用户"可能重复"/"可能新文件"
- **强哈希入库保证**：文件复制到归档后，必须完成 XXH3-128 或 SHA-256 计算才能入库
- 数据库写入不成为并发瓶颈
- 任何情况下不写入脏数据（多层安全网）

---

## 流水线架构（Pipeline Model）

导入过程按阶段拆分为流水线，各阶段通过 channel 连接，独立并发执行：

```
┌─────────────────────────────────────────────────────────┐
│  Stage A: 目录扫描（scan）                               │
│  pipeline::scan::scan_stream 递归遍历源目录               │
│  使用 jwalk（rayon 并行 readdir）                         │
│  扩展名过滤在 walk 内完成                                │
│  产出：ScannedFile 流 → channel → Stage B                │
└──────────────────────┬──────────────────────────────────┘
                       │ channel
┌──────────────────────▼──────────────────────────────────┐
│  Stage B: CRC32C 计算（crc）                             │
│  pipeline::crc::compute_crcs_stream                      │
│  读取头部/尾部 64KB 计算 CRC32C                          │
│  产出：CrcEntry 流 → channel → Stage C                   │
└──────────────────────┬──────────────────────────────────┘
                       │ channel
┌──────────────────────▼──────────────────────────────────┐
│  Stage C: DB 查询去重（lookup）                          │
│  pipeline::lookup::lookup_stream                         │
│  查数据库 CRC32C 缓存 → 标记 likely_new/likely_duplicate │
│  解析 EXIF 关键字段（日期、设备序列号）                  │
│  → 实时反馈用户（"预计传输 X 个文件"）                   │
│  产出：LookupResult 流 → channel → Stage D               │
└──────────────────────┬──────────────────────────────────┘
                       │ channel（仅 likely_new 的文件）
┌──────────────────────▼──────────────────────────────────┐
│  Stage D: 文件复制 + 强哈希（hash）                      │
│  执行实际文件传输（reflink → hardlink → copy fallback）  │
│  复制完成后计算 XXH3-128 / SHA-256                       │
│  查内存 DashMap（本批次去重）+ 查数据库（跨会话去重）    │
│  产出：HashResult 流 → channel → Stage E                 │
└──────────────────────┬──────────────────────────────────┘
                       │ channel
┌──────────────────────▼──────────────────────────────────┐
│  Stage E: 数据库写入（insert）                           │
│  pipeline::insert::insert_batch                          │
│  批量入库（500 条/批），写入 files 表 + events 表        │
│  生成导入清单（manifest）                                │
│  入库必须有强哈希（xxh3_128 或 sha256）                  │
└─────────────────────────────────────────────────────────┘
```

### 阶段分离的原因

| 阶段 | 执行位置 | 特点 | 目的 |
|------|----------|------|------|
| Stage B (CRC) | 源端 | 只读 64KB，快速 | 快速给用户反馈，减少不必要传输 |
| Stage C (Lookup) | 任意 | 纯内存/DB 操作 | 在复制前判定重复，避免复制 |
| Stage D (Hash) | 归档端 | 本地高速 IO | 强哈希计算代价低，可并发 |

分开可以为各阶段独立调节并发数，并支持早退策略（Stage C 发现全部重复时直接退出）。

### CRC32C 的角色

CRC32C 是**临时的快速过滤器**，不参与最终去重决策，不入库作为文件身份：

```
Stage C Lookup 结果：
  CRC32C 命中缓存 → 标记 likely_duplicate → 跳过传输（节省带宽）
  CRC32C 未命中   → 标记 likely_new       → 进入 Stage D 复制

Stage D 强哈希才是最终裁定：
  likely_duplicate 的文件已跳过传输，不再验证
  likely_new 的文件复制后必须完成强哈希才能入库
```

**全部命中时的早退策略：**

Stage C 完成后若 `likely_new = 0`（所有文件均命中缓存），默认提示用户并退出，不进入传输阶段：

```
All 245 files matched cache (no new files detected).
To verify duplicates, run:
  svault recheck   # 基于 manifest 校验源文件与 vault 副本
```

`svault recheck` 会读取最近一次导入的 manifest，同时校验源文件和 vault 副本的哈希一致性。任何不匹配都会被写入报告，供用户手动核查。

---

## 并发去重：三层安全网

### 问题场景

两个线程独立读取了内容相同的两个文件，各自完成强哈希计算后同时准备写入：

```
线程 A：IMG_001.CR3     → XXH3-128 = abc123 → 准备写入
线程 B：IMG_001_copy.CR3 → XXH3-128 = abc123 → 准备写入
                                    ↑ 竞态：两者都认为自己是新文件
```

### 三层安全网

#### 层 1：内存 DashMap（同批次去重，阻止文件复制）

```
Arc<DashMap<Hash, FileInfo>>  // Hash = XXH3-128 或 SHA-256
```

- Stage D 完成强哈希后，执行原子性 `insert_if_absent`
- 先到的线程 insert 成功 → 标记为 imported
- 后到的线程发现已存在 → 立即标记 duplicate，归档文件待清理
- 使用 `DashMap`（分片锁）而非 `Mutex<HashMap>`，避免单点热点

**作用**：发现同一批次内的重复。

#### 层 2：数据库查询（跨会话去重）

```sql
SELECT id FROM files WHERE xxh3_128 = ? LIMIT 1  -- global.hash = xxh3_128
SELECT id FROM files WHERE sha256 = ? LIMIT 1    -- global.hash = sha256
```

- Stage D 完成强哈希后，在 insert DashMap 之前先查一次数据库
- 命中：历史导入中已存在该文件 → 标记 duplicate，归档文件待清理
- 未命中：继续 insert DashMap 流程
- 此查询为只读，可在 Stage D 线程中直接执行（SQLite WAL 模式支持并发读）

**作用**：发现跨会话的重复（上次导入过的文件）。

#### 层 3：数据库唯一约束（最终兜底，防止脏数据）

```sql
CREATE UNIQUE INDEX idx_files_xxh3_128 ON files(xxh3_128) WHERE xxh3_128 IS NOT NULL;
CREATE UNIQUE INDEX idx_files_sha256   ON files(sha256)   WHERE sha256 IS NOT NULL;
```

- 即使层 1 和层 2 因极端竞态条件漏掉了重复，数据库拒绝插入
- 写入线程捕获 `UNIQUE constraint failed`，将该条记录补标为 duplicate
- **任何情况下不会产生重复的数据库记录**

**作用**：兜底保障，不依赖上层逻辑的正确性。

### 完整去重决策流程

```
Stage C: 计算 SHA-256 完成
    │
    ├─ 查数据库：历史记录存在? ──→ duplicate，跳过复制
    │
    ├─ insert DashMap（原子）：本批次已存在? ──→ duplicate，跳过复制
    │
    └─ 两者均无 → 进入文件复制
                        │
                        └─ 复制完成 → 写入 Stage D channel

Stage D: 批量写入数据库
    └─ UNIQUE 冲突（极端竞态）→ 补标 duplicate，不报错
```

---

## 数据库写入策略

### 为什么批量写入

SQLite 每次事务提交触发 `fsync`（刷盘），逐条写入的性能约为 100-500 条/秒。
批量写入（单事务多条）可达 10,000-50,000 条/秒，差距 100 倍以上。

### 批量提交触发条件（满足任一即提交）

```
- 缓冲区积累达 500 条记录
- 距上次提交超过 2 秒
- 上游所有 channel 关闭（导入结束，强制 flush）
```

### 写入时序

| 时机 | 写入内容 | 说明 |
|------|----------|------|
| Stage B 完成 | `crc32c` | 更新指纹缓存，供下次扫描复用 |
| Stage D 完成 | `xxh3_128` 或 `sha256` | 强哈希入库 |
| 文件复制完成 | `status=imported`, `import_session_id` | 记录最终导入结果 |
| duplicate 判定 | `status=duplicate`, `duplicate_of` | 记录重复关系 |
| 导入开始/结束 | `import_sessions` 行 | session 状态更新 |

所有写入通过单一 Stage D 线程序列化执行，避免 SQLite 写锁竞争。
读操作（层 2 查询）在各 Stage C 线程中直接执行（WAL 模式下并发读安全）。

---

## 并发数参考

| 阶段 | 本地 SSD | 本地 HDD | 网络 SMB/NFS |
|------|----------|----------|-------------|
| Stage B（64KB 指纹） | 8–16 | 4–8 | 16–32 |
| Stage C（SHA-256 + 复制） | 4–8 | 2–4 | 4–8 |

| Stage D（DB 写入） | 1 | 1 | 1 |

- 网络场景 Stage B 并发数更高：IO 延迟大，多线程掩盖等待时间
- HDD 场景 Stage C 并发数更低：顺序读优于并发随机读
- 并发数可配置，默认值根据本地存储类型自动选择

---

## 导入结果清单（Manifest）

导入完成后输出映射清单，供用户核查后手动删除源文件：

```
# svault-import-manifest-20240315T143000.txt
# Review this file. Delete source files manually after verifying the archive.

IMPORTED   /archive/2024/03/15/IMG_001.CR3   <--  /mnt/card/DCIM/IMG_001.CR3
IMPORTED   /archive/2024/03/15/IMG_001.JPG   <--  /mnt/card/DCIM/IMG_001.JPG
DUPLICATE  (sha256:abc123, existing: /archive/2023/12/01/IMG_001.CR3)  <--  /mnt/card/DCIM/IMG_002.CR3
SKIPPED    (cache hit, no change)             <--  /mnt/card/DCIM/IMG_003.JPG
```

Svault 不删除任何源文件。清单是用户与工具之间的契约：工具报告发生了什么，用户决定下一步。

---

---

## 用户交互流程（CLI）

### Reporter 类型系统

CLI 使用 typed reporter traits 实现输出：

| Trait | 阶段 | 输出内容 |
|-------|------|----------|
| `ScanReporter` | Stage A+B+C | 扫描进度、发现的文件、重复检测 |
| `CopyReporter` | Stage D (复制) | 文件传输进度 |
| `HashReporter` | Stage D (哈希) | 强哈希计算进度 |
| `InsertReporter` | Stage E | 数据库写入进度、导入摘要 |

**输出格式：**
- **Human (默认)**: 彩色终端输出，带进度条（`Terminal*` 实现）
- **JSON (`--output=json`)**: 结构化 JSON 事件流（`Json*` 实现）

### Stage A-C：扫描阶段（流式输出）

扫描和比对同时进行，每发现一个新文件立即打印，底部固定进度条实时更新：

```
Found  DCIM/Canon/IMG_001.CR3   18.2 MiB
Found  DCIM/Canon/IMG_002.CR3   17.8 MiB
Duplicate DCIM/Canon/IMG_003.CR3   17.1 MiB            # 仅 --show-dup 时显示
Scanning [==>          ]  50/200  /mnt/sdcard/DCIM/Canon
```

### 扫描完成：写入 Pending 文件 + 用户确认

Stage B 完成后：
1. 将扫描结果写入 `.svault/import/<session-id>.pending`
2. 输出汇总 + 完整新文件列表（不折叠）
3. 提示用户确认（`--yes` 跳过）

```
Scan complete: 142 new  (2.3 GiB),  58 duplicates,  200 total

  DCIM/Canon/IMG_001.CR3    18.2 MiB
  DCIM/Canon/IMG_002.CR3    17.8 MiB
  ...

Import 142 files (2.3 GiB) into vault? [y/N]
```

### Pending 文件格式

`.svault/import/<session-id>.pending`（纯文本，UTF-8，Tab 分隔）：

```
source=/mnt/sdcard
session=20240331T143000
total=200 new=142 duplicate=58
DCIM/Canon/IMG_001.CR3	18874368
DCIM/Canon/IMG_002.CR3	17825792
```

**生命周期：**
- Stage B 完成 → 写入
- 导入完成（Stage E）→ 删除，写入 manifest
- 进程中断 → 残留，下次 `svault import` 检测到后提示续传

### Stage D：复制 + 哈希阶段

文件直接复制到最终路径（由 `path_template` 解析），复制完成后立即计算强哈希：

```
Copying  DCIM/Canon/IMG_001.CR3  →  2024/03-15/Canon EOS R5
Hashing  [===============]  142/142
```

复制和哈希在同一个阶段完成，减少中间状态。传输策略优先级：`reflink` → `hardlink` → `copy`。

### Stage E：数据库写入

批量入库（500 条/批），完成后删除 `.pending`，写入 manifest：

```
Import complete: 142 imported,  58 duplicates,  0 failed
Manifest: .svault/manifests/import-20240331T143000.txt
```

### 原子性与可恢复性

- 复制直接到最终路径，DB 用事务写入，未提交自动回滚（WAL 模式）
- 已复制但未入库的文件可通过 `svault reconcile` 补录
- `.pending` 是唯一的状态持久化，进程中断后可续传

---

*此文档为 Svault 核心设计文档，随实现演进持续更新。*
