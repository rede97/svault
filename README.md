# Svault

<p align="center">
  <img src="https://raw.githubusercontent.com/rede97/svault/main/docs/assets/logo.svg" alt="Svault Logo" width="120">
</p>

<p align="center">
  <b>Your memories, replicated forever.</b><br>
  <i>Built entirely by AI. Verified by reality.</i>
</p>

<p align="center">
  <a href="https://github.com/rede97/svault/actions/workflows/ci.yml">
    <img src="https://github.com/rede97/svault/actions/workflows/ci.yml/badge.svg" alt="CI">
  </a>
  <a href="https://github.com/rede97/svault/actions/workflows/release.yml">
    <img src="https://github.com/rede97/svault/actions/workflows/release.yml/badge.svg" alt="Release">
  </a>
  <a href="https://crates.io/crates/svault">
    <img src="https://img.shields.io/crates/v/svault" alt="Crates.io">
  </a>
  <a href="https://crates.io/crates/svault-core">
    <img src="https://img.shields.io/crates/v/svault-core" alt="svault-core">
  </a>
  <a href="https://opensource.org/licenses/MIT">
    <img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License: MIT">
  </a>
</p>

---

## 🎯 What is Svault?

**Svault** = **S**valbard + **Vault**

Svault is a **content-addressed multimedia archive** designed for photographers, videographers, and anyone who values their digital memories. Written in Rust for performance and reliability, it safely backs up photos and videos across multiple drives with **bit-for-bit deduplication**, manages composite media formats (Live Photos, RAW+JPEG), and provides **tamper-evident integrity verification** — all from a fast, intuitive command line interface.

### The AI Experiment

Svault is also a **public benchmark** for AI software engineering. Every line of code is written by AI, demonstrating that artificial intelligence can design, implement, and maintain production-grade systems. The repository timeline documents this journey from requirements to working software.

---

## 🤔 Why Svault?

**Problem**: Traditional backup tools copy everything, wasting space and time. Cloud services compress your photos and lock you into subscriptions.

**Solution**: Svault uses content-addressing to store each unique file exactly once, preserves original quality, and gives you full control over your archive.

| Feature | Svault | rsync | Cloud Storage |
|---------|--------|-------|---------------|
| Deduplication | ✅ Bit-level | ❌ | ⚠️ Lossy |
| Integrity verification | ✅ Cryptographic | ❌ | ❌ |
| Offline access | ✅ | ✅ | ❌ |
| Privacy | ✅ Local-first | ✅ | ❌ |
| Cost | Free | Free | $$$ |

---

## ✨ Features

### 🔒 **Content-Addressed Storage**
Files are stored by their cryptographic hash (SHA-256), ensuring absolute integrity. Same content = same address, automatic deduplication.

### 🚀 **High-Performance Pipeline**
Three-tier hashing for speed and accuracy:
1. **CRC32C** — Fast fingerprinting (hardware-accelerated)
2. **XXH3-128** — Collision-resistant identification
3. **SHA-256** — Cryptographic content identity

### 🛡️ **Tamper-Evident Database**
Event-sourced SQLite with SHA-256 hash chain. Every state change is recorded and verifiable.

### 🔗 **Smart Copy Strategies**
Automatic fallback chain: `reflink` (CoW) → `hardlink` → `copy`. Preserves space while ensuring reliability.

### 📝 **Safety-First Design**
- **No delete command** — Review manifests and delete sources manually
- **Process locking** — Prevents concurrent modifications
- **Vault self-protection** — Automatically skips vault metadata during import

### 📊 **Rich CLI Experience**
```bash
svault status      # Beautiful vault overview
svault history     # Browse import sessions
svault verify      # Integrity verification with progress
svault recheck     # Compare source against vault
```

### 🔧 **Unix Philosophy**
Composable pipeline architecture for scripting and automation:
```bash
# Scan → Filter → Import workflow
svault scan /photos --show-dup | rg 'new:.*\.CR3$' | svault import /photos --files-from -

# Chain with external tools
svault history sessions --output json | jq 'select(.event=="history_sessions_item")'
```

---

## 🏎️ Performance

- **10,000 photos** imported in ~45 seconds (NVMe SSD, reflink)
- **CRC32C** at 8 GB/s (hardware-accelerated)
- **Parallel processing** scales with CPU cores
- **Zero-copy** on CoW filesystems (Btrfs, XFS, APFS)

---

## 🚀 Quick Start

### Installation

```bash
# Using cargo
cargo install svault

# Or download prebuilt binary from releases
curl -L https://github.com/rede97/svault/releases/latest/download/svault-$(uname -s)-$(uname -m) -o svault
chmod +x svault
sudo mv svault /usr/local/bin/
```

### Your First Vault

```bash
# 1. Create a vault
cd /mnt/backup/photos
svault init

# 2. Import from a camera SD card
svault import /media/SD_CARD/DCIM/ --target 2024

# 3. Check what was imported
svault status

# 4. Verify everything is intact
svault verify
```

### Daily Workflow

```bash
# After a shoot, import new photos
svault import /mnt/card/DCIM/100CANON/ --target shoots/$(date +%Y-%m-%d)

# Verify the archive is healthy
svault verify --recent 86400  # Files imported in last 24 hours

# Check import history
svault history sessions --limit 5
```

---

## 📖 Command Reference

| Command | Status | Description | Example |
|---------|--------|-------------|---------|
| `init` | ✅ Ready | Initialize a new vault | `svault init` |
| `import <source>` | ✅ Ready | Import media from directory | `svault import /path/to/photos` |
| `recheck [source]` | ✅ Ready | Verify import session against manifest | `svault recheck --session <id>` |
| `add <path>` | ✅ Ready | Register files already in vault | `svault add ./existing/photos` |
| `update` | ✅ Ready | Update paths for moved files | `svault update --target ./vault` |
| `verify` | ✅ Ready | Check file integrity | `svault verify --file photo.jpg` |
| `status` | ✅ Ready | Show vault overview | `svault status` |
| `history` | ✅ Ready | Browse import history | `svault history sessions --limit 20` |
| `scan` | ✅ Ready | Scan directory for import pipeline | `svault scan ./photos` |
| `db dump` | ✅ Ready | Dump database contents | `svault db dump --format json` |
| `db verify-chain` | 🧪 Experimental | Verify event-log hash chain integrity | `svault db verify-chain` |
| `sync` | ⛔ Stub | Sync files from another vault | — |
| `clone` | ✅ Ready | Clone a subset to working directory | `svault clone --target /tmp/work --filter-date 2024-05-01..2024-05-31` |

**Status legend:**
- ✅ Ready — Fully implemented and tested
- 🧪 Experimental — Implemented but may have edge cases; API/behavior may change
- ⛔ Stub — Command exists but exits with `todo!()` (not yet implemented)

---

## 🏗️ Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    svault-cli (bin)                     │
│         CLI parsing · Output formatting · Progress      │
└────────────────────┬────────────────────────────────────┘
                     │
┌────────────────────▼────────────────────────────────────┐
│                   svault-core (lib)                     │
│  ┌──────────────────────────────────────────────────┐   │
│  │  Config  (TOML-based, per-vault settings)        │   │
│  ├──────────────────────────────────────────────────┤   │
│  │  Hash    CRC32C → XXH3-128 → SHA-256 (lazy)     │   │
│  ├──────────────────────────────────────────────────┤   │
│  │  FS      reflink/hardlink/copy with fallback    │   │
│  ├──────────────────────────────────────────────────┤   │
│  │  Pipeline 5-stage import (scan → crc → lookup   │   │
│  │            → hash → insert)                     │   │
│  ├──────────────────────────────────────────────────┤   │
│  │  DB      Event-sourced SQLite with hash chain   │   │
│  └──────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
```

### Import Pipeline Flow

```
Source → [A] Scan → [B] CRC32C → Lookup → [D] Hash → [E] Insert → Vault
          ↓          ↓            ↓         ↓          ↓
        Files    Fingerprint   Dedup     Identity    DB+Events
         (FS)    (parallel)    (cache)   (parallel)  (atomic)
```

**Stage A (Scan)**: Parallel directory traversal via local filesystem
**Stage B (CRC32C)**: Hardware-accelerated fingerprinting for fast cache lookup
**Lookup**: Check CRC cache to skip known duplicates (early exit)
**Stage D (Hash)**: Compute strong hash (XXH3-128/SHA-256) for new files only
**Stage E (Insert)**: Atomic DB insertion with event logging and manifest generation

**Key Design Decisions:**

- **Append-only event log** — All state changes recorded as events, enabling full history and tamper detection
- **Lazy SHA-256** — Computed only when needed for collision resolution
- **Pipeline architecture** — Shared 5-stage pipeline used by `import` and `add` commands
- **Filesystem module** — Local filesystem primitives with transfer fallback chain
- **Early deduplication** — CRC32C cache eliminates 90%+ of duplicate work before hashing

---

## ⚙️ Configuration

On `svault init`, a `svault.toml` is created at the vault root:

```toml
[global]
hash = "xxh3_128"           # fast | secure (SHA-256)
sync_strategy = "reflink"   # reflink | hardlink | copy

[import]
store_exif = false
rename_template = "$filename.$n.$ext"  # Conflict resolution
path_template = "$year/$mon-$day/$device"
allowed_extensions = [
    "jpg", "jpeg", "heic", "heif",
    "dng", "cr2", "cr3", "nef", "arw", "raf",
    "mov", "mp4", "mts"
]
```

### Conflict Resolution

When importing files with the same name but different content:
```
DSC0001.jpg       (first file)
DSC0001.1.jpg     (second file - different hash)
DSC0001.2.jpg     (third file)
```

---

## 🧪 Testing

```bash
# Run unit tests
cargo test

# Run E2E tests (uses RAMDisk for isolation)
cd tests/e2e && bash run.sh

# Run with verbose output
cd tests/e2e && bash run.sh --verbose

# Test specific file system
bash run.sh --test-dir /mnt/btrfs
```

**Test Coverage:** 198 E2E tests passing, 117 unit tests.

---

## 🗺️ Roadmap

| Phase | Status | Deliverables |
|-------|--------|--------------|
| ✅ Phase 1 | Complete | CLI skeleton, event-sourced DB, local FS module, `init` |
| ✅ Phase 2 | Complete | `import`, 5-stage pipeline, manifest output |
| 🚧 Phase 3 | In Progress | `sync` / `clone` / `recover` (designed, implementing) |
| ✅ Phase 4 | Complete | `verify`, `history`, `recheck`, background-hash |
| 🚧 Phase 5 | In Progress | Composite media (Live Photo, RAW+JPEG) |
| 📋 Phase 6 | Planned | Flutter GUI (photo grid, album management, Lightroom-style import), thumbnail cache |
| 📋 Phase 7 | Planned | MTP device import (`svault-mtp`), only needed for GUI import wizard |
| 📋 Later | Planned | Perceptual dedup, device auto-detection, remote sync transport, HDR preview |

### Phase 6: Flutter Desktop GUI

Svault's GUI will be built with **Flutter** and maintained as a **separate repository** (`svault-gui`). This keeps the Rust workspace and the Flutter project independent: svault-core is published as a library crate; the GUI consumes it via `flutter_rust_bridge`.

#### Why Flutter

The GUI is an **image-intensive desktop application** — virtual-scrolled photo grids, thumbnail previews, album drag-and-drop, RAW-to-preview transitions. Three properties matter most for this workload:

- **AOT-compiled rendering**: Flutter's Impeller rasterises via Metal (macOS) / Vulkan (Linux) / D3D (Windows) directly; no DOM, no JS bridge, no browser compositor in the way.
- **Zero-copy image display**: `Image.file(path)` loads from disk without crossing an FFI boundary. svault's thumbnails sit in `.svault/thumbnails/` — Flutter reads them natively.
- **Hot reload**: Frontend iteration is sub-second. Unlike Tauri, which re-links the Rust binary on every frontend change, Flutter only recompiles the changed Dart code.

#### Architecture

```
┌────── svault (Rust workspace) ──────────────────┐
│  svault-core  ─── crate.io  ───  svault-gui/rust │
│  svault-cli                          flutter_rust_bridge │
│  svault-mtp                          ┌──────────────────┤
└───────────────────────────────────────┤  Flutter (Dart)  │
                                        │  · PhotoGrid     │
                                        │  · AlbumManager  │
                                        │  · ImportWizard  │
                                        │  · VerifyPanel   │
                                        └──────────────────┘
```

**Data path** (critical for performance):

- **Metadata** crosses the FFI: file lists, hashes, sizes, dates — lightweight structs via `flutter_rust_bridge`.
- **Images never cross the FFI**: Flutter loads photos and thumbnails directly from disk with `Image.file()`.
- **Progress events** flow through the existing `Reporter` trait: svault-core emits typed reporter calls, the bridge translates them to Dart streams for the UI.

#### Thumbnail cache

Three-tier, mirroring Lightroom's approach:

| Tier | Size | Purpose | Generated | TTL |
|------|------|---------|-----------|-----|
| Small | 256 px | Photo grid browsing | At import | Permanent |
| Medium | 2048 px | Single-photo view | On-demand | 30 days |
| Large | Full resolution | 100% zoom | On-demand | 30 days |

Thumbnails are stored in `.svault/thumbnails/` as webp files, indexed by `cache.db` (separate from the event-sourced `vault.db`). They are pure caches — deletable and rebuildable without affecting archive integrity.

#### RAW decoding pipeline

RAW preview requires native code. Svault embeds **LibRaw** (C) and exposes it through svault-core as a Rust function. Flutter never touches a RAW file — it only receives the rendered result.

```
RAW → LibRaw (native, C) → 16-bit linear RGB → color space + gamma → webp
                                                         ↑
                                                  all in svault-core (Rust)
          │
          └── Flutter: Image.file(thumbnails/small/abc.webp)
```

Key properties:

- **RAW decoding is done in svault-core**, not in Dart. The full C/Rust toolchain handles demosaicing, white balance, and color profiles.
- **Zero-copy FFI**: `flutter_rust_bridge` passes structs and `Vec<u8>` without serialization overhead. Thumbnail bytes, when passed, cross the boundary as raw pointers.
- **Native libraries as dependencies**: Flutter on desktop can link against system-installed or bundled native libraries (libraw, libheif, ffmpeg/libavcodec) via the Rust crate's build script. This is a supported and common pattern — Dart FFI calls Rust, Rust calls C.

Supported RAW formats:

| Format | Cameras |
|--------|---------|
| DNG | Universal (Pentax, Leica, DJI, Apple ProRAW) |
| CR2 / CR3 | Canon |
| NEF | Nikon |
| ARW | Sony |
| RAF | Fujifilm |
| RW2 | Panasonic |
| ORF | Olympus / OM System |

#### Why not Tauri (WebView)

- Photo grids require virtual scrolling at 60 fps; WebView compositing adds 10-15ms per frame.
- `Image.file()` vs `<img>` + browser decode: native image decoding in Impeller is measurably faster for large RAW-to-preview transitions.
- Every Tauri frontend edit re-embeds assets into the Rust binary; Flutter hot reload is sub-second.

---

## 🤝 Contributing

We welcome contributions! Please see [AGENTS.md](./AGENTS.md) for development guidelines and architecture notes.

### Development Setup

```bash
git clone https://github.com/rede97/svault
cd svault
cargo build --release

# Run tests in RAMDisk (recommended)
cd tests/e2e && bash run.sh --verbose
```

---

## 📄 License

MIT — See [LICENSE](./LICENSE) for details.

---

<p align="center">
  <i>Built with ❤️ by AI, for humans.</i><br>
  <i>Every memory matters. Back them up.</i>
</p>
