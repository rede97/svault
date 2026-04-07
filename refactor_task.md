这轮任务的目标不是再扩功能，而是把 reporting 重构做完整。当前状态还是“core 里加了事件”，还没做到“core 不再控制终端”。下一步请只做 import 相关收尾，不要扩到 add/recheck/update/verify。

  任务目标
  把 import 主路径中的终端渲染和交互从 svault-core 移到 svault-cli，让 reporting 成为 import 的唯一展示边界。

  范围
  只处理：

  - svault-core/src/import/mod.rs
  - svault-core/src/import/vfs_import.rs
  - svault-cli/src/reporting.rs
  - import 命令相关 CLI wiring

  不要顺手改：

  - add
  - update
  - recheck
  - verify

  任务 1：把 import 进度条从 core 移走
  要求：

  - core 不再创建 ProgressBar
  - core 不再设置 ProgressStyle
  - core 不再直接输出扫描/复制/插入阶段的进度显示
  - phase/item 进度由事件表达，CLI TerminalReporter 负责渲染

  验收：

  - svault-core/src/import/mod.rs 不再依赖 import 主路径的终端进度条
  - svault-core/src/import/vfs_import.rs 不再依赖 import 主路径的终端进度条
  - human 模式下 import 仍有合理进度显示

  任务 2：把 import 状态文本从 core 移走
  要求：

  - core 不再直接输出这些人类文案：
      - Found
      - Duplicate
      - Recover
      - Moved
      - pre-flight summary
      - final summary
  - 这些都改由 CLI reporter 根据事件渲染
  - 如果某些信息事件还不够表达，就补事件，不要把文案留在 core

  验收：

  - import 主路径中不再有终端文案渲染逻辑
  - CLI 仍能显示原有关键状态

  任务 3：补上 VFS import 的真实 reporter 接线
  要求：

  - 现在 MTP/VFS 路径不能继续用 NoopReporter
  - human 模式和 local import 一样，应当能收到事件
  - 删除现有 TODO，保证实现完整

  验收：

  - svault-cli/src/commands/import.rs 中 VFS import 使用真实 reporter
  - 不再存在 Task 3 - pass real reporter 这种未完成标记

  任务 4：引入最小交互抽象
  要求：

  - core 不再直接做 stdin.read_line() 之类的终端确认
  - 先做一个最小 Interactor 抽象即可
  - import 的确认行为通过 interactor 完成
  - CLI 负责终端确认实现
  - JSON 模式和自动化模式不要被人类提示污染

  注意：

  - 不要做完整 UI 框架
  - 只抽 import 所需的确认接口

  验收：

  - core import 主路径不直接读取 stdin
  - CLI 仍支持现有确认行为
  - --yes 行为不回归

  任务 5：修正 JSON 模式语义
  要求：

  - --output json 时，structured 输出不能再混入 pre-flight 和 prompt 文本
  - 如果需要交互，行为要明确且一致
  - 最低要求是：不要把人类文案混进 JSON 结果流

  验收：

  - 实测 svault --output json import ... 不再输出人类文本污染结果
  - 说明当前 JSON 模式在需要确认时的策略

  任务 6：补测试
  至少补这几类验证：

  - local import 会发出足够的 phase/item 事件
  - VFS import 也会走 reporter
  - JSON 模式不会被人类文本污染
  - import confirmation 通过 interactor 而不是直接 stdin

  验收：

  - cargo test -p svault-core
  - cargo build
  - cd tests/e2e && uv run python -m pytest --collect-only -q

  质量要求
  这轮完成后必须达到：

  - core 的 import 主路径不再控制终端展示
  - VFS import 不再绕开 reporter
  - JSON 模式语义干净
  - 交互边界明确
  - 不要留下新的 TODO 或半接线状态

  汇报格式
  完成后只汇报：

  - 修改了哪些文件
  - 哪些 core 输出/交互已移出
  - JSON 模式现在的行为
  - 验证命令结果摘要