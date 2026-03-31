# Svault 测试计划

## 一、当前待处理事项

### 1. 提交未完成的修改

`svault-core/src/import/mod.rs` 有一处待提交的改动：在 `HashResult` 结构体和 `copied` 元组中传递 `crc32c` 字段，使 Stage E 写入数据库时能正确保存 CRC32C 值（而非 NULL）。

**状态：** 构建通过，待提交。

---

## 二、自动化测试方案

### 目标

验证 import pipeline 的核心行为：
- Stage A：正确扫描指定扩展名的文件
- Stage B：CRC32C 指纹计算 + 重复检测（`LikelyNew` vs `LikelyCacheDuplicate`）
- Stage C：文件复制到 vault，目标路径按模板解析正确
- Stage D：强哈希计算 + 三层去重（DB、batch、UNIQUE 约束）
- Stage E：DB 写入正确（`crc32c_val`、`xxh3_128`、`sha256`、`path`、`status`）

### 方案：生成合成测试图片 + 预期规则文件

#### 2.1 测试图片生成脚本（`tests/gen_fixtures.py`）

使用 Python + Pillow 生成带 EXIF 元数据的最小 JPEG 文件，覆盖以下场景：

| 文件名 | EXIF DateTimeOriginal | EXIF Make/Model | 预期 $year/$mon-$day | 预期 $device |
|--------|----------------------|-----------------|----------------------|--------------|
| `a.jpg` | `2024:05:01 10:30:00` | `Apple / iPhone 15` | `2024/05-01` | `Apple iPhone 15` |
| `b.jpg` | `2024:05:01 18:00:00` | `(无 EXIF)` | `2024/05-01` | `Unknown` |
| `c.jpg` | `(无 EXIF)` | `(无 EXIF)` | 由 mtime 决定 | `Unknown` |
| `dup_of_a.jpg` | 与 `a.jpg` 内容完全相同 | — | — | 应被识别为 duplicate |
| `same_crc_diff_content.jpg` | — | — | — | CRC32C 碰撞场景，Stage D 应正确区分 |

脚本负责：
1. 生成 JPEG 像素数据（1x1 像素即可）
2. 注入 EXIF 元数据（piexif 库）
3. 输出到 `tests/fixtures/source/` 目录
4. 同时生成 `tests/fixtures/expected.json`，记录每个文件的预期行为

#### 2.2 预期说明文件（`tests/fixtures/expected.json`）

```json
{
  "files": [
    {
      "src": "a.jpg",
      "expected_dest_pattern": "2024/05-01/Apple iPhone 15/a.jpg",
      "expected_status": "imported",
      "expected_crc32c": null,
      "note": "正常导入，EXIF 日期和设备信息正确解析"
    },
    {
      "src": "b.jpg",
      "expected_dest_pattern": "2024/05-01/Unknown/b.jpg",
      "expected_status": "imported",
      "note": "无设备 EXIF，fallback 到 Unknown"
    },
    {
      "src": "dup_of_a.jpg",
      "expected_status": "duplicate",
      "dup_reason": "db",
      "note": "与 a.jpg 内容相同，Stage D DB 查询应命中，不写入 files 表"
    }
  ]
}
```

#### 2.3 集成测试（`svault-core/tests/import_integration.rs`）

使用 Rust 集成测试框架：

```
#[test]
fn test_import_normal_file() { ... }       // a.jpg 正常导入，验证 dest 路径、DB 记录
#[test]
fn test_import_duplicate_detection() { ... } // dup_of_a.jpg 第二次导入被识别为 duplicate
#[test]
fn test_import_no_exif_fallback() { ... }  // b.jpg 无 EXIF，fallback 路径正确
#[test]
fn test_crc32c_stored_in_db() { ... }      // 验证 files.crc32c_val 非 NULL
```

每个测试：
1. 在 `tempdir` 中创建 vault（`db::init()`）
2. 调用 `import::run()`
3. 查询 SQLite `files` 表，断言字段值符合 `expected.json` 中的预期

#### 2.4 AI 辅助验证层（可选增强）

对于路径模板解析这类「规则映射」测试，可以用 Claude API 作为 oracle：
- 给 AI 提供 path_template 规则 + 输入（EXIF 日期/设备/文件名）
- AI 输出预期目标路径
- 与 `import::run()` 实际输出对比

这对于模板规则复杂化（新增 token 如 `$hour`、`$make` 等）时特别有价值。

---

## 三、实施优先级

| 优先级 | 任务 | 说明 |
|--------|------|------|
| P0 | 提交当前 `crc32c` 修改 | 构建已通过，直接提交 |
| P1 | 编写 `gen_fixtures.py` | 生成带 EXIF 的最小测试图片 |
| P2 | 编写 `import_integration.rs` | Rust 集成测试，覆盖核心场景 |
| P3 | 完善 `expected.json` | 补充边缘 case（EXIF 缺失、CRC 碰撞） |
| P4 | AI oracle 层 | 路径模板规则的 AI 验证（可选） |

---

## 四、已知 Bug / 待验证问题

1. **`crc32c_val` 曾写入 NULL**：上一个提交修复了 Stage C 不传递 `crc32c`，导致 DB 写入时该字段为 NULL 的 bug。集成测试需要覆盖此回归。
2. **`rename_template` 尚未实现**：config 中有 `rename_template = "$filename.$n.$ext"` 但 Stage C/E 中未见冲突重命名逻辑。
3. **`staging` 文件的 dest 路径列**：`write_staging()` 目前只写 `src_path` 和 `size`，缺少 `dest_path` 列。
