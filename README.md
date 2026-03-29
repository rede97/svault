# Svault

**Your memories, replicated forever.**

*Built entirely by AI. Verified by reality.*

> Svault = Svalbard + Vault

[![CI](https://github.com/rede97/svault/actions/workflows/ci.yml/badge.svg)](https://github.com/rede97/svault/actions/workflows/ci.yml)
[![Release](https://github.com/rede97/svault/actions/workflows/release.yml/badge.svg)](https://github.com/rede97/svault/actions/workflows/release.yml)

---

## What is Svault?

Svault is an open-source, content-addressed multimedia archival tool written in Rust. It is designed to safely back up photos and videos across multiple drives, deduplicate files by content, and manage composite media formats like Live Photos and RAW+JPEG pairs — all from the command line.

Svault is also an experiment: every line of code is written by AI. The repository serves as a public benchmark for AI's ability to design, implement, and maintain a real production-grade software system.

---

## 这是什么?

Svault 是一个开源的、基于内容寻址的多媒体归档工具，使用 Rust 编写。它帮助你将碎片化的照片和视频安全备份到多块硬盘，对文件进行精确去重，并管理 Live Photo、RAW+JPEG 等复合媒体格式——一切操作均通过命令行完成。

Svault 同时也是一场公开实验：所有代码均由 AI 编写。本仓库作为一份公开的基准测试，用于验证 AI 是否具备设计、实现并长期维护生产级软件系统的完整能力。

---

## Current Status / 当前状态

Svault is in early development. The architecture, database schema, VFS layer, and CLI surface are fully designed. Only `svault init` is implemented end-to-end. All other commands are stubbed and under active development.

当前处于早期开发阶段。架构设计、数据库 schema、VFS 层和 CLI 接口已完整设计。目前仅 `svault init` 可端到端运行，其余命令均为桩实现，正在开发中。

| Command | Status |
|---------|--------|
| `svault init` | Implemented |
| `svault import` | Stub |
| `svault add` | Stub |
| `svault sync` | Stub |
| `svault reconcile` | Stub |
| `svault verify` | Stub |
| `svault status` | Stub |
| `svault history` | Stub |
| `svault clone` | Stub |
| `svault db verify-chain` | Stub |
| `svault db replay` | Stub |

---

## Architecture / 技术架构

The project is a Cargo workspace with two crates:

```
svault/
├── svault-core/   # lib crate — config, db, hash, vfs (no clap dependency)
└── svault-cli/    # bin crate — CLI entry point (clap), produces `svault` binary
```

```
┌─────────────────────────────────────────┐
│         svault-cli (bin)                │
│  clap · JSON output · dry-run ·         │
│  structured exit codes                  │
└──────────────┬──────────────────────────┘
               │
┌──────────────▼──────────────────────────┐
│         svault-core (lib)               │
│  ┌──────────────────────────────────┐   │
│  │  Config (svault.toml / serde)    │   │
│  ├──────────────────────────────────┤   │
│  │  Hash   XXH3-128 · SHA-256 ·     │   │
│  │         CRC32C                   │   │
│  ├──────────────────────────────────┤   │
│  │  VFS    reflink → hardlink →     │   │
│  │         stream copy              │   │
│  ├──────────────────────────────────┤   │
│  │  DB     Event-sourced SQLite     │   │
│  │         (append-only event log + │   │
│  │          materialised views)     │   │
│  └──────────────────────────────────┘   │
└─────────────────────────────────────────┘
```

**Language:** Rust (edition 2024)

**Storage:** Local filesystem — reflink (btrfs/xfs) → hardlink → stream copy, selected automatically

**Database:** Event-sourced SQLite — every state change is appended to an immutable event log with a SHA-256 tamper-evident hash chain

**Hashing pipeline:** CRC32C (fast fingerprint) → XXH3-128 (collision resolution) → SHA-256 (content identity, lazy)

---

## Configuration / 配置

Run `svault init` to create a vault. A `svault.toml` is generated at the vault root:

```toml
[global]
compare_level = "sha256"   # fast | sha256
sync_strategy = "auto"     # auto | reflink | hardlink | copy

[import]
rename_template = "$filename.$n.$ext"   # conflict resolution template
path_template = "$year/$mon-$day/$device"
allowed_extensions = ["jpg", "jpeg", "heic", "dng", "cr2", "cr3",
                      "nef", "arw", "raf", "orf", "rw2", "mov", "mp4", ...]
```

---

## Roadmap / 开发路线

| Phase | Deliverables | Status |
|-------|--------------|--------|
| Phase 1 | CLI skeleton · event-sourced DB · local VFS · exact dedup · `svault init` | In progress |
| Phase 2 | `svault import` · 4-stage fingerprint pipeline · manifest output | Planned |
| Phase 3 | `svault sync` · multi-target replication · `svault reconcile` | Planned |
| Phase 4 | `svault verify` · hash chain audit · `svault status` / `history` | Planned |
| Phase 5 | Composite media (Live Photo, RAW+JPEG) · `svault clone` | Planned |
| Later | Perceptual dedup · TUI · device auto-detection | Planned |

---

## Design Decisions / 设计决策

- **Append-only event log** — All state changes are recorded as events in SQLite. Materialised view tables are derived by replaying those events. This enables full history queries, tamper detection, and database recovery.
- **Lazy SHA-256** — Full-file SHA-256 is computed only when needed for collision resolution. Fast pre-filters (size, CRC32C tail, XXH3-128) eliminate almost all comparisons before reaching the cryptographic hash.
- **Svault never deletes your files** — After import, Svault outputs a manifest (archive path ↔ source path). You verify the result and delete source files yourself. A bug in Svault cannot destroy your originals.
- **OS-managed network shares** — SMB/NFS mounts are treated as ordinary local paths. The kernel handles protocol details; Svault stays focused on content addressing.

---

## Safety-First Workflow / 安全优先的工作流

Svault deliberately has no delete command. After an import, you receive a manifest:

```
# svault-import-manifest-20240315T143000.txt
# Review this file. If the archive looks correct, delete source files manually.

IMPORTED  /archive/2024/03/15/IMG_001.CR3  <--  /mnt/card/DCIM/100CANON/IMG_001.CR3
SKIPPED   (duplicate sha256:a3f…)           <--  /mnt/card/DCIM/100CANON/IMG_002.CR3
```

导入完成后，Svault 输出一份映射清单（归档路径 ↔ 原始路径）。你核查归档结果无误后，自行删除源文件。Svault 不提供任何删除命令——对原始数据的任何破坏性操作，都必须经过人工确认。

---

## The Experiment / 这场实验

Svault is not just a tool — it is a public test. The repository timeline documents AI's ability to go from requirements to architecture to working code, sustain long-term decision consistency across sessions, and handle real-world edge cases.

Milestones worth watching:
- First commit — AI builds from scratch
- First real-world import run
- First bug found and diagnosed
- Architecture comparison across model generations

---

## License

MIT
