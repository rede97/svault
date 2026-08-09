# 数据库结构文档

> 本文档描述 **当前实现** 的 SQLite 结构（以 `svault-core/src/db/mod.rs` 中
> 的 `SCHEMA` 常量为准）。历史设计（如 `import_sessions` 表、`.pending`
> 续传文件、**`events` 事件溯源表**——2026-08-09 移除，见
> [PARKED.md](./PARKED.md) §A2）从未实现或已移除，不在本文档记录。

---

## 总览

| 表 | 用途 |
|------|------|
| `files` | 文件记录（运行时状态索引：路径/大小/哈希/状态） |
| `media_groups` | 复合媒体组（Live Photo、RAW+JPEG 等；`files.group_id` 直接指向它） |
| `albums` | 相册（`parent_id` 邻接表树，兄弟级名称唯一） |
| `album_items` | 相册成员（指向 `files.id`；**评级在成员关系上**） |

### 场景对照（哪个功能用哪张表）

| 场景 | 表 | 读/写 |
|------|----|------|
| `import` / `add` 查重（Stage B CRC 短路、Stage D 哈希二次去重） | `files` | 读 |
| `import` / `add` / `sync` 入库（整批单事务） | `files` | 写 |
| `verify` 完整性校验（定位磁盘文件 + 比对哈希） | `files` | 读（`--background-hash` 写 `sha256`） |
| `update` 路径修正 / missing 标记 | `files` | 读写 |
| `sync` diff 比对（两侧记录） | `files` | 读（源只读） |
| `clone` 选文件导出 | `files` | 读 |
| `status` 统计 | `files` | 读 |
| `album create/list/show/add/remove/rate/delete` | `albums` / `album_items`（join `files`） | 读写 |
| 复合媒体绑定（Live Photo / RAW+JPEG） | `media_groups` | **休眠** |

**注意**：`media_groups` 当前**没有任何
SQL 读写者**——复合媒体绑定（`media/binding.rs`）现阶段只在文件系统层面
识别配对，尚未落库。`files.group_id` / `role` / `exif_fp` 三列同样休眠，
随绑定落库启用（`exif_fp` 是 EXIF 指纹，用于分组时快速匹配、避免为配对
重读全量 EXIF）。逐文件**历史**不在任何表中：见下方"会话日志"。

磁盘上的辅助文件（不在 DB 内）：

| 路径 | 用途 |
|------|------|
| `.svault/sessions/import/<ts-id>/plan.json` | import 复制前意图（src/dest/size/crc32c，原子写入） |
| `.svault/sessions/import/<ts-id>/manifest.json` | import 结果清单（原子写入） |
| `.svault/sessions/import/<ts-id>/staging/` | import 暂存 payload（入库 commit 后 rename 到最终路径） |
| `.svault/sessions/sync/<ts-id>/{plan,manifest}.json` | sync 会话日志（diff 意图 + 结果） |
| `.svault/lock` | 进程咨询锁 |

会话目录内容即状态：**有 manifest.json = 已提交**（审计记录，永久保留）；
**无 manifest.json = 被中断**（下次 import 对账报告，svault 绝不删除）。

---

## files（文件记录）

```sql
CREATE TABLE files (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    xxh3_128             BLOB,              -- XXH3-128（快速身份，导入时必算）
    sha256               BLOB,              -- SHA-256（确定身份，--full-id 或后台补算）
    size                 INTEGER NOT NULL,
    path                 TEXT    NOT NULL,  -- vault 相对路径，Unix 风格（可变，update 修正）
    mtime                INTEGER NOT NULL,  -- 源文件 mtime（Unix 毫秒）
    group_id             INTEGER REFERENCES media_groups(id),
    role                 TEXT,              -- primary/motion/depth/auxiliary
    crc32c               INTEGER,           -- 格式相关 CRC32C 指纹（导入快速预筛）
    raw_unique_id        TEXT,              -- RAW 唯一 ID（机身序列号:图像 ID）
    exif_fp              TEXT,              -- EXIF 指纹（分组用）
    status               TEXT    NOT NULL DEFAULT 'imported',  -- imported/duplicate/missing
    duplicate_of         INTEGER REFERENCES files(id),
    imported_at          INTEGER NOT NULL   -- 入库时间（Unix 毫秒）
);

CREATE INDEX idx_files_sha256 ON files(sha256);
CREATE INDEX idx_files_xxh3   ON files(xxh3_128);
CREATE INDEX idx_files_size   ON files(size);
CREATE INDEX idx_files_group  ON files(group_id);
```

**身份规则**：`sha256 IS NOT NULL` 时 SHA-256 是规范内容身份；
否则 XXH3-128 作为临时身份（由 `--background-hash` 或碰撞时升级为 SHA-256）。

**status 语义**：
- `imported` — 文件在库且在盘
- `missing` — 曾在库，磁盘上找不到（`update` 标记；可被再导入"复活"）
- `duplicate` — 重复记录（`duplicate_of` 指向主记录）

## albums / album_items（相册与评级）

```sql
CREATE TABLE albums (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_id   INTEGER REFERENCES albums(id),   -- 邻接表树，NULL = 根级
    name        TEXT    NOT NULL,
    created_at  INTEGER NOT NULL
);
-- 兄弟级名称唯一；COALESCE 让根级（NULL parent）也参与唯一约束
CREATE UNIQUE INDEX idx_albums_sibling ON albums (COALESCE(parent_id, 0), name);

CREATE TABLE album_items (
    album_id    INTEGER NOT NULL REFERENCES albums(id),
    file_id     INTEGER NOT NULL REFERENCES files(id),
    rating      INTEGER,            -- 1-5，NULL=未评级；成员级独立评级
    added_at    INTEGER NOT NULL,
    UNIQUE (album_id, file_id)
);
```

设计决策（2026-08-09，维护者拍板）：

- **成员引用 `files.id`** 而非内容哈希。成立前提（两条硬规则）：
  `files` 行**永不物理删除**——`album_items.file_id` 的外键（默认
  NO ACTION）把这条规则从约定升级为数据库约束，删除被引用行会直接
  报错；且 DB **不按 id 重建**（重建会使 id 静默指向其他文件）。
- **评级在成员关系上**（`album_items.rating`），不是文件属性：同一张
  照片在不同相册中可以有不同评级；`files` 表不持有 rating 列。
- 相册寻址用斜杠路径（`挪威旅行/特罗姆瑟`），`create` 自动建父级；
  同名可存在于不同父级下。
- `album delete` 只删空相册（无成员、无子相册）；删除的仅是元数据行，
  不触碰文件（G1）。

## media_groups

```sql
CREATE TABLE media_groups (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    group_type          TEXT    NOT NULL,   -- live_photo / raw_jpeg / single
    content_identifier  TEXT,
    captured_at         INTEGER
);
```

复合媒体绑定（Live Photo、RAW+JPEG）是进行中的工作，见 `svault-core/src/media/binding.rs`。
物理文件经 `files.group_id` 直接指向组，`files.role` 标角色
（primary/motion/auxiliary），`files.exif_fp` 存 EXIF 指纹供分组快速匹配。

> **`assets` 表已删除（2026-08-09，维护者决策）**：原三级链
> `files → media_groups → assets` 中，`assets` 只多出 `title`/`created_at`，
> 配对组与逻辑照片恒 1:1，中间层纯 join 开销。组身份由
> `media_groups.content_identifier` + `captured_at` 承载。旧 vault 中可能
> 残留空的 assets 表，无害。

> **`derivatives` 表已删除（2026-08-09，维护者决策）**：缩略图/转码等派生物
> 只有 GUI 管理场景才需要，届时应独立实现为文件系统存储（如
> `.svault/derivatives/` 二进制目录 + 独立索引），不占用主库表。
> 旧 vault 中可能残留空的 derivatives 表，无数据、无读写者，无害。

---

## 会话日志（plan / manifest）

每次 `import` / `sync` 在 `.svault/sessions/<kind>/<ts-id>/` 写 JSON
（tmp+fsync+rename 原子写入）：

- `plan.json` — 复制**前**落盘的操作意图。import：`files[]` 含
  `src_path` / `dest_path`（vault 相对，Unix 风格）/ `size` / `crc32c`；
  sync：`files[]` 含 `path` / `size` / `xxh3_128` / `sha256`。
  plan 是事后剖析的 hint，DB 是唯一真值。
- `manifest.json` — 复制**后**的结果清单：

```json
{
  "session_id": "20260809T153012-a1b2c",
  "session_type": "import",
  "source_root": "/mnt/card",
  "imported_at": 1710518400000,
  "hash_algorithm": "xxh3_128",
  "files": [
    {
      "src_path": "/mnt/card/IMG_001.CR3",
      "dest_path": "2024/03-15/Canon/IMG_001.CR3",
      "size": 52428800, "mtime_ms": 1710518400000, "crc32c": 123456789,
      "xxh3_128": "ab...", "sha256": null,
      "imported_at": 1710518400000, "status": "added", "error": null
    }
  ],
  "summary": { "total": 142, "added": 140, "duplicate": 2, "failed": 0, "skipped": 0 }
}
```


---

*发现与实现不一致时，以 `svault-core/src/db/mod.rs` 的 `SCHEMA` 为准并修正本文档。*
