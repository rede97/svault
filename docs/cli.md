# CLI 参考文档

> 本文档描述 **当前实现** 的命令行接口（与代码同步维护）。
> 架构与功能分级见 [ARCHITECTURE.md](./ARCHITECTURE.md)。
> 每个命令按统一结构组织：**定位 → 使用场景 → 参数 → 示例与预期结果**。

---

## 设计原则

- **幂等**：重复执行同一命令不产生重复数据（XXH3-128 全量哈希去重）
- **机器可读**：`--output json` 输出逐行 JSON 事件流（schema 见 `svault-core/src/event.rs`）
- **安全优先**：无任何删除文件的命令；写操作默认交互确认或 `--yes`
- **可追溯**：每个写操作在 `.svault/sessions/<kind>/<ts-id>/` 留会话日志
  （plan.json 意图 + manifest.json 结果；import 另有 staging/ 暂存）

---

## 全局选项

| 选项 | 说明 |
|------|------|
| `--output <format>` | 输出格式：`human`（默认）/ `json` |
| `--dry-run` | 预览操作，不执行任何写入 |
| `--yes` | 跳过交互确认 |
| `--quiet` | 抑制非错误输出 |
| `--threads <n>` | Rayon 工作线程数（0 = 默认） |

> `--output json` 需要 `--yes`（JSON 模式不交互，避免提示污染事件流）。
> 管道/重定向等非终端环境下确认提示的行为见各命令说明。

## 退出码

| 退出码 | 含义 |
|--------|------|
| `0` | 成功（含部分文件失败——逐文件失败不致命） |
| `1` | 致命失败（锁冲突 / DB 写失败 / plan 或 manifest 写失败等）；`verify` 例外：检出损坏即 1 |

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

## `svault init`

**定位**：在当前目录创建 vault（`.svault/vault.db` + `svault.toml`）。

**使用场景**：首次建立归档库；或在外置硬盘/NAS 挂载点建仓。

**示例与预期结果**：

```bash
$ mkdir ~/photos-vault && cd ~/photos-vault && svault init
✓ Initialized empty svault at /home/user/photos-vault/.svault
```

重复执行报错（exit 1）：`vault already initialized at ...`。

---

## `svault import`

**定位**：把源目录（SD 卡/相机/下载目录）的媒体文件导入归档——复制、
哈希、入库一条龙；中断安全（staging 原子提交）。

**使用场景**：
- 相机卡导入：`svault import /mnt/sdcard --yes`
- 只导入某子目录/某类文件：`--include 'DCIM/**/*.JPG'`、`-r 1`
- 怀疑上次导入有疏漏：`--compare-level mid`（或 `high`）重跑同一源目录
- 管道组合：先用 `scan` 过滤再定向导入（见 `svault scan`）

**参数**：

| 选项 | 说明 |
|------|------|
| `<source>` | 源目录（必填；不得位于 vault 内，vault 内文件用 `add`）。`-` 配合 `--files-from` 从 stdin 读列表 |
| `--files-from <path>` | 从文件读路径列表（一行一个），跳过扫描 |
| `--target <path>` | vault 子目录；从此路径向上发现 vault root（默认当前目录） |
| `--strategy <list>` | 传输策略：`reflink` / `hardlink` / `copy`，逗号组合（默认 `reflink`；`copy` 始终兜底） |
| `-c, --compare-level <fast\|mid\|high>` | 疑似重复的再验证：fast（默认）信任指纹；mid 对命中者做源端 XXH3-128 全量比对；high 优先 SHA-256（DB 无则回退 XXH3）。别名 0/1/2 |
| `-r, --max-depth <N>` | 扫描深度：0 = 不限（默认），1 = 仅源目录一层。`--files-from` 时忽略 |
| `--include <GLOB>` | 只导入匹配项（源相对路径、大小写不敏感、可重复） |
| `--exclude <GLOB>` | 跳过匹配项（优先于 --include）。与扩展名白名单是 AND 关系 |
| `--force` | 确认重复也强制导入（并计算 SHA-256） |
| `--full-id` | 计算 SHA-256 作为确定身份（更强去重，更慢） |
| `--show-dup` | 扫描输出中显示被跳过的重复文件 |

**示例与预期结果**：

```bash
$ svault import /mnt/sdcard --yes
  Found DCIM/IMG_001.CR3 (24.2 MiB)
  ...
Finished: Scanned 245 files from /mnt/sdcard

Pre-flight:
  Likely new:          142  will be imported
  Likely duplicate:    103  already in vault (cache hit)
✓ Copy complete (142/142)
✓ Fingerprint complete (142/142)
✓ Insert complete (142/142)

Import operation completed
  Total files processed: 245
  New files imported:  142
  Manifest: /vault/.svault/sessions/import/20260810T143000-a1b2c/manifest.json
```

- 文件落在 vault 的 `path_template` 目录（默认 `$year/$mon-$day/$device/`），
  入库 = 可见，半成品永不出现在最终路径
- 重跑同一源：指纹短路，秒级完成，`all_cache_hit: true`
- 中断恢复：下次 import 自动补 rename 并报告残留（不删除），详见
  [failure-handling.md](./failure-handling.md) §3.1/§5

---

## `svault add`

**定位**：注册已物理存在于 vault 内的文件（原地跟踪，不复制）。

**使用场景**：
- 手动把一批照片拷进了 vault 目录，纳入管理：`svault add 2026/08-10/`
- 多个目录一次注册：`svault add dir1 dir2` 或 vault 根下 `svault add .`

**参数**：

| 选项 | 说明 |
|------|------|
| `<path>...` | vault 内的目录路径（必填，可多个；每个都必须在 vault 内） |

**与 import 的关键差异**：查重基于**全量 XXH3-128**（不用 64KB 区域指纹）——
vault 内文件可能被原地编辑，指纹盲区会把中段修改误判为重复。
扫描时算的全量哈希直接供入库复用，不多读一遍。

**示例与预期结果**：

```bash
$ svault add 2026/08-10/ --yes
  Found 2026/08-10/IMG_001.jpg (4.1 MiB)
✓ Scan complete (37 files; new 37, duplicate 0, recover 0, moved 0, failed 0)

Pre-flight:
  Likely new:           37  will be imported
Proceed with add? [y/N] y        # --yes 跳过；管道环境自动确认
✓ Insert complete (37/37)
Finished: 37 file(s) added
```

会话日志：`.svault/sessions/add/<ts-id>/`（plan.json + manifest.json）。
疑似 vault 内部移动的文件会提示改用 `svault update`。

---

## `svault update`

**定位**：用户在文件管理器里移动/重命名了 vault 文件后，修正 DB 里的
路径记录；找不到的文件标记 `missing`（**永不删除**）。

**使用场景**：
- 整理了目录结构后：`svault update --yes`
- 先看影响面：`svault update --dry-run`

**参数**：

| 选项 | 说明 |
|------|------|
| `--target <path>` | 扫描根目录（默认当前目录，按 vault 发现规则） |

**示例与预期结果**：

```bash
$ svault update --yes
  Matched: 2026/08-09/Unknown/a.jpg -> moved/a.jpg (fast)
  Missing: 1 file(s) from DB
  Matched: 1 file(s) relocated
  Unmatched: 1 file(s) not found
  Updated: 1 file(s) path corrected
```

匹配依据：XXH3-128 + size 初筛，记录有 SHA-256 时再确认。
确认默认 No；拒绝时连 missing 标记都不写；非终端无 `--yes` 时判 No。
会话日志：`sessions/update/<ts-id>/`（plan.json 修正清单 + manifest.json
逐条结果）；dry-run 或拒绝时不写。

---

## `svault verify`

**定位**：校验 vault 文件的当前完整性（磁盘内容 vs DB 记录哈希）。

**使用场景**：
- 定期巡检：`svault verify`
- 搬迁/挂载异常后抽查：`svault verify --recent 86400`
- 给老记录补 SHA-256：`svault verify --background-hash`

**参数**：

| 选项 | 说明 |
|------|------|
| `--file <path>` | 仅校验指定文件 |
| `--recent <seconds>` | 仅校验最近 N 秒内导入的文件 |
| `--upgrade-links` | 将 hardlink 文件原地升级为独立拷贝（临时文件 + fsync + 原子 rename） |
| `--background-hash` | 校验前补齐缺失的 SHA-256 |
| `--background-hash-limit <N>` | 最多处理 N 个文件 |

**预期结果**：全部一致 exit 0；任何 missing / size mismatch /
hash mismatch / IO error → exit 1。
注意边界：导入时源已损坏的数据 verify 查不出（记录的就是损坏内容的哈希），
该场景用 `import -c mid/high` 重跑（见 failure-handling.md §4）。

---

## `svault clone`

**定位**：把 vault 的文件子集单向导出到普通目录（非 vault），保留相对路径。

**使用场景**：拷一批照片给同事/另一台机器；按时间段导出。

**参数**：

| 选项 | 说明 |
|------|------|
| `--target <dir>` | 导出目标目录（必填；不得位于 vault 内） |
| `--filter-date <range>` | 按 mtime 过滤，如 `2024-03-01..2024-03-31` |
| `--strategy <list>` | 传输策略（同 import） |

**预期结果**：目标目录得到文件副本 + `svault-clone-manifest.json`；
源 vault 不被修改；单文件失败计数后继续，exit 0。

---

## `svault sync`

**定位**：从另一个 vault 复制本 vault 缺失的文件（Beyond Compare 风格，
只比对 DB 记录，不做全量重哈希）。

**使用场景**：两台机器/两块盘的 vault 互相同步；新盘从旧盘补齐。

**参数**：

| 选项 | 说明 |
|------|------|
| `<source_vault>` | 源 vault 根目录（必须含 `.svault/vault.db`） |
| `--strategy <list>` | 传输策略（同 import） |
| `--verify <scope>` | 同步后校验：`none` / `norm`（仅本次新增，默认）/ `full` |

**比对分类**：

| 分类 | 含义 | 行为 |
|------|------|------|
| Identical | 两侧哈希与路径均相同 | 跳过 |
| To copy | 仅源 vault 有 | 复制并入库（写 `sessions/sync/<ts-id>/` 会话日志） |
| Only local | 仅本 vault 有 | 仅报告（永不删除） |
| Moved | 哈希相同但路径不同 | 仅报告（不改路径） |
| Conflict | 路径相同但哈希不同 | 跳过复制，保留本地，报告 |

源 vault 只读打开，永不被修改。

---

## `svault album`

**定位**：相册管理——vault 文件的命名集合（多级路径），成员级独立评级。
只操作 DB 成员关系，不复制/移动/删除任何文件。

**使用场景**：
- 旅行相册：`svault album create 挪威旅行/特罗姆瑟`
- 精选评级：同一照片在"精选集"评 5 星、在"待修图"评 2 星，互不影响
- 通配浏览：`svault album list "挪威旅行/*"`

**子命令**：

```
svault album create <path>                 # 创建（父级自动创建）
svault album list [glob]                   # 树形列出；可选通配过滤（保留父链）
svault album show <path|glob>              # 成员及评级；通配可匹配多个相册
svault album add <album> <path>...         # 添加成员（vault 相对路径或 vault 内绝对路径）
svault album remove <album> <path>...      # 移除成员（不删文件）
svault album rate <album> <0-5> <path>...  # 成员评级（1-5 星，0 清除）
svault album delete <path>                 # 删除空相册（有成员/子相册则拒绝）
```

**规则**：通配大小写不敏感，作用于完整相册路径；`add/remove/rate` 只接受
精确路径（通配批量改成员关系太危险，刻意不支持）；评级前须先 add 为成员；
`files` 表不持有评级（评级挂在成员关系上）。

**示例与预期结果**：

```bash
$ svault album create 挪威旅行/特罗姆瑟
✓ Album created: 挪威旅行/特罗姆瑟
$ svault album add 挪威旅行/特罗姆瑟 2026/08-09/Unknown/a.jpg
✓ 1 file(s) added to album '挪威旅行/特罗姆瑟'
$ svault album rate 挪威旅行/特罗姆瑟 5 2026/08-09/Unknown/a.jpg
✓ 1 file(s) rated in album '挪威旅行/特罗姆瑟'
$ svault album show 挪威旅行/特罗姆瑟
Album: 挪威旅行/特罗姆瑟 (1 member(s))
    5★  2026/08-09/Unknown/a.jpg
```

---

## `svault status`

**定位**：vault 总览 = 统计信息 + 中断会话 + git 风格工作区状态。

**使用场景**：
- 日常查看：`svault status`
- 只看某类变动：`svault status --untracked` / `--moved` / …

**参数**：

| 选项 | 说明 |
|------|------|
| `--untracked` | 只看未入库文件（磁盘有、DB 无记录） |
| `--moved` | 只看移动（DB 路径消失、同内容现于新路径） |
| `--missing` | 只看丢失（DB 有记录、磁盘不存在） |
| `--modified` | 只看修改（路径在库、磁盘大小已变） |

默认输出全部类别；指定标志则只看该类（JSON 输出始终为完整报告）。
检测策略：路径在库且大小一致的文件不重算哈希（git stat-cache 式捷径），
只对未知路径算全量 XXH3-128 区分 moved/untracked；`svault.toml` 与
`.svault/` 永不出现为 untracked。

**示例与预期结果**：

```bash
$ svault status
📦 Svault Vault Status
   /vault
   /vault/.svault/vault.db
📊 Files ...
🧭 Working Tree
  Untracked (not yet added) (1):
    stray.jpg
  Moved (run `svault update` to fix paths) (1):
    2026/08-09/Unknown/a.jpg -> 2026/08-09/Unknown/moved-a.jpg
```

---

## `svault db dump`

**定位**：导出数据库表内容（审计、调试、外部工具集成）。

**示例**：

```bash
svault db dump                          # 全部表，CSV
svault db dump files --format json      # files 表，JSON
svault db dump files album_items --limit 100
```

JSON 格式为 `[{name, columns, row_count, rows}]`。

---

## `svault scan`（仅 debug 构建）

**定位**：只跑扫描阶段（Stage A/B），以 pipe 协议输出分类结果。

**典型管道工作流**：

```bash
svault scan /mnt/card | svault import /mnt/card --files-from -
svault scan /mnt/card --show-dup > report.txt
```

---

*此文档与实现同步维护；发现不一致请以代码为准并修正本文档。*
