# Svault 开发指南

> 本文档为 AI 助手提供项目背景、开发规范和关键决策记录。

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
cargo test                              # Rust 单元/集成测试
cd e2e_tests && bash run.sh --verbose   # Python E2E 测试

# 初始化 vault
cargo run -p svault-cli -- init

# 导入文件
cargo run -p svault-cli -- import <source-dir>
```

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

### ⚠️ 重要：必须在 RAMDisk 中测试

**永远不要**在项目目录中运行 `svault init` 或 `svault import`！

✅ **正确做法**:
```bash
# 方法 1: E2E 测试框架（推荐）
cd e2e_tests && bash run.sh --verbose

# 方法 2: 手动进入 RAMDisk
bash tests/setup_ramdisk.sh
cd .ramdisk/vault
svault status
```

❌ **错误做法**:
```bash
cd /home/mxq/Codes/svault
svault init      # 错误！会污染项目目录
```

详见 [docs/UNIT_TESTS.md](./docs/UNIT_TESTS.md) 的 "重要测试规则" 章节。

---

## 架构提醒

### 模块边界

| Crate | 用途 | 依赖 |
|-------|------|------|
| `svault-core` | 核心逻辑（lib） | 无 clap（cli feature 可选） |
| `svault-cli` | CLI 入口（bin） | 依赖 svault-core + clap |

### VFS 架构

| 模块 | 用途 |
|------|------|
| `vfs/mod.rs` | `VfsBackend` trait、核心类型 |
| `vfs/transfer.rs` | `TransferEngine`：跨后端文件传输编排 |
| `vfs/system.rs` | `SystemFs`：本地文件系统原子操作 |
| `vfs/mtp.rs` | `MtpFs`：MTP 设备后端（单流） |
| `vfs/manager.rs` | `VfsManager`：URL 路由与发现 |

### 关键设计

- **永不删除用户文件** - Svault 没有 delete 命令
- **事件溯源数据库** - 所有变更记录在 `events` 表
- **三层哈希** - CRC32C → XXH3-128 → SHA-256
- **Vault 发现** - 从 CWD 向上查找 `.svault/vault.db`
- **进程锁保护** - 修改命令自动获取 `<vault>/.svault/lock` 咨询锁
- **Vault 自保护** - 导入扫描时自动跳过源目录下的 `.svault/` 及 vault root 子树

---

## 已知限制

1. **Windows 支持** - 基础功能可用，但 reflink 需要额外实现
2. **内存使用** - 导入大量文件时进度条可能占用较多内存
3. **测试覆盖率** - 单元测试较少，主要依赖 E2E 测试（64 passed）

---

## 更新记录

| 日期 | 更新内容 |
|------|----------|
| 2026-03-31 | 添加 AGENTS.md 和 UNIT_TESTS.md |
| 2026-04-02 | VFS 重构：解耦 transfer strategy；`--force` 替换 `--ignore-duplicate`；导入自保护；E2E 新增至 64 个 |
