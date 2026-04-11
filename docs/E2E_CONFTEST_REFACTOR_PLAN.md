# Svault E2E `conftest.py` 复用重构方案

> 状态：提案，待确认后实施  
> 日期：2026-04-08

---

## 目标

将 `tests/e2e/conftest.py` 从“底层工具集合”逐步重构为“可复用场景层”，减少如下问题：

- 同一类 import 场景在多个测试文件中重复搭环境
- 每个测试都重复写一遍“造文件 -> 首次导入 -> 再次导入 -> 查询 DB -> 断言数量”
- 用例精简后，测试职责更清楚了，但 setup/断言重复仍然很多

本轮目标不是重写整个测试框架，而是建立一套**可渐进迁移**的复用层。

---

## 当前现状

`tests/e2e/conftest.py` 目前已经提供了较好的底层能力：

- `VaultEnv.run(...)`
- `VaultEnv.import_dir(...)`
- `VaultEnv.db_files()`
- `VaultEnv.find_file_in_db(...)`
- `VaultEnv.get_vault_files(...)`
- `source_factory(...)`
- `copy_fixture(...)`
- `assert_file_imported(...)`
- `assert_file_duplicate(...)`
- `assert_path_contains(...)`

这些能力适合“搭原材料”，但还不够支撑高层复用。  
当前缺的主要不是 primitive，而是**场景级 helper**。

---

## 问题分类

### 1. 重复的场景搭建

反复出现的模式包括：

- 创建 1 个文件并导入
- 创建 1 组文件，导入两次，验证无重复
- 创建 duplicate 对，验证第二次被跳过
- 创建 conflict 对，验证自动重命名
- 先 scan，再过滤，再 import

这些场景目前大多直接散落在各测试文件里，没有统一 helper。

### 2. 重复的断言模式

反复出现的断言包括：

- `len(vault.db_files()) == N`
- vault 中只有 1 个目标文件
- 某个文件路径被重命名为 `.1`
- duplicate 文件未进入 DB
- 两次导入前后总数不变

这些断言目前很多仍是测试内联，不利于后续收紧语义。

### 3. 业务流程与测试意图耦合过紧

现在不少测试把“场景搭建、命令执行、结果断言”全写在一个函数里。  
这对少量测试没问题，但当测试数量上升时，会带来：

- 读起来慢
- 合并/精简时难识别重复
- 修改 CLI 语义时需要全局改许多局部写法

---

## 重构原则

### 原则 1：不隐藏关键业务意图

不要把测试包装成过于抽象的 DSL。  
helper 应该减少重复，而不是让测试读不懂。

建议保留这种可读性：

```python
original, duplicate = create_duplicate_pair(...)
import_once(vault)
import_again(vault)
assert_duplicate_detected(vault, duplicate.name)
```

不要走到这种方向：

```python
scenario = DuplicateScenario(...).run().assert_ok()
```

### 原则 2：优先提取“高频重复场景”

不是所有重复都要抽。  
优先提取在多个文件反复出现的 setup/断言模板。

### 原则 3：向后兼容，渐进迁移

现有 `VaultEnv` 和底层 helper 不应被推翻。  
新 helper 应建立在现有能力之上，允许旧测试逐步迁移。

---

## 建议重构层次

### 第 1 层：保留现有底层 primitive

保留并继续使用：

- `VaultEnv.run`
- `VaultEnv.import_dir`
- `VaultEnv.db_files`
- `VaultEnv.find_file_in_db`
- `VaultEnv.get_vault_files`
- `source_factory`
- `copy_fixture`

这一层已经足够好，不建议大改。

### 第 2 层：新增“场景 helper”

建议新增一组函数，统一放在 `tests/e2e/conftest.py` 或后续拆到 `tests/e2e/helpers/import_scenarios.py`。

#### A. 导入执行 helper

建议新增：

- `import_once(vault, source=None, **kwargs)`
- `import_twice(vault, source=None, **kwargs)`
- `import_and_parse_json(vault, source=None, **kwargs)`

用途：

- 统一 `vault.import_dir(...)`
- 自动解析 JSON
- 减少每个测试里重复 `json.loads(...)`

#### B. 文件构造 helper

建议新增：

- `create_duplicate_pair(vault, original_name, duplicate_name, *, subdir1=None, subdir2=None)`
- `create_conflict_pair(vault, filename, *, subdir1, subdir2, content1, content2)`
- `create_batch_duplicates(vault, base_name, count)`
- `copy_camera_fixture_pair(vault, cam_a, cam_b, filename="DSC0001.jpg")`

用途：

- 统一 duplicate / conflict setup
- 降低 `test_import_dedup.py` 和未来相关用例的重复代码

#### C. 流水线 helper

建议新增：

- `run_scan(vault, source=None, *, show_dup=False)`
- `extract_new_paths_from_scan(scan_output)`
- `run_import_from_scan_output(vault, scan_output, source=None, **kwargs)`
- `run_scan_filter_import(vault, predicate, source=None, **kwargs)`

用途：

- 统一 `scan -> filter -> import --files-from` 工作流
- 方便后续补强 pipeline 测试

---

### 第 3 层：新增“断言 helper”

建议新增：

- `assert_db_file_count(vault, expected)`
- `assert_vault_file_count(vault, pattern, expected)`
- `assert_import_counts(result, *, total=None, imported=None, duplicate=None, failed=None)`
- `assert_conflict_renamed(vault, filename)`
- `assert_no_duplicate_paths(vault)`

这些 helper 的价值：

- 把统计语义统一
- 避免每个测试自己拼装 `stdout/stderr/db_files`
- 后续 CLI 输出格式调整时，修改点更集中

---

## 建议新增 API 草案

下面是建议的最小接口集合：

```python
def import_and_parse_json(
    vault: VaultEnv,
    source: Path | str | None = None,
    **kwargs: Any,
) -> dict[str, Any]:
    ...

def import_twice(
    vault: VaultEnv,
    source: Path | str | None = None,
    **kwargs: Any,
) -> tuple[subprocess.CompletedProcess[str], subprocess.CompletedProcess[str]]:
    ...

def create_duplicate_pair(
    vault: VaultEnv,
    original_name: str,
    duplicate_name: str,
    *,
    subdir1: str | None = None,
    subdir2: str | None = None,
) -> tuple[Path, Path]:
    ...

def create_conflict_pair(
    vault: VaultEnv,
    filename: str,
    *,
    subdir1: str,
    subdir2: str,
    content1: bytes,
    content2: bytes,
) -> tuple[Path, Path]:
    ...

def assert_import_counts(
    result: subprocess.CompletedProcess[str],
    *,
    total: int | None = None,
    imported: int | None = None,
    duplicate: int | None = None,
    failed: int | None = None,
) -> dict[str, Any]:
    ...
```

这组接口足够覆盖当前最常见的重复模式。

---

## 推荐迁移顺序

### Phase 1：只加 helper，不迁移旧测试

目标：

- 不动现有测试行为
- 先建立高层 helper API

验收：

- `conftest.py` 新 helper 有基础单元式验证（如轻量自测）
- 现有 E2E 全部收集通过

### Phase 2：迁移 `test_import_dedup.py`

原因：

- 这是当前重复 setup 最密集的文件之一
- duplicate/conflict 两类场景天然适合抽 helper

目标：

- 让 dedup/conflict 测试率先使用 `create_duplicate_pair / create_conflict_pair / assert_import_counts`

### Phase 3：迁移 `test_import.py` 中的 CLI 行为和双次导入场景

目标：

- 统一 `import_and_parse_json`
- 统一 `show_dup/json_mode` 断言风格

### Phase 4：迁移 `test_scan_import_pipeline.py`

目标：

- 统一 `run_scan_filter_import`
- 固化 scan/import 接口契约

---

## 不建议现在就做的事

### 1. 不建议把 `conftest.py` 一次拆成很多模块

虽然长期可能有必要，但当前还在形成测试设计阶段。  
过早拆文件会增加迁移成本。

建议顺序是：

1. 先在 `conftest.py` 内新增 helper
2. helper 稳定后，再拆到 `helpers/`

### 2. 不建议引入复杂场景对象

例如：

- `ImportScenarioBuilder`
- `DuplicateCase`
- `ScanPipelineCase`

这类对象对当前规模来说容易过度设计。  
先用简单函数式 helper 即可。

---

## 风险与控制

### 风险 1：helper 过度抽象，测试可读性下降

控制方式：

- helper 名称直接体现业务动作
- 测试里保留“场景 + 核心断言”

### 风险 2：迁移时引入行为变化

控制方式：

- 每迁移一个文件，先做纯机械替换
- 迁移后跑该文件 + 关联 marker 测试

### 风险 3：`conftest.py` 继续膨胀

控制方式：

- 先接受短期膨胀
- helper 稳定后再拆分

---

## 完成定义

满足以下条件即可认为本轮 `conftest.py` 复用重构完成：

- 新增高层 helper 覆盖主要重复场景
- `test_import_dedup.py` 至少完成一轮迁移
- `test_import.py` 或 `test_scan_import_pipeline.py` 至少完成一个文件迁移
- 文档说明新 helper 的职责
- 不改变现有测试语义

---

## 推荐实施范围

如果确认开始实施，建议第一轮只做以下内容：

1. 在 `conftest.py` 新增：
   - `import_and_parse_json`
   - `import_twice`
   - `create_duplicate_pair`
   - `create_conflict_pair`
   - `assert_import_counts`

2. 迁移：
   - `test_import_dedup.py`

3. 验收：

```bash
cd tests/e2e
bash run.sh -k "dedup or conflict"
```

这一轮改完，收益会比较直接，而且风险最低。
