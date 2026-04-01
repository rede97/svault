# CLI Interface Design

> 命令行接口设计文档

---

## 设计原则

- **幂等性**：所有写操作支持 `--dry-run`，预览变更不执行
- **机器可读**：`--output json` 输出结构化数据，供脚本和 AI Agent 消费
- **安全优先**：无任何删除命令，危险操作需 `--yes` 显式确认
- **标准退出码**：所有命令遵循统一退出码约定
- **进度可观测**：`--progress` 输出逐行 JSON 事件流到 stderr

---

## 全局选项

所有命令均支持以下全局选项：

| 选项 | 说明 |
|------|------|
| `--output <format>` | 输出格式：`human`（默认）/ `json` |
| `--dry-run` | 预览操作，不执行任何写入 |
| `--yes` | 跳过交互确认 |
| `--quiet` | 抑制非错误输出 |
| `--progress` | 输出逐行 JSON 进度事件到 stderr |
| `--config <path>` | 指定配置文件路径（默认 `~/.config/svault/svault.toml`） |
| `--vault <path>` | 指定归档根目录（覆盖配置文件） |

---

## 退出码

| 退出码 | 含义 |
|--------|------|
| `0` | 成功 |
| `1` | 通用错误 |
| `2` | 参数错误 |
| `3` | 源不可达（设备离线、路径不存在） |
| `4` | 目标空间不足 |
| `5` | 冲突需人工介入 |
| `6` | 数据库一致性错误 |

---

## 命令列表

### `svault import`

从源目录或设备导入媒体文件到归档。

```
svault import <source> [options]
```

| 选项 | 简写 | 说明 |
|------|------|------|
| `<source>` | | 源目录或挂载点（必填，位置参数） |
| `--target <path>` | | 目标归档目录（默认使用配置文件中的 vault 路径） |
| `--hash <algo>` | `-H` | 哈希算法：`fast`（XXH3-128，高吞吐，默认）/ `secure`（SHA-256，加密强度）。优先级：CLI > `svault.toml [global].hash` > 内置默认值（`fast`）|
| `--recheck <mode>` | `-R` | 全部命中缓存时对重复项二次验证：`fast`（默认，信任缓存）/ `exif`（EXIF 二进制比对）/ `hash`（全文件哈希）|

**清单文件：**

每次导入自动生成清单到 `<vault_root>/manifests/import-<timestamp>.txt`，记录所有文件的源路径、归档路径和处理结果。

**全部命中缓存时的行为：**

Stage B 完成后若 `likely_new = 0`，默认输出提示并退出：
```
All 245 files matched cache (no new files detected).
To verify duplicates, re-run with:
  -R exif   EXIF binary comparison (recommended)
  -R hash   full-file hash comparison
```

**输出（human）：**
```
Scanning /mnt/card... 245 files found
Importing: [====================] 142/142

Summary:
  Imported:   142
  Duplicate:   23
  Skipped:     80 (cache hit)
  Failed:       0

Manifest: ./manifests/import-20240315T143000.txt
```

**输出（json）：**
```json
{
  "files_found": 245,
  "imported": 142,
  "duplicate": 23,
  "skipped": 80,
  "failed": 0,
  "manifest": "./manifests/import-20240315T143000.txt"
}
```

**进度事件流（--progress，stderr）：**
```jsonl
{"event": "file.discovered", "path": "/mnt/card/DCIM/IMG_001.CR3", "size": 52428800}
{"event": "file.imported", "src": "/mnt/card/DCIM/IMG_001.CR3", "dest": "/archive/2024/03-15/Canon_EOS_R5/IMG_001.cr3"}
{"event": "file.duplicate", "src": "/mnt/card/DCIM/IMG_002.CR3", "duplicate_of": "/archive/2023/12/01/IMG_002.cr3"}
{"event": "progress", "done": 42, "total": 245}
{"event": "finished", "imported": 142, "duplicate": 23, "skipped": 80}
```

---

### `svault sync`

将本地归档同步到远程备份目标（增量，基于哈希）。

```
svault sync --target <path> [options]
```

| 选项 | 说明 |
|------|------|
| `--target <path>` | 目标路径（本地挂载点，必填） |
| `--strategy <strategy>` | 传输策略：`auto`（默认）/ `reflink` / `hardlink` / `copy` |
| `--verify` | 同步后校验目标文件完整性 |

---

### `svault reconcile`

扫描归档目录，找回被用户在 Svault 外部移动或重命名的文件，更新数据库路径。

```
svault reconcile --root <path> [options]
```

| 选项 | 说明 |
|------|------|
| `--root <path>` | 扫描根目录（必填） |

**流程：**
1. 扫描 `--root` 下所有文件，计算 CRC32C 指纹
2. 与数据库中 `status=imported` 但路径失效的记录匹配
3. 输出路径变更清单（dry-run 默认开启，需 `--yes` 执行写入）
4. 写入 `file.path_updated` 事件，更新 `files.path`

---

### `svault verify`

校验归档文件的完整性。

```
svault verify [options]
```

| 选项 | 说明 |
|------|------|
| `--fast` | 使用 XXH3-128 快速校验（不验证 SHA-256） |
| `--full` | 使用 SHA-256 全量校验（默认） |
| `--file <path>` | 仅校验指定文件 |

**输出示例：**
```
Verifying 142 files...
  OK:       140
  Corrupt:    1  → /archive/2024/03-15/IMG_005.cr3
  Missing:    1  → /archive/2024/03-15/IMG_010.cr3

Run `svault reconcile` to locate moved files.
```

---

### `svault status`

显示归档库的当前状态概览。

```
svault status [options]
```

**输出示例：**
```
Vault: /archive
  Files:        142  (imported)
  Duplicates:    23
  Pending SHA:   18  (sha256 not yet computed)
  Groups:        12  (live_photo: 8, raw_jpeg: 4)
  Derivatives:    0
  Events:       312
  DB size:      1.2 MB
```

---

### `svault history`

查看文件或全局操作历史（事件日志）。

```
svault history [options]
```

| 选项 | 说明 |
|------|------|
| `--file <path>` | 查看指定文件的事件历史 |
| `--from <datetime>` | 起始时间过滤 |
| `--to <datetime>` | 结束时间过滤 |
| `--event-type <type>` | 按事件类型过滤 |
| `--limit <n>` | 限制输出条数（默认 50） |

**输出示例：**
```
seq  time                  event                  entity
---  --------------------  ---------------------  ---------------------------
 1   2024-03-15 14:30:01   file.imported          /archive/2024/03-15/IMG_001.cr3
 2   2024-03-15 14:30:01   file.imported          /archive/2024/03-15/IMG_001.jpg
 3   2024-03-15 14:30:02   media_group.created    live_photo (IMG_001)
12   2024-03-16 09:12:44   file.path_updated      IMG_001.cr3 → 2024/canon/IMG_001.cr3
15   2024-03-16 09:15:00   file.sha256_resolved   IMG_001.cr3
```

---

### `svault background-hash`

对 `sha256 = NULL` 的文件执行后台 SHA-256 补全计算。

```
svault background-hash [options]
```

| 选项 | 说明 |
|------|------|
| `--limit <n>` | 本次最多处理的文件数 |
| `--nice` | 以低优先级运行（IO nice），减少对系统的影响 |

通常由系统空闲触发，也可手动执行。完成后写入 `file.sha256_resolved` 事件。

---

### `svault clone`

从归档克隆文件子集到本地工作目录（用于移动办公场景）。

```
svault clone --target <path> [options]
```

| 选项 | 说明 |
|------|------|
| `--target <path>` | 克隆目标目录（必填） |
| `--filter-date <range>` | 按日期过滤，如 `2024-03-01..2024-03-31` |
| `--filter-camera <model>` | 按相机型号过滤 |
| `--filter-group <type>` | 按 MediaGroup 类型过滤 |

---

### `svault db verify-chain`

验证事件日志的哈希链完整性。

```
svault db verify-chain [options]
```

遍历 `events` 表，逐条验证 `self_hash` 和 `prev_hash` 的一致性。任何链断裂都会报告具体的 `seq` 位置。

**输出示例：**
```
Verifying event chain (312 events)...
  Chain OK: seq 1 → 312
```

---

### `svault db replay`

从事件日志重建物化视图（用于数据库损坏恢复）。

```
svault db replay [options]
```

| 选项 | 说明 |
|------|------|
| `--to-seq <n>` | 仅重放到指定事件序号（时间点恢复） |
| `--to-time <datetime>` | 仅重放到指定时间点 |

---

## AI Agent 集成示例

```bash
# 1. 预览导入，获取结构化输出
svault import --source /mnt/card --dry-run --output json

# 2. Agent 解析输出，决策后执行
svault import --source /mnt/card --yes --output json --progress

# 3. 校验归档完整性
svault verify --fast --output json

# 4. 查询最近导入历史
svault history --from 2024-03-15 --output json
```

---

*此文档为 Svault 核心设计文档，随实现演进持续更新。*
