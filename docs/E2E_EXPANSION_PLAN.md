# Svault E2E 扩展测试方案

> 状态：提案，待确认后实施  
> 日期：2026-04-08

---

## 目标

在不引入 FUSE 深度故障注入的前提下，补齐当前 E2E 测试中最容易出现回归、但覆盖仍然偏薄的部分。

本轮优先目标不是扩大测试总数，而是补足以下三类“高价值回归保护”：

1. `Reporter / output` 语义测试
2. `marker / 测试分组` 自检测试
3. `scan -> filter -> import` 流水线补强测试

---

## 范围

### 包含

- `--output human` / `--output json` 的输出边界
- JSON 模式下 `stdout` / `stderr` 职责分离
- pytest marker 选择语义
- `scan | filter | import --files-from` 工作流
- 路径过滤、空输入、部分失效输入

### 不包含

- FUSE 深度故障注入
- GUI 专属测试
- 大规模性能 benchmark

---

## 背景问题

最近的测试精简已经把 import E2E 结构收拢到较清晰的职责划分，但仍有几个风险点：

- `json` 模式已经开始承载结构化输出语义，但 E2E 只锁了“最终 JSON 可解析”，尚未锁定 `stderr` 事件流与 `stdout` 最终结果的边界。
- 测试文件合并后，`dedup/conflict` marker 语义一度发生偏移，说明“测试基础设施本身”缺少回归保护。
- `scan -> filter -> import` 是真实用户会使用的工作流，但其异常边界覆盖仍然不足。

---

## 测试设计

### 1. Reporter / Output 语义

**目标：**
锁定 CLI 在 `human/json` 两种输出模式下的职责边界，防止后续 reporter 重构污染输出。

**建议文件：**
- 优先放入 `tests/e2e/test_import.py`
- 如用例数量明显增长，可拆为 `tests/e2e/test_output_semantics.py`

**建议新增用例：**

1. `json` 模式下，`stdout` 仅包含最终结果 JSON  
说明：不允许混入人类提示、阶段名、逐文件输出。

2. `json` 模式下，结构化事件输出仅出现在 `stderr`  
说明：若当前实现选择 `stderr` 输出 JSONL 事件，测试需要锁定这一点。

3. `human` 模式下，不输出 JSON 事件对象  
说明：避免 human 模式误打印 JSON event line。

4. `json` 模式失败时，`stdout` 仍保持机器友好  
说明：失败场景至少不应出现半截人类输出污染最终结果通道。

5. `--show-dup` 只影响可见输出，不影响最终统计  
说明：已有基础测试，可补强到 human/json 两个维度。

**验收命令：**

```bash
cd tests/e2e
bash run.sh -k "json_mode or output_semantics or show_dup"
```

---

### 2. Marker / 测试分组自检

**目标：**
保护测试套件本身的结构语义，避免“内容还在，但分组选择错了”。

**建议文件：**
- 新建 `tests/e2e/test_test_suite_contract.py`

**建议新增用例：**

1. `-m dedup` 应至少收集 duplicate 语义测试  
例如：
- `test_same_file_imported_twice`
- `test_exact_duplicate_not_imported`

2. `-m conflict` 应只收集 conflict 语义测试  
例如：
- `test_two_cameras_same_filename`
- `test_multiple_cameras_same_filename`

3. `-m conflict` 不应包含 duplicate 语义测试  
例如不应包含：
- `test_same_name_same_content_is_duplicate_not_conflict`
- `test_renamed_file_detected_as_duplicate`

4. 关键 CLI 回归测试仍在默认收集集内  
例如：
- `test_json_mode_requires_yes_for_import`
- `test_json_mode_with_yes_import_succeeds`

**实现建议：**

- 使用 `subprocess.run([... "--collect-only" ...])`
- 断言返回码、收集输出和关键测试名
- 该类测试只检查“收集契约”，不执行完整业务逻辑

**验收命令：**

```bash
cd tests/e2e
bash run.sh -k "suite_contract or marker"
```

---

### 3. Scan -> Filter -> Import 流水线补强

**目标：**
锁定 `scan` 与 `import --files-from` 之间的接口契约，覆盖真实用户流水线场景。

**建议文件：**
- 扩展 `tests/e2e/test_scan_import_pipeline.py`

**建议新增用例：**

1. 全部 duplicate 时，`import --files-from` 明确失败并给出稳定错误  
说明：当前 CLI 对空 new-file 列表有显式错误路径，这一行为应锁定。

2. `files-from` 中混入不存在路径时，现存文件仍可导入  
说明：锁定部分失效输入的容错。

3. 路径包含空格时，scan 输出可被 import 正确消费  
说明：锁定转义/反转义契约。

4. 路径包含中文或非 ASCII 字符时，scan/import 管道仍正确  
说明：这类路径对归档工具是高价值真实场景。

5. 过滤后导入的统计与直接导入一致  
说明：只导入 `new:` 项时，最终 `imported/duplicate/failed` 应符合预期。

**验收命令：**

```bash
cd tests/e2e
bash run.sh -k "scan_import_pipeline or files_from"
```

---

## 优先级

### P0

- `Reporter / output` 语义测试
- `marker / 测试分组` 自检

理由：
- 直接保护 CLI 重构
- 直接保护测试维护质量

### P1

- `scan -> filter -> import` 流水线补强

理由：
- 用户工作流重要
- 已有基础测试，可低风险扩展

### P2

- import 进度单调性/统计一致性测试
- manifest/recheck 联动补强
- 特殊路径名批量场景

理由：
- 有价值，但可放在 reporter/进度设计稳定之后

---

## 非目标与约束

- 不要求在本轮实现字节级性能指标测试
- 不要求验证进度条视觉样式，只验证输出语义和统计一致性
- 不要求覆盖 FUSE/跨文件系统/磁盘满等环境专用场景

---

## 实施顺序

1. 先补 `Reporter / output` 语义测试
2. 再补 `marker / 测试分组` 自检
3. 最后扩展 `scan -> filter -> import` 用例

这个顺序的原因是：

- 第一类直接保护当前正在重构的 reporter 逻辑
- 第二类保护测试套件自身稳定性
- 第三类是业务工作流补强，适合在前两者落稳后进行

---

## 完成定义

满足以下条件，即视为本轮测试补强完成：

- 新增测试全部通过
- `bash tests/e2e/run.sh -k "json_mode or output_semantics or marker or scan_import_pipeline"` 通过
- 文档与实际测试文件职责一致
- 不引入新的专用环境依赖

---

## 建议落点

如果你确认实施，建议按以下文件落地：

- `tests/e2e/test_import.py`
- `tests/e2e/test_scan_import_pipeline.py`
- `tests/e2e/test_test_suite_contract.py`（新建）

这样可以保持：

- 主流程测试仍集中在 `test_import.py`
- 流水线接口仍集中在 `test_scan_import_pipeline.py`
- 测试基础设施契约独立，不污染业务测试文件
