# Svault 开发指南

> 本文档为 AI 助手和开发者提供项目背景、开发规范和关键决策记录。
>
> **架构规则的唯一事实源是 [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md)。**
> 本文档不写模块清单（会腐烂），只写流程规范。

---

## 项目简介

**Svault** = Svalbard + Vault

一个开源的、基于内容寻址的多媒体归档工具，使用 Rust 编写。所有代码由 AI 编写，作为验证 AI 能否设计、实现并维护生产级软件的公开实验。

---

## 架构铁律（每次会话必读）

**在任何代码变更之前，先读 [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md)。** 摘要：

1. **三 crate 分层**：`svault-core`（纯库）→ `svault-ui`（终端展现）→ `svault-cli`（薄入口）
2. **core 禁止**终端依赖（indicatif/console/rich_rust/clap）和 stdin/stdout 直接读写
3. core 的进度通信只有一条通道：`event::Event` + `EventSink`；交互只有 `event::Interactor`
4. 查询类功能用 Pull 模型（返回 `Serialize` 数据，UI 格式化）；长耗时操作用 Push 模型（事件）
5. 管线只有一套（`pipeline/`），命令是管线的参数化组合（`ops/`）
6. **永不删除用户文件**——任何命令不得提供删除磁盘文件的功能

新增功能前检查 [docs/PARKED.md](./docs/PARKED.md)，不要复活已被移除的功能（除非先更新设计文档）。

---

## 快速开始

```bash
# 构建
cargo build --release

# 运行测试
cargo test --workspace                     # Rust 单元测试
cd tests/e2e && bash run.sh --verbose      # Python E2E 测试（Linux/macOS）

# 初始化 vault（不要在项目目录执行！见下文 RAMDisk 规则）
cargo run -p svault -- init

# 导入文件
cargo run -p svault -- import <source-dir>
```

### 构建发布版本

```bash
./scripts/build-release.sh              # 标准发布构建
./scripts/build-release.sh --centos     # CentOS 7 / 旧版 glibc 兼容
./scripts/build-release.sh --all        # 所有变体
```

---

## 关键文档

| 文档 | 说明 |
|------|------|
| [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md) | **架构规则与分层边界（单一事实源）**，含源码地图（概念→文件:符号） |
| [docs/failure-handling.md](./docs/failure-handling.md) | **故障处理决策矩阵（故障注入测试判据单一事实源）** |
| [docs/PARKED.md](./docs/PARKED.md) | 已移除/暂缓功能登记（防止复活已否决设计） |
| [docs/UNIT_TESTS.md](./docs/UNIT_TESTS.md) | 测试概览统计与变更记录 |
| [docs/cli.md](./docs/cli.md) | CLI 使用文档 |
| [docs/database-schema.md](./docs/database-schema.md) | 数据库结构与磁盘布局 |
| [docs/file-identity.md](./docs/file-identity.md) | 三层哈希与文件身份 |
| [docs/media-binding.md](./docs/media-binding.md) | 媒体绑定实施计划（Live Photo / RAW+JPEG 配对） |
| [tests/e2e/README.md](./tests/e2e/README.md) | E2E 套件宪法（一测试一契约等） |
| [README.md](./README.md) | 用户面向的项目介绍 |

---

## 开发规范

### 代码风格

- `cargo fmt`
- `cargo clippy --workspace --all-targets -- -D warnings`（必须零警告）
- 所有公共 API 必须有文档注释 (`///`)

### 提交信息

```
<type>: <subject>

<body>

<footer>
```

类型：`feat` / `fix` / `docs` / `test` / `refactor`

### 测试要求

- 新功能必须伴随单元测试或集成测试
- core 的用例必须用 `NoopSink` 可在无终端环境测试
- 更新 [docs/UNIT_TESTS.md](./docs/UNIT_TESTS.md) 的概览统计
- Python E2E 测试用于验证端到端场景
- **测试环境操作指南（RAMDisk/exiftool/FUSE 前置/新增用例规范）：
  `.claude/skills/svault-testing/SKILL.md`**

#### E2E 测试执行规范

```bash
cd tests/e2e && bash run.sh                  # 所有测试（RAMDisk）
cd tests/e2e && bash run.sh -k "test_raw"    # 特定测试
sudo bash run.sh -k "test_cross_fs"          # 需要 root 的测试
```

#### 测试固件规范

- **exiftool 是 E2E 强制依赖**——EXIF 固件写入的公认标准实现；`run.sh` 入口硬检查，
  `source_factory` 在请求 EXIF 写入但 exiftool 缺失时直接 `pytest.fail`（不做静默降级）
- **禁止**使用 `piexif` 等 Python EXIF 库
- **必须使用** `exiftool` 写入 EXIF 数据（确保与真实相机文件一致）

### ⚠️ 重要：必须在 RAMDisk 中测试

**永远不要**在项目目录中运行 `svault init` 或 `svault import`！

```bash
# 正确做法
cd tests/e2e && bash run.sh --verbose        # E2E 框架（推荐）
bash tests/setup_ramdisk.sh && cd .ramdisk   # 手动 RAMDisk
bash run.sh --test-dir /mnt/ext4 --cleanup   # 指定文件系统
```

### E2E 测试目录选项

| 选项 | 说明 |
|------|------|
| 默认 | 使用 RAMDisk (`/tmp/svault-ramdisk`) |
| `--test-dir PATH` | 直接使用现有目录（不挂载 RAMDisk） |
| `--ramdisk-path PATH` | 在该路径挂载 tmpfs RAMDisk |
| `--cleanup` | 测试后清理目录 |

---

## 关键设计（不可妥协）

- **永不删除用户文件** — Svault 只清理本次会话自建的 staging 子树；中断残留只报告不删
- **会话日志** — import/sync 的 plan.json + manifest.json 与 recheck 报告在 `.svault/sessions/<kind>/<ts-id>/`，原子写入
- **两层哈希** — XXH3-128（区域指纹 + 全量快速身份）→ SHA-256（确定身份）
- **Vault 发现** — 从 CWD 向上查找 `.svault/vault.db`
- **进程锁保护** — 修改命令自动获取 `<vault>/.svault/lock` 咨询锁
- **Vault 自保护** — 导入扫描时自动跳过 vault root 子树
- **Manifest 导入清单** — 每次导入在会话目录写 JSON 清单

---

## 已知限制

1. **Windows 支持** - 基础功能可用，reflink 需要额外实现
2. **内存使用** - 导入大量文件时进度条可能占用较多内存

---

## 更新记录

| 日期 | 更新内容 |
|------|----------|
| 2026-03-31 | 添加 AGENTS.md 和 UNIT_TESTS.md |
| 2026-04-02 | 文件传输策略重构；`--force` 替换 `--ignore-duplicate`；导入自保护 |
| 2026-04-04 | `--strategy` 默认 `reflink`，`copy` 始终兜底 |
| 2026-04-05 | CLI 拆分命令模块；pipeline 架构供 import/add 共享 |
| 2026-04-12 | Reporting typed reporters 重构（后被证明接口面过大） |
| 2026-04-16 | **架构重构**：三 crate 分层（core/ui/cli）；12 reporter trait → 单一 `Event`+`EventSink`；core 移除 clap/rich_rust；裁减 history/update --delete（见 docs/PARKED.md）；`import/` → `ops/` |
| 2026-04-16 | **Clone/Sync 实现**：Beyond Compare 风格（ARCHITECTURE.md §6）；纯函数 diff 引擎（11 单测）；turso 降至最低优先级 |
| 2026-08-05 | 新增 `docs/failure-handling.md`：故障处理决策矩阵（三轮代码核查生成），作为故障注入测试判据单一事实源；含缺陷登记（BUG-1~4）与待拍板项（OPEN-1~7） |
| 2026-08-05 | 文档清理：删除 CLAUDE.md 陈旧内容/testing-plan/HANDOFF-LINUX/sync-design（recover 设计抢救入 PARKED §6）；FUSE 故障注入落地：INFRA-1/2/4 + P0/P1/P2 共 20 例全绿；修复 BUG-1（哈希错误误分类） |
|2026-08-06|**E2E 套件重设计**（220→167）：删 60 冗余用例、修 dedup 同名类遮蔽死代码、chaos/background-hash 合并、6 弱断言加固、套件宪法入 tests/e2e/README.md；修复 scan 协议转义缺陷（空格文件名管线导入）|
|2026-08-09|**import staging 原子提交**（OPEN-3 闭环）：复制入 `.svault/staging/import/<session>/` → fsync → hash → 整批入库 → commit 后 rename（`pipeline/staging.rs`、`fs::atomic_commit`）；启动对账补 rename/清残留；G1 精确化为"不删除用户文件"；+7 单测 +2 E2E|
|2026-08-09|**事件溯源移除 + 会话日志落地**：维护者决策删 events 表/verify-chain（伪需求，PARKED §A2）；`.svault/sessions/<kind>/<ts-id>/` 统一布局（plan.json 复制前 fail-fast 落盘 + staging/ + manifest.json 原子写，sync 带 plan，recheck 报告迁入）；session_id 时间戳+唯一后缀（BUG-3/4 修复）；reconcile 改只报告不删（G1 最终形态）；复制 ENOSPC 锁定逐文件失败 exit 0|
|2026-08-09|**相册与评级**：`svault album`（多级树 `parent_id` 邻接表 + 成员引用 `files.id` + 成员级独立评级 `album_items.rating`）；`files` 行永不物理删除升级为 FK 硬约束；derivatives/assets 表删除（GUI 派生物独立实现；三级链拍平）；媒体绑定计划入 docs/media-binding.md|
|2026-08-09|**add 会话化 + git 风格 status**：add 支持多目录参数、Preflight+y/N（非终端自动确认）、写 plan+manifest 会话；status 新增工作区状态（untracked/moved/missing/modified，stat-cache 式只对未知路径算哈希）+ 类别过滤标志|
|2026-08-09|**add 改全量哈希查重**：vault 内文件可能原地编辑，指纹盲区会误判 duplicate；add 跳过指纹预筛直接全量 XXH3（lookup_by_hash），Stage D 经 precomputed_hash 复用不重读|
|2026-08-09|**指纹算法统一 XXH3-128**：64KB 区域指纹 CRC32C→XXH3（相机快速扫描路径保留）；files.crc32c→fingerprint BLOB + 首个幂等迁移（ALTER ADD COLUMN）；删 crc32fast/crc32c 依赖；CrcEntry→FingerprintEntry 重命名|
|2026-08-09|**recheck 移除 + compare-level**：recheck 命令删除（PARKED §A4）；`import -c fast/mid/high` 承接源端审计——指纹命中时对源端算 XXH3/SHA-256 与 DB 比对，不符推翻为新文件导入；ManifestManager 死代码清理|
|2026-08-09|**查询体验三件套**：import 支持 `-r/--max-depth`（0 全递归/1 仅一层）与 `--include/--exclude` 通配过滤（globset，大小写不敏感，exclude 优先）；album list/show 支持通配（add/remove/rate 保持精确路径）；status 报告中断会话（无 manifest 的会话目录 + 残留量 + 路径）|
