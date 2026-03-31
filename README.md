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

Svault is in active development. Core commands `init`, `import`, and `status` are fully implemented. Other commands are stubbed and under development.

当前处于活跃开发阶段。核心命令 `init`、`import` 和 `status` 已完全实现，其余命令正在开发中。

| Command | Status | Description |
|---------|--------|-------------|
| `svault init` | ✅ Implemented | Initialize a new vault |
| `svault import` | ✅ Implemented | Import media from source directory or MTP device |
| `svault status` | ✅ Implemented | Show vault overview and statistics |
| `svault mtp ls` | ✅ Implemented | List MTP device contents |
| `svault mtp tree` | ✅ Implemented | Display MTP device structure as tree |
| `svault add` | 📝 Stub | Register files already in vault |
| `svault sync` | 📝 Stub | Sync with another vault |
| `svault reconcile` | 📝 Stub | Update paths for moved files |
| `svault verify` | 📝 Stub | Verify file integrity |
| `svault history` | 📝 Stub | Query event log |
| `svault clone` | 📝 Stub | Clone subset of vault |
| `svault db dump` | ✅ Implemented | Export database contents for debugging |
| `svault db verify-chain` | 📝 Stub | Verify event hash chain |
| `svault db replay` | 📝 Stub | Replay events to rebuild views |

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

## Features / 功能特性

### MTP Device Support / MTP 设备支持
Import directly from cameras and phones via USB:
```bash
svault mtp ls mtp://1/           # List available storages
svault mtp ls mtp://1/SD/        # List SD card contents
svault mtp tree mtp://1/SD/      # Display as tree
svault import mtp://1/SD/DCIM/   # Import from device
```

### Filename Conflict Resolution / 文件名冲突处理
When multiple files have the same name (e.g., two cameras with `DSC0001.jpg`), Svault automatically renames subsequent files using the configured `rename_template`:
```
DSC0001.jpg       (first file)
DSC0001.1.jpg     (second file - same name, different content)
DSC0001.2.jpg     (third file)
```

This prevents overwrites when multiple photographers with the same camera model import on the same day.

---

## Configuration / 配置

Run `svault init` to create a vault. A `svault.toml` is generated at the vault root:

```toml
[global]
hash = "xxh3_128"
sync_strategy = "auto"

[import]
store_exif = false
rename_template = "$filename.$n.$ext"
path_template = "$year/$mon-$day/$device/$filename"
allowed_extensions = [
    "jpg",
    "jpeg",
    "heic",
    "heif",
    "dng",
    "cr2",
    "cr3",
    "nef",
    "nrw",
    "arw",
    "raf",
    "orf",
    "rw2",
    "pef",
    "raw",
    "mov",
    "mp4",
]
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

## Testing / 测试

Run the integrated test suite:
```bash
python3 tests/run_tests.py
```

This performs end-to-end validation including:
- EXIF date extraction and device detection
- Deduplication (exact duplicates by content hash)
- Filename conflict resolution (same name, different content)
- MTP device compatibility (when device connected)

The test framework generates synthetic fixtures, runs imports in a RAM disk, and validates database state against expected outcomes.

---

## License

MIT
