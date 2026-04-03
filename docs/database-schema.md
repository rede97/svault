# Database Schema Design

> 数据库结构设计方案

---

## 设计原则

- **路径不是身份**：文件的永久身份由内容哈希确定（SHA-256 或 XXH3-128，取决于配置），路径是可变的当前位置
- **哈希双用途分离**：`import.hash` 用于导入去重检测，`global.id_hash` 用于归档数据认证；两者可独立配置
- **SHA-256 优先原则**：若文件已有 SHA-256，无论配置如何，始终以 SHA-256 作为身份标识
- **物化视图模式**：`files`、`media_groups`、`assets` 等表是当前状态的快照，由事件重放得到
- **追加不修改**：所有变更先写 `events` 表，再更新物化视图，同一事务提交
- **防篡改哈希链**：每条事件记录前一条的哈希，构成区块链式校验

---

## 实体模型

```
Asset（逻辑资产，用户视角的「一张照片」）
  └── MediaGroup（一次拍摄产生的文件集合，可 NULL 表示单文件）
        ├── File (role=primary)     RAW / HEIC
        ├── File (role=motion)      Live Photo MOV
        ├── File (role=depth)       深度图
        └── File (role=auxiliary)   相机直出 JPEG

Derivative（后期衍生版本，挂在 Asset 下）
  ├── Derivative (type=export)      Lightroom 导出
  ├── Derivative (type=edit)        裁剪/调色版本
  └── Derivative (type=thumbnail)   系统缓存缩略图
```

---

## 核心表结构

### assets

```sql
CREATE TABLE assets (
    id          INTEGER PRIMARY KEY,
    created_at  INTEGER NOT NULL,
    title       TEXT                  -- 可选，用户自定义名称
);
```

### media_groups

```sql
CREATE TABLE media_groups (
    id                  INTEGER PRIMARY KEY,
    asset_id            INTEGER NOT NULL REFERENCES assets(id),
    group_type          TEXT NOT NULL,  -- 'live_photo'/'raw_jpeg'/'depth'/'single'
    content_identifier  TEXT,           -- Live Photo UUID，其他格式为 NULL
    captured_at         INTEGER         -- 拍摄时间，从 primary file EXIF 取
);
```

### metadata

全局元数据表，记录数据库版本和应用版本：

```sql
CREATE TABLE metadata (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);

-- 初始化记录
INSERT INTO metadata VALUES ('schema_version', '1');
INSERT INTO metadata VALUES ('app_version', '0.1.0');
INSERT INTO metadata VALUES ('crc32c_epoch', '1');  -- CRC32C 缓存世代，应用升级时递增
```

**`crc32c_epoch` 的作用**：应用版本更新时，格式处理器可能变化（读取区域调整、解析逻辑优化），所有 CRC32C 缓存自动失效。每次应用启动时检查 `app_version`，若不匹配则递增 `crc32c_epoch` 并执行 `UPDATE files SET crc32c = NULL`。

### files

```sql
CREATE TABLE files (
    id                   INTEGER PRIMARY KEY,
    xxh3_128             TEXT,              -- XXH3-128 哈希（dedup_hash=xxh3_128 时必填）
    sha256               TEXT,              -- SHA-256 哈希（dedup_hash=sha256 时必填，或后台补全）
    size                 INTEGER NOT NULL,  -- 字节数，普通索引
    path                 TEXT NOT NULL,     -- 当前路径（物化视图，可变）
    mtime                INTEGER NOT NULL,  -- 最后修改时间戳
    group_id             INTEGER REFERENCES media_groups(id),  -- NULL = 独立文件
    role                 TEXT,              -- 'primary'/'motion'/'depth'/'auxiliary'
    crc32c               INTEGER,           -- CRC32C 值（临时指纹，epoch 失效）
    status               TEXT NOT NULL DEFAULT 'imported',  -- 'imported'/'duplicate'/'deleted'
    duplicate_of         INTEGER REFERENCES files(id),      -- 重复时指向原始文件
    import_session_id    INTEGER REFERENCES import_sessions(id) NOT NULL
);

CREATE UNIQUE INDEX idx_files_xxh3_128 ON files(xxh3_128) WHERE xxh3_128 IS NOT NULL;
CREATE UNIQUE INDEX idx_files_sha256   ON files(sha256)   WHERE sha256 IS NOT NULL;
CREATE INDEX idx_files_size            ON files(size);
CREATE INDEX idx_files_group           ON files(group_id);
CREATE INDEX idx_files_session         ON files(import_session_id);
```

### import_sessions

```sql
CREATE TABLE import_sessions (
    id            INTEGER PRIMARY KEY,
    started_at    INTEGER NOT NULL,     -- 导入开始时间（Unix 毫秒时间戳）
    finished_at   INTEGER,              -- 导入完成时间（NULL = 未完成/中断）
    source_path   TEXT NOT NULL,        -- 源目录路径
    file_count    INTEGER DEFAULT 0,    -- 本次导入文件数（物化）
    status        TEXT NOT NULL,        -- 'running'/'completed'/'interrupted'
    manifest      TEXT                  -- 清单文件路径
);

CREATE INDEX idx_import_sessions_started ON import_sessions(started_at);
```

**设计说明：**
- `files.import_session_id` 替代 `imported_at`，时间从 session 表取
- 可按批次查询"这次从相机卡导入了哪些文件"
- `status='interrupted'` 的 session 可用于中断恢复
- `manifest` 记录清单文件路径，集中管理
```

**哈希字段语义：**
- **`xxh3_128`**：XXH3-128 哈希，当 `import.dedup_hash = xxh3_128` 时入库必填
- **`sha256`**：SHA-256 哈希，当 `import.dedup_hash = sha256` 时入库必填；或通过 `svault verify --background-hash` 后台补全
- **`crc32c`**：临时指纹（纯数值），仅用于导入时快速反馈，应用升级时通过 `crc32c_epoch` 批量失效

**文件身份解析优先级：**
```
if sha256 IS NOT NULL:
    identity = sha256      # SHA-256 优先，无论 global.id_hash 配置
else:
    identity = xxh3_128    # 回退到 XXH3-128
```

**`group_id = NULL` 的语义**：独立单文件，无需 MediaGroup。

### derivatives

```sql
CREATE TABLE derivatives (
    id              INTEGER PRIMARY KEY,
    asset_id        INTEGER NOT NULL REFERENCES assets(id),
    source_file_id  INTEGER NOT NULL REFERENCES files(id),
    deriv_type      TEXT NOT NULL,  -- 'export'/'edit'/'thumbnail'
    params          TEXT,           -- JSON，导出参数
    path            TEXT,           -- 衍生文件路径
    created_at      INTEGER NOT NULL
);
```

### file_exif

可选表，仅当 `import.store_exif = true` 时写入：

```sql
CREATE TABLE file_exif (
    file_id         INTEGER PRIMARY KEY REFERENCES files(id),
    captured_at     INTEGER,          -- DateTimeOriginal（Unix 时间戳）
    camera_make     TEXT,             -- EXIF Make
    camera_model    TEXT,             -- EXIF Model
    lens_model      TEXT,             -- EXIF LensModel
    focal_length    REAL,             -- 焦距（mm）
    aperture        REAL,             -- 光圈（f/N）
    shutter         TEXT,             -- 快门速度（如 "1/250"）
    iso             INTEGER,          -- ISO 感光度
    gps_lat         REAL,             -- 纬度
    gps_lon         REAL,             -- 经度
    gps_alt         REAL,             -- 海拔（m）
    raw             TEXT              -- 完整 EXIF JSON（可选）
);

CREATE INDEX idx_file_exif_captured ON file_exif(captured_at);
CREATE INDEX idx_file_exif_camera   ON file_exif(camera_model);
```

**设计说明：**
- `file_id` 作为主键（1:1 关系），查询直接 `JOIN files`
- `raw` 列存完整 EXIF JSON，方便未来扩展字段而不需要改表结构
- 默认 `import.store_exif = false`，不增加基础使用的存储开销

---

## 哈希配置（svault.toml）

```toml
[global]
id_hash = "xxh3_128"     # 永久身份哈希：xxh3_128 | sha256

[import]
dedup_hash = "xxh3_128"  # 导入去重哈希算法：xxh3_128 | sha256
store_exif = false        # 是否将完整 EXIF 写入 file_exif 表，默认关闭
```

**配置说明：**
- **`global.id_hash`**：文件的永久身份标识，入库后不再变更
- **`import.dedup_hash`**：导入时用于去重检测的哈希算法，在文件复制到归档目录后计算
- **`import.store_exif`**：开启后导入时解析完整 EXIF 并写入 `file_exif` 表，适合需要按相机/地点/时间检索的场景
- **SHA-256 优先原则**：若文件已有 `sha256`，无论 `id_hash` 配置如何，始终以 SHA-256 作为身份

**配置迁移：**
从 `id_hash = xxh3_128` 迁移到 `sha256` 时，需执行 `svault verify --background-hash` 为历史文件补全 SHA-256。

---

## 路径模板（可配置）

文件导入时，目标路径由配置文件中的模板计算得出，计算结果存入 `files.path`。模板本身不存数据库。

### 模板变量

| 变量 | 来源 | 示例 |
|------|------|------|
| `$year` | EXIF `DateTimeOriginal` 年 | `2024` |
| `$mon` | EXIF `DateTimeOriginal` 月 | `03` |
| `$day` | EXIF `DateTimeOriginal` 日 | `15` |
| `$camera` | EXIF `Model` 或 `CameraSerialNumber` | `Canon_EOS_R5` |
| `$orig_name` | 原始文件名（不含扩展名） | `IMG_001` |
| `$ext` | 文件扩展名（小写） | `cr3` |

### 模板示例

```toml
# svault.toml
[import]
path_template = "$year/$mon-$day/$camera/$orig_name.$ext"
# → 2024/03-15/Canon_EOS_R5/IMG_001.cr3

# 其他示例
# path_template = "$year/$camera/$mon$day/$orig_name.$ext"
# path_template = "$year/$mon/$day/$orig_name.$ext"
```

### 路径变更语义

- 用户修改模板**不影响**已导入文件的路径，除非显式执行 `svault reorganize`
- 路径变更（reconcile / reorganize）通过 `file.path_updated` 事件记录，历史路径全部可查
- `files.path` 始终是当前路径的物化视图，历史路径在 `events` 表中

---

## 事件溯源（Event Sourcing）

所有对实体状态的变更必须先写 `events` 表，再更新物化视图，在同一事务中提交。

### events 表

```sql
CREATE TABLE events (
    seq          INTEGER PRIMARY KEY,  -- 全局严格单调递增
    occurred_at  INTEGER NOT NULL,     -- Unix 毫秒时间戳
    event_type   TEXT NOT NULL,        -- 见事件类型表
    entity_type  TEXT NOT NULL,        -- 'file'/'media_group'/'asset'/'derivative'
    entity_id    INTEGER NOT NULL,     -- 对应实体的 id
    payload      TEXT NOT NULL,        -- JSON，变更内容
    prev_hash    TEXT NOT NULL,        -- 上一条事件的 self_hash
    self_hash    TEXT NOT NULL         -- SHA-256(seq||occurred_at||event_type||entity_id||payload||prev_hash)
);
```

### 防篡改哈希链

每条事件的 `self_hash` 由以下字段计算：

```
self_hash = SHA-256(
    seq || occurred_at || event_type || entity_id || payload || prev_hash
)
```

遍历 `events` 表验证相邻事件的哈希链，任何篡改都会导致链断裂。

### 事件类型

| event_type | 触发场景 | payload 关键字段 |
|------------|----------|------------------|
| `file.imported` | 文件首次导入 | `path`, `size`, `sha256`, `role`, `import_session_id` |
| `file.path_updated` | 路径变更（reconcile / reorganize） | `old_path`, `new_path` |
| `file.sha256_resolved` | 后台补全 SHA-256 | `sha256` |
| `file.duplicate_marked` | 标记为重复 | `duplicate_of` (file_id) |
| `file.deleted` | 从归档中移除 | `reason` |
| `media_group.created` | Combiner 建立关联 | `group_type`, `content_identifier`, `member_ids` |
| `media_group.member_added` | 新增成员 | `file_id`, `role` |
| `asset.created` | 资产创建 | `media_group_id` |
| `asset.deleted` | 资产删除（级联） | `cascade_file_ids` |
| `derivative.created` | 衍生版本生成 | `source_file_id`, `deriv_type`, `params` |
| `import_session.started` | 导入会话开始 | `source_path` |
| `import_session.finished` | 导入会话完成 | `file_count`, `manifest` |

### 写入流程

```
任何状态变更：
1. 构造 event payload（JSON）
2. 读取上一条事件的 self_hash → prev_hash
3. 计算 self_hash
4. INSERT INTO events          ← append-only
5. UPDATE 物化视图表            ← files / media_groups / assets
6. COMMIT                      ← 步骤 4+5 在同一事务
```

### 历史查询示例

```sql
-- 查某文件的完整路径变更历史
SELECT occurred_at, payload
FROM events
WHERE entity_type = 'file'
  AND entity_id = ?
  AND event_type = 'file.path_updated'
ORDER BY seq;

-- 回放某时间点之前的文件状态
SELECT * FROM events
WHERE entity_type = 'file'
  AND entity_id = ?
  AND occurred_at <= ?
ORDER BY seq;

-- 验证事件链完整性
SELECT seq, prev_hash, self_hash FROM events ORDER BY seq;
```

---

## 删除语义

| 操作 | 行为 |
|------|------|
| 删除 `Asset` | 级联软删除其 MediaGroup、所有 File 和 Derivative，写入 `asset.deleted` 事件 |
| 删除 `Derivative` | 仅删除衍生文件，不影响原始 File，写入 `derivative.deleted` 事件 |
| 删除 `File`（单文件） | 软删除（`status = 'deleted'`），物理文件由用户手动删除，Svault 不执行 |

**Svault 不执行任何物理文件删除。** 数据库中的删除均为软删除，物理文件的清理由用户根据导出的清单手动执行。

---

*此文档为 Svault 核心设计文档，随实现演进持续更新。*
