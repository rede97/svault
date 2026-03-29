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
| Multi-target backup | Sync to multiple local drives, network mounts (SMB/NFS via OS), or MTP devices simultaneously. |
| Exact dedup | SHA-256 content addressing eliminates exact duplicates safely and deterministically. |
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

**Storage backends:** Local FS (including OS-mounted network shares: SMB/NFS/WebDAV) · MTP (via FUSE or direct)

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
- **Network storage via OS mounts, not built-in protocol clients** — SMB/NFS/WebDAV shares are accessed through the OS mount point (`mount.cifs`, `autofs`, `gvfs`). Svault treats them as ordinary local paths. Implementing SMB session management, authentication, and reconnection logic inside the tool would introduce a large surface area for bugs with no benefit over what the kernel already provides reliably. Known trade-off: copying files within the same SMB share via a mount point routes data through the host machine's memory and network interface — the kernel CIFS client does not automatically issue server-side copy (SMB `FSCTL_SRV_COPYCHUNK`) requests. This is acceptable for v1 where correctness takes priority over throughput. If intra-NAS copy performance becomes a bottleneck in a future release, it can be addressed by an optional SMB-aware transfer path, without changing the default behaviour.
- **No perceptual dedup in core** — Visual similarity matching (pHash/dHash) carries an inherent false-positive risk: two photos with similar composition are not the same photo. Automatically acting on that judgment is unsafe in an archival tool. Svault provides exact dedup (SHA-256) only. Perceptual dedup will be addressed by a separate, purpose-built companion tool with its own review-first workflow, designed around the same safety principles.
- **Svault never deletes your files** — Deletion is irreversible and outside Svault's mission. Instead of deleting source files after import, Svault outputs a mapping file (archive path ↔ source path) for your review. You verify the import succeeded, then delete the source files yourself. This separation of concerns means a bug in Svault can never destroy your originals.

---

## Safety-First Workflow / 安全优先的工作流

Svault deliberately has no delete command. After an import, you receive a manifest:

```
# svault-import-manifest-20240315T143000.txt
# Review this file. If the archive looks correct, delete source files manually.

IMPORTED  /archive/2024/03/15/IMG_001.CR3  <--  /mnt/card/DCIM/100CANON/IMG_001.CR3
IMPORTED  /archive/2024/03/15/IMG_001.JPG  <--  /mnt/card/DCIM/100CANON/IMG_001.JPG
SKIPPED   (duplicate sha256:a3f…)           <--  /mnt/card/DCIM/100CANON/IMG_002.JPG
```

Once you have verified the archive, you decide what to do with the source. Svault will never make that decision for you.

导入完成后，Svault 输出一份映射清单（归档路径 ↔ 原始路径）。你核查归档结果无误后，自行删除 SD 卡或源目录中的文件。Svault 不提供任何删除命令——对原始数据的任何破坏性操作，都必须经过人工确认。

这一设计基于一个简单的原则：**工具的 bug 不应该能够销毁你的记忆。**

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
