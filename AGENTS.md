# Svault 开发指南

> 本文档为 AI 助手和开发者提供项目背景、开发规范和关键决策记录。

---

## 项目简介

**Svault** = Svalbard + Vault

一个开源的、基于内容寻址的多媒体归档工具，使用 Rust 编写。所有代码由 AI 编写，作为验证 AI 能否设计、实现并维护生产级软件的公开实验。

---

## 快速开始

```bash
# 构建
cargo build --release

# 运行测试
cargo test                              # Rust 单元测试
cd tests/e2e && bash run.sh --verbose   # Python E2E 测试

# 初始化 vault
cargo run -p svault -- init

# 导入文件
cargo run -p svault -- import <source-dir>
```

### 构建发布版本

```bash
# 标准发布构建
./scripts/build-release.sh

# CentOS 7 / 旧版 glibc 兼容构建 (使用 cargo-zigbuild)
./scripts/build-release.sh --centos
# 或
./scripts/build-centos.sh

# 构建所有变体
./scripts/build-release.sh --all
```

**CentOS 兼容构建要求：**
- 安装 [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild): `cargo install cargo-zigbuild`
- 安装 [Zig](https://ziglang.org/download/) (用于提供 libc 链接)

---

## 关键文档

| 文档 | 说明 |
|------|------|
| [CLAUDE.md](./CLAUDE.md) | 详细技术架构、设计决策、测试框架说明 |
| [docs/UNIT_TESTS.md](./docs/UNIT_TESTS.md) | **测试跟踪文档** - 记录所有测试状态和待办 |
| [README.md](./README.md) | 用户面向的项目介绍 |

---

## 开发规范

### 代码风格

- 使用 `cargo fmt` 格式化
- 使用 `cargo clippy --all-targets --all-features -- -D warnings` 检查
- 所有公共 API 必须有文档注释 (`///`)

### 提交信息

```
<type>: <subject>

<body>

<footer>
```

类型：
- `feat` - 新功能
- `fix` - 修复
- `docs` - 文档
- `test` - 测试
- `refactor` - 重构

### 测试要求

- 新功能必须伴随单元测试或集成测试
- 更新 [docs/UNIT_TESTS.md](./docs/UNIT_TESTS.md) 添加新测试记录
- Python E2E 测试用于验证端到端场景

#### E2E 测试执行规范

```bash
# 运行所有 E2E 测试（使用 RAMDisk）
cd tests/e2e && bash run.sh

# 运行特定测试（使用 -k 参数）
cd tests/e2e && bash run.sh -k "test_raw"

# 需要 root 的测试（如跨文件系统测试）
sudo bash run.sh -k "test_cross_fs"
```

#### 测试固件规范

- **禁止**使用 `piexif` 等 Python EXIF 库
- **必须使用** `exiftool` 写入 EXIF 数据（确保与真实相机文件一致）
- 示例：
  ```python
  def create_dng_with_exif(path: Path, serial: str = "ABC123", image_id: str = "IMG001"):
      # 先创建基础 DNG 文件
      create_minimal_raw(path)
      # 使用 exiftool 写入 EXIF
      subprocess.run([
          "exiftool", "-overwrite_original",
          f"-BodySerialNumber={serial}",
          f"-ImageUniqueID={image_id}",
          str(path)
      ], check=True)
  ```

### ⚠️ 重要：必须在 RAMDisk 中测试

**永远不要**在项目目录中运行 `svault init` 或 `svault import`！

✅ **正确做法**:
```bash
# 方法 1: E2E 测试框架（推荐）
cd tests/e2e && bash run.sh --verbose

# 方法 2: 手动进入 RAMDisk
bash tests/setup_ramdisk.sh
cd .ramdisk/vault
svault status

# 方法 3: 指定自定义目录（测试特定文件系统）
cd tests/e2e && bash run.sh --test-dir /mnt/ext4    # 在 ext4 上测试
bash run.sh --test-dir /mnt/btrfs --cleanup         # 在 btrfs 上测试并清理
```

❌ **错误做法**:
```bash
cd /home/mxq/Codes/svault
svault init      # 错误！会污染项目目录
```

详见 [docs/UNIT_TESTS.md](./docs/UNIT_TESTS.md) 的 "重要测试规则" 章节。

### E2E 测试目录选项

| 选项 | 说明 | 示例 |
|------|------|------|
| 默认 | 使用 RAMDisk (`/tmp/svault-ramdisk`) | `bash run.sh` |
| `--test-dir PATH` | 使用自定义目录 | `bash run.sh --test-dir /mnt/ext4` |
| `--ramdisk-path PATH` | 自定义 RAMDisk 挂载点 | `bash run.sh --ramdisk-path /tmp/my-ramdisk` |
| `--cleanup` | 测试后清理目录 | `bash run.sh --cleanup` |

**注意**: `--test-dir` 和 `--ramdisk-path` 的区别：
- `--test-dir`: 直接使用现有目录（不挂载 RAMDisk）
- `--ramdisk-path`: 在该路径挂载 tmpfs RAMDisk

---

## 架构提醒

### 模块边界

| Crate | 用途 | 依赖 |
|-------|------|------|
| `svault-core` | 核心逻辑（lib） | 无 clap（cli feature 可选） |
| `svault-cli` | CLI 入口（bin） | 依赖 svault-core + clap |

### CLI 架构 (2026-04-05 更新)

```
svault-cli/src/
├── main.rs          # 入口：解析 CLI，提取全局参数，路由到命令
├── cli.rs           # CLI 定义（clap 结构体）
└── commands/        # 命令实现模块
    ├── mod.rs       # 共享函数 (find_vault_root, format_bytes, 信号处理)
    ├── init.rs      # init 命令
    ├── import.rs    # import 命令 (本地文件系统)
    ├── recheck.rs   # recheck 命令
    ├── add.rs       # add 命令
    ├── sync.rs      # sync 命令 (stub)
    ├── reconcile.rs # reconcile 命令
    ├── verify.rs    # verify 命令
    ├── status.rs    # status 命令
    ├── history.rs   # history 命令
    ├── clone.rs     # clone 命令 (stub)
    └── db.rs        # db 子命令
```

**设计原则：**
- 每个命令独立文件，单一职责
- `main.rs` 只负责 CLI 解析和命令路由
- 全局参数（`output`, `dry_run`, `yes`）在 `main.rs` 提取后传递给命令
- 避免在命令模块中直接引用 `&Cli`（防止 borrow checker 问题）

### 文件系统模块

| 模块 | 用途 |
|------|------|
| `fs.rs` | 本地文件系统扫描与传输（reflink/hardlink/copy fallback） |

### Pipeline 架构 (2026-04-05 新增)

```
svault-core/src/pipeline/
├── mod.rs      # Stage trait 定义
├── scan.rs     # Stage A: 目录扫描 (含 vault 路径过滤)
├── crc.rs      # Stage B: CRC32C 计算
├── lookup.rs   # Stage C: DB 查询重复
├── hash.rs     # Stage D: XXH3/SHA256 哈希
└── insert.rs   # Stage E: 批量 DB 插入
```

**设计目的：**
- `import` 和 `add` 命令共享相同的 5 阶段流水线
- 消除 ~69% 的代码重复
- 便于测试和维护

### 关键设计

- **永不删除用户文件** - Svault 没有 delete 命令
- **事件溯源数据库** - 所有变更记录在 `events` 表
- **三层哈希** - CRC32C → XXH3-128 → SHA-256
- **Vault 发现** - 从 CWD 向上查找 `.svault/vault.db`
- **进程锁保护** - 修改命令自动获取 `<vault>/.svault/lock` 咨询锁
- **Vault 自保护** - 导入扫描时自动跳过源目录下的 `.svault/` 及 vault root 子树
- **Manifest 导入清单** - 每次导入写入 JSON 清单，记录源路径、归档路径和哈希
- **统一 recheck** - `svault recheck` 基于 manifest 同时校验源文件和 vault 副本的一致性

---

## 已知限制

1. **Windows 支持** - 基础功能可用，但 reflink 需要额外实现
2. **内存使用** - 导入大量文件时进度条可能占用较多内存

---

## 更新记录

| 日期 | 更新内容 |
|------|----------|
| 2026-03-31 | 添加 AGENTS.md 和 UNIT_TESTS.md |
| 2026-04-02 | 文件传输策略重构：解耦 transfer strategy；`--force` 替换 `--ignore-duplicate`；导入自保护；E2E 新增至 64 个 |
| 2026-04-04 | `--strategy` 移除 `auto`，默认改为 `reflink`；支持逗号组合（如 `reflink,hardlink`）；`copy` 始终兜底；hardlink 不再出现在默认策略中 |
| 2026-04-04 | 实现 `history` / `background-hash` / `verify --upgrade-links`；支持将 hardlink 原地升级为二进制复制 |
| 2026-04-05 | CLI 重构：拆分 main.rs 为 13 个命令模块；新增 pipeline 架构供 import/add 共享；所有 198 E2E 测试通过 |
