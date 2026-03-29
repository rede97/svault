# Svault

**Your memories, replicated forever.**

*Built entirely by AI. Verified by reality.*

> Svault = Svalbard + Vault

[GitHub](https://github.com/rede97/svault)

---

## What is Svault?

Svault is an open-source, distributed multimedia archival tool built on content-addressed storage. It helps you safely back up photos and videos across multiple drives, deduplicate files intelligently (including visually similar images), and manage complex media formats like Live Photos and RAW+JPEG pairs — all from the command line.

Svault is also an experiment: every line of code is written by AI. The repository serves as a public benchmark for AI's ability to design, implement, and maintain a real production-grade software system.

---

## 这是什么?

Svault 是一个开源的、基于内容寻址的分布式多媒体归档工具。它帮助你将碎片化的照片和视频安全备份到多块硬盘，对文件进行精确去重和视觉相似去重，并管理 Live Photo、RAW+JPEG 等复合媒体格式——一切操作均通过命令行完成。

Svault 同时也是一场公开实验：所有代码均由 AI 编写。本仓库作为一份公开的基准测试，用于验证 AI 是否具备设计、实现并长期维护生产级软件系统的完整能力。

---

## Features / 核心功能

| Feature | Description |
|---------|-------------|
| Content-addressed storage | Files are identified by SHA-256, not path. Moves and renames are tracked automatically. |
| Multi-target backup | Sync to multiple local drives, NAS (SMB), MTP devices, or S3-compatible storage simultaneously. |
| Exact & perceptual dedup | SHA-256 for exact duplicates; pHash/dHash/wHash + color histograms for visually similar images. |
| Composite media | Live Photos (HEIC+MOV via ContentIdentifier), RAW+JPEG pairs, depth maps — managed as single logical assets. |
| Event-sourced database | Every operation is appended to an immutable event log (SQLite). Full history replay and tamper detection included. |
| File reconciliation | If you move files outside Svault, `svault reconcile` relocates them by hash and updates the database. |
| Device auto-detection | On Linux, listens for udev events to detect USB drives, SD cards, and MTP cameras on plug-in. |
| AI Agent friendly | All commands support `--output json`, `--dry-run`, `--yes`, and structured exit codes. |

---

## Architecture / 技术架构

```
┌─────────────────────────────────────────┐
│            CLI / TUI (Rust)             │
│  JSON output · dry-run · structured     │
│  exit codes · progress event stream     │
└──────────────┬──────────────────────────┘
               │
┌──────────────▼──────────────────────────┐
│           Core Library (Rust)           │
│  ┌──────────────────────────────────┐   │
│  │  Asset / MediaGroup model        │   │
│  ├──────────────────────────────────┤   │
│  │  Combiner + Format Trait         │   │
│  │  + Plugin system (.so)           │   │
│  ├──────────────────────────────────┤   │
│  │  Transfer Engine + VFS layer     │   │
│  │  reflink→hardlink→srv-copy→      │   │
│  │  rsync→stream→fallback           │   │
│  ├──────────────────────────────────┤   │
│  │  Event-sourced DB (SQLite)       │   │
│  └──────────────────────────────────┘   │
└─────────────────────────────────────────┘
```

**Language:** Rust (static binary, musl target for Linux servers)

**Storage backends:** Local FS · SMB/CIFS · MTP · WebDAV · S3-compatible

**Perceptual hashing pipeline:** dHash (pre-filter) → pHash (DCT, primary) → wHash (anti-crop) → color histogram

---

## Roadmap / 开发路线

| Phase | Deliverables |
|-------|--------------|
| Phase 1 | CLI + event-sourced DB + local VFS + exact dedup |
| Phase 2 | Multi-protocol VFS (SMB/MTP) + device auto-detection + file reconciliation |
| Phase 3 | Perceptual hashing + similarity dedup + BK-tree search |
| Phase 4 | Composite media (Combiner) + subset clone/push |
| Phase 5 | TUI + tree-diff view |
| Later | GUI (Avalonia) + HDR preview + AI classification |

---

## CLI Overview / 命令概览

```bash
# Import from a memory card (dry-run preview)
svault import --source /mnt/card --dry-run --output json

# Sync library to a NAS
svault sync --remote smb://nas/photos

# Relocate files moved outside Svault
svault reconcile --root /mnt/archive

# Find visually similar photos
svault dedup --similarity 15

# Verify archive integrity
svault verify --full

# View operation history
svault history --from 2024-01-01
```

All commands support `--output json`, `--dry-run`, and `--yes` for scripting and AI agent integration.

---

## Design Principles / 设计原则

- **Identity is content, not path** — SHA-256 is the permanent identity of a file; paths are just labels.
- **Event sourcing over snapshots** — Every operation is recorded; any historical state can be replayed.
- **Declare capabilities, don't assume** — Each storage backend declares what it supports (reflink, server-side copy, etc.); the transfer engine picks the optimal strategy.
- **Open for extension, closed for modification** — New formats, protocols, and algorithms are added via Traits and plugins, not by modifying core logic.
- **Cold storage is the filesystem's job** — Compression, error-correction, and tape management are delegated to ZFS/btrfs/LTFS. Svault focuses on identity, sync, and organization.

---

## The Experiment / 这场实验

Svault is not just a tool — it is a public test. The repository timeline documents AI's ability to go from requirements to architecture to working code, sustain long-term decision consistency across sessions, and handle real-world edge cases (power loss, partial transfers, format quirks).

Milestones worth watching:
- First commit — AI builds from scratch
- First real-world import run
- First bug found and diagnosed
- Architecture comparison across model generations

---

## License

MIT
