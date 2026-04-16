# Svault AI 驱动用户行为测试方案

## 目标

本文档定义一套面向 `svault` CLI 的 AI 驱动测试方案：

- 由 AI 先设计用户场景规格
- 在隔离测试环境中执行真实命令
- 自动采集证据与结果
- 生成结构化测试报告
- 将失败项转为后续修复任务

这套方案的定位不是替代单元测试，而是补强以下能力：

- 端到端工作流验证
- 文档与真实行为一致性验证
- 用户视角的回归测试
- 探索式异常行为发现

---

## 适用范围

优先覆盖以下高价值场景：

- `svault init`
- `svault import`
- `svault add`
- `svault verify`
- `svault recheck`
- `svault update`
- `svault history`
- `svault clone`
- `svault db verify-chain`

不作为当前阶段主目标的内容：

- `sync`
- GUI 自动化测试

---

## 测试分层

Svault 的测试策略应保持三层并行：

### 1. 单元测试

验证纯逻辑与底层模块：

- hash
- config
- db
- pipeline
- media parsing

### 2. 现有 E2E 测试

验证固定工作流与回归场景：

- Python `pytest`
- RAMDisk / 自定义测试目录
- 固定 fixtures

### 3. AI 驱动用户行为测试

验证“真实用户如何使用工具”：

- 多步骤命令链路
- 输出与退出码
- 用户可理解的结果
- 文档承诺是否成立
- 非预期操作顺序

AI 驱动测试应建立在前两层之上，而不是单独存在。

---

## 设计原则

### 1. 测试真实二进制

AI 不应直接调用内部函数模拟成功，而必须运行真实命令，例如：

```bash
cargo run -p svault -- init
cargo run -p svault -- import /tmp/source
cargo run -p svault -- verify
```

### 2. 严格隔离环境

必须遵守项目规则：

- 不得在仓库目录中运行 `svault init`
- 优先使用 RAMDisk
- 每个场景单独创建测试目录
- 每个场景的输入、输出、日志、数据库结果都要独立保存

### 3. 证据优先

测试报告不接受“看起来正常”。每个场景必须采集至少一部分可审计证据：

- 命令行参数
- 退出码
- stdout / stderr
- 生成的 vault 目录结构
- manifest 路径
- 关键数据库查询结果
- 文件校验结果

### 4. 报告必须可转任务

测试报告不能停留在叙述层。每个失败项必须能转成：

- bug
- 测试缺失
- 文档不一致
- UX 问题
- 架构问题

---

## 建议目录结构

建议新增如下目录：

```text
tests/ai_scenarios/
├── specs/
│   ├── init-basic.md
│   ├── import-first-run.md
│   ├── import-duplicate.md
│   ├── verify-corruption.md
│   ├── recheck-source-modified.md
│   └── update-manual-move.md
├── runner/
│   ├── README.md
│   ├── schema.md
│   └── planned_runner.py
└── reports/
    └── .gitkeep
```

如果当前不打算立刻实现 runner，至少先建立：

- 场景规格文档格式
- 执行结果 JSON 结构
- 测试报告模板

---

## 场景规格模板

每个 AI 测试场景必须是一个结构化规格，而不是自由描述。

建议模板：

```md
# 场景名称

## 场景 ID
AI-IMPORT-001

## 用户目标
首次导入一个包含 JPEG 和 MP4 的目录，并确认文件被正确归档。

## 风险类型
- 主流程
- 路径组织
- 去重

## 前置条件
- 已构建 svault 二进制
- 使用 RAMDisk 测试目录
- 准备 source fixtures: apple_with_exif.jpg, samsung_photo.jpg, test_video_2024.mp4

## 执行步骤
1. 创建空测试目录
2. 在 vault 目录中执行 `svault init`
3. 执行 `svault import <source>`
4. 执行 `svault status`
5. 执行 `svault verify`

## 预期结果
- `init` 返回码为 0
- `import` 返回码为 0
- 至少 3 个文件进入 vault
- `verify` 返回码为 0
- manifest 文件存在

## 失败判定
- 任一命令非 0 退出
- verify 报错
- vault 中缺失导入文件
- manifest 未生成

## 必采证据
- 所有命令的 stdout/stderr
- vault 目录树
- manifest 路径
- files 表行数

## 备注
- 如果路径模板变更，需同步更新预期检查逻辑
```

---

## 推荐场景集

第一批建议落地 10 个场景：

### A. 基线主流程

1. `AI-INIT-001` 空目录初始化 vault
2. `AI-IMPORT-001` 首次导入标准图片目录
3. `AI-IMPORT-002` 首次导入图片 + 视频混合目录
4. `AI-VERIFY-001` 导入后立即执行 verify

### B. 回归高风险场景

5. `AI-IMPORT-003` 重复导入同一目录
6. `AI-IMPORT-004` 导入后源目录新增文件，再次导入
7. `AI-RECHECK-001` 导入后修改源文件，再执行 recheck
8. `AI-UPDATE-001` 手动移动 vault 文件后执行 update

### C. 破坏性与恢复场景

9. `AI-VERIFY-002` 手动破坏 vault 文件后 verify 应失败
10. `AI-DB-001` 手动篡改 events 表后 `db verify-chain` 应失败

这些场景已经覆盖：

- 用户主工作流
- 去重
- 完整性验证
- 清单重查
- 手工移动恢复
- 事件链防篡改

---

## 执行器设计

AI 驱动测试的执行器建议分两层：

### 1. 场景执行层

负责：

- 创建隔离目录
- 构造源文件和 fixtures
- 执行命令
- 采集返回结果
- 保存原始证据

### 2. 报告生成层

负责：

- 聚合场景结果
- 给出通过/失败/阻塞统计
- 提取失败原因
- 转成可执行修复任务

---

## 执行器输入格式

建议每个场景最后转成一个结构化输入对象：

```json
{
  "scenario_id": "AI-IMPORT-001",
  "title": "首次导入标准图片目录",
  "workspace": "/tmp/svault-ai/AI-IMPORT-001",
  "vault_dir": "/tmp/svault-ai/AI-IMPORT-001/vault",
  "source_dir": "/tmp/svault-ai/AI-IMPORT-001/source",
  "steps": [
    {
      "name": "init",
      "command": ["cargo", "run", "-p", "svault", "--", "init"]
    },
    {
      "name": "import",
      "command": ["cargo", "run", "-p", "svault", "--", "import", "/tmp/.../source"]
    },
    {
      "name": "verify",
      "command": ["cargo", "run", "-p", "svault", "--", "verify"]
    }
  ]
}
```

---

## 执行结果 JSON 结构

建议执行器输出一份机器可读结果：

```json
{
  "scenario_id": "AI-IMPORT-001",
  "status": "passed",
  "started_at": "2026-04-08T10:00:00+08:00",
  "finished_at": "2026-04-08T10:00:12+08:00",
  "environment": {
    "os": "linux",
    "binary": "target/debug/svault",
    "test_dir": "/tmp/svault-ai/AI-IMPORT-001"
  },
  "steps": [
    {
      "name": "init",
      "returncode": 0,
      "stdout_path": "artifacts/init.stdout.txt",
      "stderr_path": "artifacts/init.stderr.txt",
      "duration_ms": 120
    },
    {
      "name": "import",
      "returncode": 0,
      "stdout_path": "artifacts/import.stdout.txt",
      "stderr_path": "artifacts/import.stderr.txt",
      "duration_ms": 4200
    }
  ],
  "artifacts": {
    "tree": "artifacts/tree.txt",
    "db_summary": "artifacts/db_summary.json",
    "manifest_paths": ["artifacts/manifest-paths.txt"]
  },
  "assertions": [
    {
      "name": "verify_exit_code",
      "status": "passed",
      "message": "verify returned 0"
    }
  ]
}
```

---

## 证据采集清单

每个场景建议至少采集以下证据：

### 必须采集

- 所有命令的退出码
- 所有命令的 stdout
- 所有命令的 stderr
- 执行耗时
- vault 文件树
- `.svault/` 目录树

### 条件采集

- 生成的 manifest 路径
- `svault status --output json`
- `svault history --output json`
- 数据库关键查询结果
- 损坏文件前后的哈希值

### 失败时额外采集

- 出错命令的完整参数
- 失败前后目录快照
- 对应场景的 fixtures 列表

---

## AI 报告模板

建议 AI 最终输出统一格式报告：

```md
# Svault AI 用户行为测试报告

## 摘要
- 测试日期：
- 提交版本：
- 执行场景数：
- 通过：
- 失败：
- 阻塞：

## 环境
- OS：
- 二进制：
- 测试目录：
- 是否使用 RAMDisk：

## 场景结果
| ID | 场景 | 结果 | 关键结论 |
|----|------|------|----------|

## 失败详情
### AI-VERIFY-002
- 步骤：
- 实际结果：
- 预期结果：
- 证据：
- 初步根因猜测：
- 严重性：

## 文档一致性问题
- README 是否与行为一致
- CLI help 是否与行为一致
- 测试文档是否与结果一致

## 建议修复项
1. ...
2. ...

## 建议新增自动化测试
1. ...
2. ...
```

---

## 回流到 backlog 的规则

AI 测试报告中的问题必须被归类，否则报告不会产生实际价值。

建议分为：

- `bug/runtime`：实际功能错误
- `bug/data-integrity`：数据损坏、校验错误、链完整性问题
- `docs-drift`：文档与行为不一致
- `ux-cli`：提示语、错误信息、输出格式问题
- `test-gap`：应补充到 pytest 或单元测试的缺口
- `arch-risk`：暴露出架构问题，但不要求立即修复

---

## 与现有 pytest E2E 的关系

AI 驱动用户行为测试不替代 `tests/e2e/`，两者分工如下：

### pytest E2E

- 固定回归套件
- 适合 CI
- 精确断言
- 可持续维护

### AI 用户行为测试

- 适合新需求探索
- 适合测试报告生成
- 适合文档/实际行为一致性检查
- 适合发现“没人写用例但用户会这样操作”的问题

建议流程：

1. AI 场景先发现问题
2. 问题确认后，沉淀为固定 pytest E2E 或单元测试

这样 AI 测试负责“发现”，而 pytest 负责“固化”。

---

## 第一阶段实施计划

### P0：文档与规格

- 建立本方案文档
- 建立场景规格模板
- 选出首批 10 个场景

### P1：最小执行器

- 支持读取场景规格
- 支持运行命令
- 支持采集 stdout/stderr/退出码
- 支持输出 JSON 结果

### P2：报告生成

- 基于 JSON 结果生成 Markdown 报告
- 汇总通过/失败
- 输出建议修复项

### P3：纳入常规流程

- 每次较大重构后跑一轮 AI 行为测试
- 每次发布前至少跑基线场景集

---

## 当前建议

当前阶段的现实目标不是“立刻做全自动 AI QA 平台”，而是：

1. 先把场景规格定义清楚
2. 先做最小执行器
3. 先形成标准化测试报告
4. 让测试结果能稳定回流到 backlog

优先顺序应是：

- `import`
- `verify`
- `recheck`
- `update`
- `clone`
- `db verify-chain`

`sync` 不应进入第一批 AI 行为测试范围。

---

## 结论

AI 驱动用户行为测试对 Svault 是可行且有价值的，但必须以“规格化、证据化、可回流”为前提。

正确落地方向不是“让 AI 随机跑命令看看”，而是建立以下闭环：

**场景规格 -> 隔离执行 -> 证据采集 -> 结构化报告 -> backlog 修复 -> 固定测试沉淀**

这套机制建立后，Svault 才能真正把 AI 测试变成工程资产，而不是一次性的试验。
