# CLI 参考文档

> 本文档描述 **当前实现** 的命令行接口（与代码同步维护）。
> 架构与功能分级见 [ARCHITECTURE.md](./ARCHITECTURE.md)。

---

## 设计原则

- **幂等性**：重复执行同一命令不会产生重复数据（三层哈希去重）
- **机器可读**：`--output json` 输出逐行 JSON 事件流（schema 见 `svault-core/src/event.rs`）
- **安全优先**：无任何删除文件的命令；写操作默认需要交互确认或 `--yes`
- **进度可观测**：所有长耗时操作通过统一事件流报告进度

---

## 全局选项

所有命令均支持：

| 选项 | 说明 |
|------|------|
| `--output <format>` | 输出格式：`human`（默认）/ `json` |
| `--dry-run` | 预览操作，不执行任何写入 |
| `--yes` | 跳过交互确认 |
| `--quiet` | 抑制非错误输出 |
| `--threads <n>` | Rayon 工作线程数（0 = 默认） |

> `--output json` 需要 `--yes`（JSON 模式不交互，避免人类提示污染事件流）。

---

## 退出码

| 退出码 | 含义 |
|--------|------|
| `0` | 成功 |
| `1` | 失败（错误信息输出到 stderr） |

---

## JSON 事件流

`--output json` 时 stdout 为逐行 JSON 事件（每行一个完整对象）：

```jsonl
{"event":"phase_started","phase":"scan","total":null,"context":{"source":"/mnt/card"}}
{"event":"scan_item","path":"/mnt/card/IMG_001.CR3","size":52428800,"mtime_ms":1710518400000,"status":"new","error":null}
{"event":"preflight","source":"/mnt/card","total":245,"new":142,"duplicate":103,"moved":0,"failed":0}
{"event":"phase_finished","phase":"scan"}
{"event":"copy_started","src":"/mnt/card/IMG_001.CR3","dst":"/vault/2024/03-15/Canon/IMG_001.CR3","bytes":52428800}
{"event":"summary","kind":"import","total":245,"imported":142,"duplicate":103,"failed":0,"manifest_path":"/vault/.svault/sessions/import/20240315T143000-a1b2c/manifest.json","all_cache_hit":false}
```

**约定**：每个操作一定以 `{"event":"summary","kind":...}` 事件收尾，
消费者可以把它当作流结束标记。

---

## 命令列表

### `svault init`

在当前目录初始化一个新 vault（创建 `.svault/vault.db` 与 `svault.toml`）。

```
svault init
```

---

### `svault import`

从源目录导入媒体文件到归档。

```
svault import <source> [options]
```

| 选项 | 说明 |
|------|------|
| `<source>` | 源目录（必填；不得位于 vault 内，vault 内文件用 `add`）。`-` 表示从 stdin 读文件列表（配合 `--files-from`） |
| `--files-from <path>` | 从文件读取要导入的路径列表（一行一个），跳过完整扫描 |
| `--target <path>` | vault 子目录；从此路径向上发现 vault root（默认当前目录） |
| `--strategy <list>` | 传输策略：`reflink` / `hardlink` / `copy`，可逗号组合（默认 `reflink`；`copy` 始终兜底） |
| `--force` | 即使确认重复也强制导入（同时计算 SHA-256 做确定身份） |
| `--full-id` | 计算 SHA-256 作为确定身份（更强去重保证，更慢） |
| `--show-dup` | 在扫描输出中显示被跳过的重复文件 |

**会话日志：** 每次导入在 `.svault/sessions/import/<ts-id>/` 写
`plan.json`（复制前意图）与 `manifest.json`（结果清单：源路径、归档路径
与哈希，供 `recheck` 使用）；复制中的暂存文件在同目录 `staging/` 子树，
成功入库后搬到最终路径。中断遗留的会话目录由下次 import 报告，
svault 不删除，用户审阅后手动处理。

**全部命中缓存时：** 输出提示并以 `all_cache_hit: true` 的 summary 退出。

---

### `svault add`

注册已经物理存在于 vault 目录内的文件，不移动数据。

```
svault add <path>
```

| 选项 | 说明 |
|------|------|
| `<path>` | vault 内的目录路径（必填） |

若发现文件疑似 vault 内部移动（内容已在库但路径失效），
会提示改用 `svault update`。

---

### `svault update`

扫描归档目录，找回被用户在 Svault 外部移动或重命名的文件，更新数据库路径。
找不到的文件标记为 `missing`（Svault 永不删除文件）。

```
svault update [--target <path>] [--dry-run] [--yes]
```

| 选项 | 说明 |
|------|------|
| `--target <path>` | 扫描根目录（默认为当前目录，按 vault 发现规则） |

**流程：**
1. 扫描目标目录下所有文件，计算哈希（XXH3-128，必要时 SHA-256 确认）
2. 与数据库中路径失效的记录匹配
3. 确认后写入 `file.path_updated` 事件，更新 `files.path`
4. 未匹配的记录标记为 `missing`

---

### `svault verify`

校验归档文件的完整性。

```
svault verify [options]
```

| 选项 | 说明 |
|------|------|
| `--file <path>` | 仅校验指定文件 |
| `--recent <seconds>` | 仅校验最近 N 秒内导入的文件 |
| `--upgrade-links` | 将 hardlink 文件原地升级为独立二进制拷贝 |
| `--background-hash` | 在验证前补齐缺失的 SHA-256 |
| `--background-hash-limit <N>` | `--background-hash` 时最多处理的文件数 |

校验策略：DB 中有 SHA-256 用 SHA-256（确定），否则用 XXH3-128（快速）。
发现 missing / size mismatch / hash mismatch / IO error 时以退出码 1 结束。

---

### `svault recheck`

基于 manifest 同时校验**源文件**和 vault 副本与导入时记录的一致性。
报告写入 `.svault/sessions/recheck/<ts-id>/report.json`。

```
svault recheck [source] [--session <id>] [--target <path>]
```

| 选项 | 说明 |
|------|------|
| `[source]` | 可选源目录，必须与 manifest 记录的 source_root 一致 |
| `--session <id>` | 指定会话（默认最近一次）；支持**唯一前缀匹配**（如 `20260809T1530`），歧义时报错并列出候选 |
| `--target <path>` | vault 子目录（同 import 的发现规则） |

状态分类：`ok` / `source_modified` / `vault_corrupted` / `both_diverged` /
`source_deleted` / `vault_deleted` / `error`。

---

### `svault clone`

把 vault 的文件子集单向导出到普通目录（非 vault），保留 vault 相对路径，
并写出 `svault-clone-manifest.json`。

```
svault clone --target <dir> [options]
```

| 选项 | 说明 |
|------|------|
| `--target <dir>` | 导出目标目录（必填；不得位于 vault 内） |
| `--filter-date <range>` | 按 mtime 过滤，如 `2024-03-01..2024-03-31` |
| `--strategy <list>` | 传输策略（同 import） |

导出完成后在目标目录写出 `svault-clone-manifest.json`；源 vault 不被修改。

---

### `svault sync`

从另一个 vault 复制本 vault 缺失的文件（Beyond Compare 风格）。
比对基于两侧数据库记录的哈希（SHA-256 优先，XXH3-128 兜底），不做全量重新哈希。
源 vault 以只读方式打开，永不被修改；仅存在于本 vault 的文件只会被报告，永不被删除。

```
svault sync <source_vault> [options]
```

| 选项 | 说明 |
|------|------|
| `<source_vault>` | 源 vault 根目录（必须包含 `.svault/vault.db`，必填） |
| `--strategy <list>` | 传输策略（同 import） |
| `--verify <scope>` | 同步后校验范围：`none` / `norm`（仅本次新增，默认）/ `full`（全库） |

**比对分类：**

| 分类 | 含义 | 行为 |
|------|------|------|
| Identical | 两侧 hash 与路径均相同 | 跳过 |
| To copy | 仅源 vault 有 | 复制并入库（会话日志 `sessions/sync/<ts-id>/plan.json` + `manifest.json`） |
| Only local | 仅本 vault 有 | 仅报告（永不删除） |
| Moved | hash 相同但路径不同 | 仅报告（不改路径） |
| Conflict | 路径相同但 hash 不同 | 跳过复制，保留本地，报告 |

---

### `svault album`

管理相册——vault 文件的命名集合，支持多级路径与成员级评级。
相册只记录成员关系（指向 DB 文件记录），不复制、不移动、不删除任何文件。

```
svault album create <path>              # 创建（父级自动创建）：album create 挪威旅行/特罗姆瑟
svault album list                       # 树形列出全部相册及成员数
svault album show <path>                # 列出成员及评级
svault album add <album> <path>...      # 添加成员（vault 相对路径或 vault 内绝对路径）
svault album remove <album> <path>...   # 移除成员（不删文件）
svault album rate <album> <0-5> <path>...  # 成员评级（1-5 星，0 清除）
svault album delete <path>              # 删除空相册（有成员/子相册则拒绝）
```

**评级语义**：评级挂在成员关系上——同一张照片在不同相册中评级相互独立；
`files` 表不持有评级。评级前须先 `add` 为成员。

---

### `svault status`

显示归档库的当前状态概览（文件统计、哈希覆盖、近期导入、数据库大小、主要文件类型）。

```
svault status [--output json]
```

---

### `svault db dump`

导出数据库表内容（用于审计、调试和外部工具集成）。

```
svault db dump [tables...] [--format csv|json|sql] [--limit N]
```

默认导出全部表。JSON 格式为 `[{name, columns, row_count, rows}]`。

---

### `svault scan`（仅 debug 构建）

仅执行扫描阶段（Stage A/B），以 pipe 协议输出分类结果，供外部工具过滤后再定向导入。

```
svault scan <source> [--show-dup]
```

**典型管道工作流：**
```bash
svault scan /mnt/card | svault import /mnt/card --files-from -
svault scan /mnt/card --show-dup > report.txt
```

---

## AI Agent 集成示例

```bash
# 1. 预览导入（JSON 事件流，不写入）
svault --output json --dry-run --yes import /mnt/card

# 2. Agent 解析事件流，决策后执行
svault --output json --yes import /mnt/card

# 3. 校验归档完整性
svault --output json verify

# 4. 查询数据库（替代已移除的 history 命令）
svault db dump files --format json | jq '.[0].rows | length'
svault db dump events --format json | jq '.[0].rows[-5:]'
```

---

*此文档与实现同步维护；发现不一致请以代码为准并修正本文档。*
