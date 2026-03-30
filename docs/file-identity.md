# File Identity & Comparison Pipeline

> 文件身份识别与比较流水线设计方案

---

## 设计目标

在本地文件系统和网络文件系统（SMB/NFS via mount）两种场景下，以最小的 IO 代价准确判断两个文件是否相同。全文件读取代价在网络场景下不可接受，因此采用分层过滤策略，将昂贵操作推迟到必要时才执行。

---

## 文件身份标识（数据库存储）

### 双哈希策略：XXH3-128 与 SHA-256

文件的永久身份由 **XXH3-128** 或 **SHA-256** 确定，取决于 `global.id_hash` 配置。两种哈希跨设备、跨路径、跨时间永久有效，是内容寻址的基础。

**哈希选择：**
- **XXH3-128**：非密码学哈希，速度极快（30-60 GB/s），碰撞概率 ~2⁻¹²⁸，个人使用足够
- **SHA-256**：密码学哈希，速度较慢（~500 MB/s），碰撞概率 ~2⁻²⁵⁶，适合数据认证场景

**身份解析优先级：**
```
if sha256 IS NOT NULL:
    identity = sha256      # SHA-256 优先，无论配置
else:
    identity = xxh3_128    # 回退到 XXH3-128
```

`size`（文件字节数）作为独立列存储，不参与主键，仅用于比较流水线的 Stage 2 快速过滤。

### 核心表结构

```
files 表
├── id                 INTEGER   内部自增行号（ORM 用）
├── xxh3_128           TEXT      XXH3-128 哈希，唯一索引（import.dedup_hash=xxh3_128 时入库必填）
├── sha256             TEXT      SHA-256 哈希，唯一索引（import.dedup_hash=sha256 时入库必填，或后台补全）
├── size               INTEGER   字节数，普通索引（用于 Stage 2 快速过滤）
├── path               TEXT      当前文件路径（可变，路径不是身份）
├── mtime              INTEGER   最后修改时间戳（缓存失效键之一）
├── crc32c             INTEGER   CRC32C 值（临时指纹，epoch 失效）
└── import_session_id  INTEGER   导入会话 ID（关联 import_sessions 表）
```

**`crc32c` 字段**：纯数值（INTEGER），临时快速指纹。应用版本更新时，通过 `metadata.crc32c_epoch` 全局失效，执行 `UPDATE files SET crc32c = NULL`。

重复文件查找查询：
```sql
-- 使用 id_hash 对应的列查询
SELECT * FROM files WHERE size = ? AND xxh3_128 = ?   -- id_hash = xxh3_128
SELECT * FROM files WHERE size = ? AND sha256 = ?     -- id_hash = sha256 或 SHA-256 优先
```
`size` 先过滤绝大多数候选，哈希精确确认，两列均有唯一索引。

### 哈希缓存失效条件

```
device_id + inode + mtime + size
```
任意一项变化则重新计算所有哈希。CRC32C 额外受 `metadata.crc32c_epoch` 控制——应用升级时递增 epoch，所有 CRC32C 缓存批量失效。

---

## 比较流水线（按 IO 代价从低到高）

```
Stage 1: 扩展名比较
        ↓ 不同 → 肯定不同，终止
        ↓ 相同 → 继续

Stage 2: 文件大小比较（stat() syscall，无需打开文件）
        ↓ 不同 → 肯定不同，终止
        ↓ 相同 → 继续

Stage 3: 前 64KB 快速比较
        ├── CRC32C 校验和比较
        └── 二进制 EXIF 关键字段比较
        ↓ 任一不同 → 肯定不同，终止
        ↓ 全部一致 → 高置信度相同
        ↓ [网络存储默认止步于此]

Stage 4: SHA-256 全文件哈希比较
        ↓ 不同 → 不同，终止
        ↓ 相同 → 密码学意义上的相同
        ↓ [本地存储默认止步于此]

Stage 5: 全文件逐字节二进制比较
        → 最终裁定，100% 确定（审计模式专用）
```

---

## 各阶段详细说明

### Stage 1 — 扩展名比较

- 规范化为小写后比较（`.CR3` → `.cr3`）
- 代价：零 IO，纯内存
- 扩展名不同可立即排除；扩展名相同不代表格式相同（损坏文件等），但可过滤大量明显不同的文件
- 无扩展名文件跳过本阶段

### Stage 2 — 文件大小比较

- 调用 `stat()` 获取精确字节数，无需打开文件
- 代价：单次 syscall，极低
- 大小不同则内容必然不同

### Stage 3 — 快速指纹比较（格式感知，可扩展）

Stage 3 的读取位置和解析逻辑由**格式处理器（Format Handler）**决定，而非硬编码为「读头部 64KB」。每种格式可以声明自己的元数据块位置和提取方式，未注册格式自动 fallback 到默认策略。

#### 格式处理器接口

每个格式处理器实现以下方法：

```
fn fingerprint_regions(file_size: u64) -> Vec<ByteRange>
    → 返回需要读取的字节区间列表（可以是头部、尾部、或多个区间）

fn extract_fingerprint(regions: &[u8]) -> Fingerprint
    → 从读取到的字节中提取 CRC32C 和结构化元数据字段
```

#### 内置格式策略

| 格式 | 读取区域 | 元数据来源 | 说明 |
|------|----------|------------|------|
| JPEG | 头部 64KB | EXIF APP1 segment | EXIF 几乎全在文件头 |
| HEIC / HEIF | 头部 64KB | `ftyp` + `meta` box | ISO BMFF 结构，元数据在头部 |
| CR3 / NEF / ARW (RAW) | 头部 64KB | EXIF IFD | 与 TIFF 结构兼容，头部解析 |
| **PNG** | **尾部 64KB** | **`tEXt` / `iTXt` / `eXIf` chunk** | PNG 元数据 chunk 可出现在文件末尾，优先读尾部 |
| MP4 / MOV | 头部 64KB | `moov` box（若在头部） | 部分文件 `moov` 在尾部，此时降级为仅 CRC32C |
| 未知 / fallback | 头部 64KB | 无结构化解析 | 仅计算 CRC32C，不提取元数据字段 |

#### 3a. CRC32C 校验和

- 对处理器声明的字节区间计算 CRC32C-32（SSE4.2 / ARM CRC32 硬件原生指令，单条指令完成）
- 代价：最多 64KB 读取，约 0.1ms（本地）/ 1–5ms（网络）；64KB ÷ 20 GB/s ≈ 3µs，完全被 IO 延迟淹没
- CRC32C 不同则指纹区域内容必然不同
- **为什么用 CRC32C 而非 XXH3**：XXH3 没有 32 位变体，最小输出为 64 位；强行截断至 32 位会使碰撞概率升至 1/2³²，在生日悖论下约 5 万文件时碰撞概率 >50%，不适合作过滤器。CRC32C 在过滤器场景下碰撞由后续阶段兜底，32 位完全够用，且硬件代价更低
- **XXH3 的适用场景**：大文件的快速完整性校验（Stage 4 预筛、`svault verify --fast`），此时使用 **XXH3-128**，吞吐量 30–60 GB/s，可触达内存带宽上限，且 128 位输出碰撞概率可忽略

#### 3b. 结构化元数据字段比较

对能够解析结构化元数据的格式，提取以下字段并逐字节比较：

| 字段 | 说明 | 误判风险 |
|------|------|----------|
| `DateTimeOriginal` | 拍摄时间（秒级） | 低 |
| `SubSecTimeOriginal` | 拍摄时间（毫秒级） | 极低 |
| `CameraSerialNumber` | 相机序列号 | 极低 |
| `ImageUniqueID` | 相机生成的唯一 ID | 极低 |
| `OriginalRawFileName` | 原始文件名（部分相机写入） | 低 |

以上字段组合在实际摄影场景下误判率可忽略不计。

**Fallback 规则（优先级从高到低）：**
1. 格式处理器提供完整指纹策略 → 使用处理器声明的区域和字段
2. 格式已知但元数据解析失败 → 仅依赖 CRC32C，记录警告
3. 格式未知 / 无注册处理器 → 读头部 64KB，仅依赖 CRC32C

### Stage 4 — SHA-256 全文件哈希

- 读取完整文件，计算 SHA-256
- 代价：全文件 IO（网络场景代价高，仅对 Stage 3 无法排除的候选执行）
- 结果写入数据库缓存，二次扫描命中缓存则代价为零
- SHA-256 相同在密码学上视为内容相同（碰撞概率 ~2⁻¹²⁸）

### Stage 5 — 全文件逐字节二进制比较

- 仅在审计模式（`--verify-full`）下启用
- 代价：两倍全文件 IO
- 唯一能提供 100% 确定性的方式，用于安全敏感场景

---

## 网络文件系统优化策略

VFS 后端通过 `capabilities()` 声明存储类型，核心库据此选择流水线终止阶段：

| 存储类型 | 默认终止阶段 | 说明 |
|----------|-------------|------|
| 本地 SSD / NVMe | Stage 4 (SHA-256) | IO 代价低，直接全哈希 |
| 本地 HDD | Stage 4 (SHA-256) | 顺序读效率高，可接受 |
| 网络挂载 (SMB/NFS) | Stage 3 (64KB) | 减少网络 IO，止步于高置信度 |
| MTP 设备 | Stage 3 (64KB) | MTP 无随机访问，减少传输次数 |

用户可通过 `--compare-level` 覆盖默认策略：

```bash
# 强制 SHA-256（网络场景也不例外）
svault import --source /mnt/nas/photos --compare-level sha256

# 仅快速比较（Stage 1–3）
svault import --source /mnt/nas/photos --compare-level fast
```

---

## 决策树总结

```
文件 A vs 文件 B
│
├─ 扩展名不同? ──→ 不同
├─ 大小不同? ──→ 不同
├─ 格式指纹区域 CRC32C 不同? ──→ 不同  (PNG: 尾部64KB / 其他: 头部64KB / fallback: 头部64KB)
├─ 结构化元数据字段不同? ──→ 不同  (无处理器时跳过)
├─ [网络存储默认止步] ──→ 视为相同（高置信度）
├─ SHA-256 不同? ──→ 不同
├─ [本地存储默认止步] ──→ 相同（密码学确定）
└─ 逐字节比较 ──→ 最终裁定（审计模式）
```

---

*此文档为 Svault 核心设计文档，随实现演进持续更新。*
