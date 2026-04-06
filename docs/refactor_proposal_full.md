# Import vs Add 重构方案

## 问题分析

### 当前代码重复情况

| 阶段 | import/mod.rs | add.rs | 重复度 | 代码量 |
|------|--------------|--------|--------|--------|
| Stage A (Scan) | ✓ | ✓ | ~90% | ~50 行 |
| Stage B (CRC32C) | ✓ | ✓ | ~90% | ~100 行 |
| Stage C (Copy) | ✓ | ✗ | N/A | ~130 行 |
| Stage D (Hash) | ✓ | ✓ | ~95% | ~100 行 |
| Stage E (DB Insert) | ✓ | ✓ | ~85% | ~80 行 |

**总计**：
- `import/mod.rs`: 774 行
- `add.rs`: 480 行
- **重复代码**: ~330 行 (占 add.rs 的 69%)

### 核心差异

| 特性 | import | add |
|------|--------|-----|
| 输入路径 | 外部源目录 | vault 内部路径 |
| Stage C (Copy) | ✓ 复制文件 | ✗ 跳过 |
| Manifest | ✓ 写入 | ✗ 不写 |
| 交互确认 | ✓ 需要 | ✗ 不需要 |
| Staging 文件 | ✓ 写入 | ✗ 不写 |

---

## 方案对比

### 方案 1: 合并为单一命令 ❌ 不推荐

```bash
# 用 flag 区分行为
svault import /external/source          # 从外部导入
svault import /vault/manual --no-copy   # 注册内部文件
svault import /vault/manual --add       # 或者用 --add flag
```

**优点**：
- 代码完全统一
- 只有一个命令

**缺点**：
- ❌ 语义混乱：`import` 暗示"从外部导入"，但 `--no-copy` 破坏了这个语义
- ❌ 用户体验差：需要记住 flag 组合
- ❌ 容易误用：用户可能在 vault 内部运行 `import` 导致意外复制
- ❌ 违反 Unix 哲学：一个命令做两件不同的事

### 方案 2: 保留两个命令 + 内部共享 Pipeline ✅ 推荐

```bash
# 语义清晰的两个命令
svault import /external/source   # 从外部导入（有 copy）
svault add /vault/manual         # 注册内部文件（无 copy）
```

**优点**：
- ✅ 语义清晰：命令名称直接表达意图
- ✅ 安全：`add` 拒绝外部路径，`import` 拒绝 vault 内部路径
- ✅ 用户体验好：不需要记忆复杂的 flag
- ✅ 代码复用：内部共享 pipeline stages

**实现**：
```rust
// 共享的 pipeline stages
svault-core/src/pipeline/
  ├── scan.rs      // Stage A: 目录扫描
  ├── crc.rs       // Stage B: CRC32C 计算
  ├── lookup.rs    // DB 查询去重
  ├── copy.rs      // Stage C: 文件传输
  ├── hash.rs      // Stage D: 强哈希验证
  └── insert.rs    // Stage E: DB 插入

// 命令实现
import::run() {
    let entries = scan::run(source)?;           // Stage A
    let crcs = crc::compute_all(entries)?;      // Stage B
    let new_files = lookup::filter_new(crcs)?;  // DB lookup
    let copied = copy::transfer(new_files)?;    // Stage C (import 独有)
    let verified = hash::verify(copied)?;       // Stage D
    insert::batch_write(verified)?;             // Stage E
}

add::run() {
    let entries = scan::run(vault_path)?;       // Stage A (复用)
    let crcs = crc::compute_all(entries)?;      // Stage B (复用)
    let new_files = lookup::filter_new(crcs)?;  // DB lookup (复用)
    // 跳过 Stage C (copy)
    let verified = hash::verify(new_files)?;    // Stage D (复用)
    insert::batch_write(verified)?;             // Stage E (复用)
}
```

---

## 推荐方案详细设计

### 1. 重构目录结构

```
svault-core/src/
├── pipeline/              # 新增：共享的 pipeline stages
│   ├── mod.rs
│   ├── scan.rs           # Stage A: 扫描文件
│   ├── crc.rs            # Stage B: CRC32C 计算
│   ├── lookup.rs         # DB 查询和去重逻辑
│   ├── copy.rs           # Stage C: 文件传输
│   ├── hash.rs           # Stage D: 强哈希验证
│   └── insert.rs         # Stage E: DB 批量插入
│
├── import/
│   ├── mod.rs            # import 命令入口（调用 pipeline）
│   ├── add.rs            # add 命令入口（调用 pipeline）
│   ├── types.rs          # 共享类型定义
│   ├── exif.rs           # EXIF 提取（被 pipeline/scan.rs 使用）
│   ├── path.rs           # 路径模板（被 pipeline/copy.rs 使用）
│   └── ...
```

### 2. 共享的 Pipeline API

```rust
// svault-core/src/pipeline/mod.rs
pub mod scan;
pub mod crc;
pub mod lookup;
pub mod copy;
pub mod hash;
pub mod insert;

// 统一的数据流类型
#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub mtime_ms: i64,
}

#[derive(Debug, Clone)]
pub struct CrcEntry {
    pub file: FileEntry,
    pub crc32c: u32,
    pub raw_unique_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LookupResult {
    pub entry: CrcEntry,
    pub status: FileStatus,  // LikelyNew / LikelyCacheDuplicate
}

#[derive(Debug, Clone)]
pub struct CopyResult {
    pub src: PathBuf,
    pub dest: PathBuf,
    pub entry: CrcEntry,
}

#[derive(Debug, Clone)]
pub struct HashResult {
    pub entry: CrcEntry,
    pub hash_bytes: Vec<u8>,
    pub is_duplicate: bool,
}
```

### 3. Stage 函数签名

```rust
// Stage A: 扫描文件
pub fn scan_files(
    root: &Path,
    extensions: &[&str],
    exclude_vault: bool,
) -> Result<Vec<FileEntry>>;

// Stage B: 计算 CRC32C
pub fn compute_crcs(
    entries: Vec<FileEntry>,
    progress: Option<ProgressBar>,
) -> Result<Vec<CrcEntry>>;

// DB 查询去重
pub fn lookup_duplicates(
    entries: Vec<CrcEntry>,
    db: &Db,
    vault_root: &Path,
) -> Result<Vec<LookupResult>>;

// Stage C: 复制文件（仅 import 使用）
pub fn copy_files(
    entries: Vec<LookupResult>,
    vault_root: &Path,
    config: &ImportConfig,
    strategies: &[TransferStrategy],
) -> Result<Vec<CopyResult>>;

// Stage D: 强哈希验证
pub fn verify_hashes(
    entries: Vec<CrcEntry>,  // 或 Vec<CopyResult>
    algo: HashAlgorithm,
    db: &Db,
) -> Result<Vec<HashResult>>;

// Stage E: DB 批量插入
pub fn batch_insert(
    entries: Vec<HashResult>,
    db: &Db,
    vault_root: &Path,
    write_manifest: bool,
) -> Result<InsertSummary>;
```

### 4. 重构后的命令实现

```rust
// svault-core/src/import/mod.rs
pub fn run(opts: ImportOptions, db: &Db) -> Result<ImportSummary> {
    use crate::pipeline::*;

    // Stage A: Scan
    let entries = scan::scan_files(
        &opts.source,
        &opts.import_config.allowed_extensions,
        true,  // exclude_vault
    )?;

    // Stage B: CRC
    let crcs = crc::compute_crcs(entries, Some(progress_bar))?;

    // DB lookup
    let lookup_results = lookup::lookup_duplicates(crcs, db, &opts.vault_root)?;

    // Filter likely_new
    let likely_new: Vec<_> = lookup_results.into_iter()
        .filter(|r| r.status == FileStatus::LikelyNew || opts.force)
        .collect();

    // Interactive confirmation
    if !opts.yes {
        confirm_import(&likely_new)?;
    }

    // Stage C: Copy
    let copied = copy::copy_files(
        likely_new,
        &opts.vault_root,
        &opts.import_config,
        &opts.strategy,
    )?;

    // Stage D: Hash
    let verified = hash::verify_hashes(
        copied.into_iter().map(|c| c.entry).collect(),
        opts.hash,
        db,
    )?;

    // Stage E: Insert
    let result = insert::batch_insert(
        verified,
        db,
        &opts.vault_root,
        true,  // write_manifest
    )?;

    Ok(result.into())
}
```

```rust
// svault-core/src/import/add.rs
pub fn run_add(opts: AddOptions, db: &Db) -> Result<AddSummary> {
    use crate::pipeline::*;

    // Stage A: Scan (复用)
    let entries = scan::scan_files(
        &opts.path,
        &config.import.allowed_extensions,
        false,  // don't exclude vault
    )?;

    // Stage B: CRC (复用)
    let crcs = crc::compute_crcs(entries, Some(progress_bar))?;

    // DB lookup (复用)
    let lookup_results = lookup::lookup_duplicates(crcs, db, &opts.vault_root)?;

    // Filter likely_new
    let likely_new: Vec<_> = lookup_results.into_iter()
        .filter(|r| r.status == FileStatus::LikelyNew)
        .map(|r| r.entry)
        .collect();

    // 跳过 Stage C (copy)

    // Stage D: Hash (复用)
    let verified = hash::verify_hashes(likely_new, opts.hash, db)?;

    // Stage E: Insert (复用)
    let result = insert::batch_insert(
        verified,
        db,
        &opts.vault_root,
        false,  // don't write manifest
    )?;

    Ok(result.into())
}
```

---

## 实现步骤

### Phase 1: 提取共享代码（2-3天）

1. 创建 `pipeline/` 模块
2. 提取 Stage A (scan) 到 `pipeline/scan.rs`
3. 提取 Stage B (crc) 到 `pipeline/crc.rs`
4. 提取 DB lookup 逻辑到 `pipeline/lookup.rs`
5. 提取 Stage D (hash) 到 `pipeline/hash.rs`
6. 提取 Stage E (insert) 到 `pipeline/insert.rs`

### Phase 2: 重构 import 命令（1天）

1. 修改 `import/mod.rs` 调用 pipeline stages
2. 保持 Stage C (copy) 在 `import/mod.rs` 或移到 `pipeline/copy.rs`
3. 运行现有测试确保功能不变

### Phase 3: 重构 add 命令（半天）

1. 修改 `add.rs` 调用 pipeline stages
2. 删除重复代码
3. 运行测试

### Phase 4: 实现 scan 命令（半天）

1. 创建 `scan` 命令，调用 Stage A + B
2. 输出 JSONL 格式

### Phase 5: 添加 --stdin 支持（1天）

1. `import` 支持从 stdin 读取 scan 输出
2. 更新测试

---

## 代码减少估算

| 文件 | 当前行数 | 重构后行数 | 减少 |
|------|---------|-----------|------|
| import/mod.rs | 774 | ~400 | -374 |
| add.rs | 480 | ~150 | -330 |
| **新增** pipeline/ | 0 | ~500 | +500 |
| **总计** | 1254 | 1050 | **-204 行** |

**收益**：
- 减少 ~200 行重复代码
- 提高可维护性（修改一处，两个命令都生效）
- 为 `scan` 命令铺路
- 支持 Unix pipeline 工作流

---

## 结论

**推荐方案 2**：保留 `import` 和 `add` 两个独立命令，内部共享 pipeline stages。

**理由**：
1. ✅ 语义清晰，用户体验好
2. ✅ 代码复用，减少重复
3. ✅ 易于扩展（添加 `scan` 命令）
4. ✅ 符合 Unix 哲学
5. ✅ 向后兼容

**不推荐合并为单一命令**，因为会导致语义混乱和用户体验下降。
