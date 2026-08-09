# 数据库结构文档

> 本文档描述 **当前实现** 的 SQLite 结构（以 `svault-core/src/db/mod.rs` 中
> 的 `SCHEMA` 常量为准）。历史设计（如 `import_sessions` 表、`.pending`
> 续传文件）从未实现，已不在本文档记录。

---

## 总览

| 表 | 用途 |
|------|------|
| `events` | 事件溯源日志（append-only，哈希链） |
| `files` | 文件记录（物化视图） |
| `assets` | 资产（媒体组的容器） |
| `media_groups` | 复合媒体组（Live Photo、RAW+JPEG 等） |
| `derivatives` | 派生文件（缩略图、转码等，预留） |

磁盘上的辅助文件（不在 DB 内）：

| 路径 | 用途 |
|------|------|
| `.svault/manifests/<type>-<session>.json` | 导入/同步清单（`recheck` 的输入） |
| `.svault/staging/recheck_<session>.json` | recheck 报告 |
| `.svault/staging/import/<session>/` | import 暂存区（复制→哈希→入库→rename 的暂存；中断残留由下次 import 对账清理） |
| `.svault/lock` | 进程咨询锁 |

---

## events（事件溯源）

```sql
CREATE TABLE events (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    occurred_at  INTEGER NOT NULL,   -- Unix 毫秒
    event_type   TEXT    NOT NULL,   -- 如 'file.imported' / 'file.path_updated' / 'vault.cloned'
    entity_type  TEXT    NOT NULL,   -- 'file' / 'vault' / ...
    entity_id    INTEGER NOT NULL,   -- 关联实体 ID（文件 ID 等）
    payload      TEXT    NOT NULL,   -- JSON 负载
    prev_hash    TEXT    NOT NULL,   -- 上一条事件的 self_hash（创世为 64 个 0）
    self_hash    TEXT    NOT NULL    -- 本条事件的哈希（链式完整性）
);
```

- **只增不改**：所有状态变更先写事件，再更新物化视图（同一事务）
- **哈希链**：`self_hash = H(event_type, entity_type, entity_id, payload, occurred_at, prev_hash)`，
  可用 `svault db verify-chain` 验证完整性

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

## assets / media_groups / derivatives

```sql
CREATE TABLE assets (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at  INTEGER NOT NULL,
    title       TEXT
);

CREATE TABLE media_groups (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id            INTEGER NOT NULL REFERENCES assets(id),
    group_type          TEXT    NOT NULL,   -- live_photo / raw_jpeg / single
    content_identifier  TEXT,
    captured_at         INTEGER
);

CREATE TABLE derivatives (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id        INTEGER NOT NULL REFERENCES assets(id),
    source_file_id  INTEGER NOT NULL REFERENCES files(id),
    deriv_type      TEXT    NOT NULL,
    params          TEXT,
    path            TEXT,
    created_at      INTEGER NOT NULL
);
```

复合媒体绑定（Live Photo、RAW+JPEG）是进行中的工作，见 `svault-core/src/media/binding.rs`。

---

## 清单文件（manifest）

每次 `import` / `add` / `sync` 在 `.svault/manifests/` 写一份 JSON：

```json
{
  "session_id": "1710518400",
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

`svault recheck` 以 manifest 为准同时校验源文件与 vault 副本。

---

*发现与实现不一致时，以 `svault-core/src/db/mod.rs` 的 `SCHEMA` 为准并修正本文档。*
