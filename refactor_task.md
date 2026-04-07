设计目标
  把 svault-core 中和终端展示耦合的进度条/文本输出抽离出来，替换为统一的“事件上报层”，满足这几个要求：

  - CLI 仍然可以显示进度条和状态文本
  - GUI 将来可以实时拿到扫描/查重/复制/校验事件
  - 扫描阶段能实时区分 new / duplicate / failed
  - GUI 可基于扫描事件异步加载缩略图
  - 复制阶段支持总体进度，预留每文件进度
  - 不把 channel 直接塞进 core API
  - 不做大重构，只先覆盖 import 主路径

  ———

  核心决策

  1. svault-core 不再直接依赖 indicatif、console、println!/eprintln! 来表达运行状态
  2. svault-core 新增一个薄的 reporting 抽象层
  3. core 只发事件，CLI/GUI 各自决定如何渲染
  4. core 使用 Reporter trait，不是直接依赖 channel
  5. GUI 若需要跨线程，可在适配层实现 ChannelReporter

  ———

  为什么不用“core 直接传 channel”

  不要让 core 直接吃 Sender<Event>，原因：

  - channel 是传输机制，不是领域边界
  - 会把线程模型泄漏到 core API
  - CLI、测试、同步场景会被迫围绕 channel 设计
  - trait 更容易做 NoopReporter、测试采集器、终端渲染器

  正确结构：

  - core: Reporter::emit(event)
  - CLI: TerminalReporter
  - GUI: ChannelReporter { tx }

  ———

  第一阶段范围
  只做 import 主路径，先不要全仓库推进。

  覆盖范围：

  - svault-core/src/import/mod.rs
  - svault-core/src/import/vfs_import.rs

  暂不要求本轮改：

  - add
  - update
  - recheck
  - verify

  这些后续按同一模式迁移。

  ———

  推荐模块结构

  在 svault-core 新增：

  - src/reporting/mod.rs

  建议包含：

  - CoreEvent
  - OperationKind
  - PhaseKind
  - ItemStatus
  - ItemPhase
  - Reporter trait
  - NoopReporter
  - 可能的 SharedReporter = Arc<dyn Reporter>

  ———

  事件模型

  第一版不要太复杂，但必须覆盖扫描分类和复制进度。

  建议：

  pub enum OperationKind {
      Import,
      ImportVfs,
  }

  pub enum PhaseKind {
      Scan,
      Fingerprint,
      DedupLookup,
      Copy,
      Verify,
      Insert,
  }

  pub enum ItemStatus {
      New,
      Duplicate,
      Recover,
      MovedInVault,
      Failed,
  }

  pub enum ItemPhase {
      Copy,
      Verify,
  }

  pub enum CoreEvent {
      RunStarted {
          operation: OperationKind,
      },
      RunFinished {
          operation: OperationKind,
          total: usize,
          imported: usize,
          duplicate: usize,
          failed: usize,
      },

      PhaseStarted {
          phase: PhaseKind,
          total: Option<u64>,
      },
      PhaseProgress {
          phase: PhaseKind,
          completed: u64,
          total: Option<u64>,
      },
      PhaseFinished {
          phase: PhaseKind,
      },

      ItemDiscovered {
          path: std::path::PathBuf,
          size: u64,
          mtime_ms: i64,
      },
      ItemClassified {
          path: std::path::PathBuf,
          status: ItemStatus,
          detail: Option<String>,
      },

      ItemStarted {
          path: std::path::PathBuf,
          phase: ItemPhase,
          bytes_total: Option<u64>,
      },
      ItemProgress {
          path: std::path::PathBuf,
          phase: ItemPhase,
          bytes_done: u64,
          bytes_total: Option<u64>,
      },
      ItemFinished {
          path: std::path::PathBuf,
          phase: ItemPhase,
      },

      Warning {
          message: String,
          path: Option<std::path::PathBuf>,
      },
      Error {
          message: String,
          path: Option<std::path::PathBuf>,
      },
  }

  ———

  为什么事件要这样设计

  ItemDiscovered

  - GUI 可以立刻把文件放进列表
  - GUI 可以开始异步请求缩略图
  - 不等查重结果

  ItemClassified

  - GUI/CLI 可以实时标记 new / duplicate / failed
  - 满足你说的“扫描时就知道哪些是新的哪些是重复的”

  ItemProgress

  - 为后续“每文件复制进度”预留
  - 第一版即使先只发开始/结束，也别把接口堵死

  PhaseProgress

  - CLI 最适合渲染总进度条
  - GUI 也能显示总体阶段进度

  ———

  Reporter trait

  建议：

  pub trait Reporter: Send + Sync {
      fn emit(&self, event: CoreEvent);
  }

  再提供：

  pub struct NoopReporter;

  impl Reporter for NoopReporter {
      fn emit(&self, _event: CoreEvent) {}
  }

  这样业务函数都可以接受：

  reporter: Option<&dyn Reporter>

  或者更稳一点：

  reporter: &dyn Reporter

  默认调用方传 NoopReporter。

  我更建议第二种，不要在 core 内到处判断 Option。

  ———

  CLI 侧实现建议

  在 svault-cli 新增一个终端 reporter，例如：

  - svault-cli/src/reporting.rs

  实现：

  - TerminalReporter

  职责：

  - 把 PhaseStarted/Progress/Finished 映射成 indicatif::ProgressBar
  - 把 ItemClassified(New) 映射成 Found xxx
  - 把 ItemClassified(Duplicate) 映射成 Duplicate xxx
  - 把 Warning/Error 映射成 eprintln!

  重要约束：

  - indicatif 只能留在 CLI
  - console::style 只能留在 CLI
  - core 不得再直接引用这些

  ———

  GUI 侧实现建议

  现在不实现 GUI，但接口要支持 GUI。

  以后 GUI 可以这样实现：

  pub struct ChannelReporter {
      tx: std::sync::mpsc::Sender<CoreEvent>,
  }

  impl Reporter for ChannelReporter {
      fn emit(&self, event: CoreEvent) {
          let _ = self.tx.send(event);
      }
  }

  GUI 主线程消费这些事件即可。

  ———

  扫描阶段的具体要求

  import 扫描过程中，事件顺序应该尽量接近：

  1. PhaseStarted(Scan)
  2. 每发现一个文件：
      - ItemDiscovered
  3. 完成 CRC/查重后：
      - ItemClassified
  4. PhaseFinished(Scan)

  如果现有实现里扫描和分类是绑定在一起的，不要求强行拆成两轮遍历，但至少要保证：

  - 文件进入处理时能发 ItemDiscovered
  - 得出状态后能发 ItemClassified

  状态映射建议：

  - LikelyNew -> ItemStatus::New
  - LikelyCacheDuplicate -> ItemStatus::Duplicate
  - Recover -> ItemStatus::Recover
  - Moved -> ItemStatus::MovedInVault
  - Failed(_) -> ItemStatus::Failed

  ———

  复制阶段的具体要求

  第一版目标：

  - 支持阶段总进度
  - 支持每文件开始/结束
  - 如果当前 copy 实现不方便拿到细粒度字节进度，先不要强行重写底层 copy

  所以第一版最低要求：

  - PhaseStarted(Copy { total = file_count })
  - 每个文件：
      - ItemStarted(path, Copy, Some(size))
      - copy 完成后 ItemFinished
  - 每完成一个文件：
      - PhaseProgress(Copy, completed, total)

  第二版再考虑把 transfer_file 或 VFS copy 包装成可上报 bytes_done。

  不要第一轮就为了 per-file bytes progress 重写所有传输逻辑，范围太大。

  ———

  对现有代码的处理原则

  在第一阶段，允许保留少量 summary 输出在 CLI 层，但 core 内这些都要逐步清掉：

  - ProgressBar
  - ProgressStyle
  - console::style
  - println!/eprintln!

  尤其这几处是重点：

  - svault-core/src/import/mod.rs
  - svault-core/src/import/vfs_import.rs

  如果第一轮无法完全删尽所有 summary 文本，也至少要做到：

  - 运行过程中的进度/状态输出改走 reporter
  - 最终 summary 可以暂时留在 CLI 包装层生成

  ———

  API 设计建议

  把 import API 改成接受 reporter，例如：

  pub fn run(
      opts: ImportOptions,
      db: &Db,
      reporter: &dyn Reporter,
  ) -> anyhow::Result<ImportSummary>

  pub fn run_vfs_import(
      opts: VfsImportOptions,
      db: &Db,
      reporter: &dyn Reporter,
  ) -> Result<ImportSummary>

  CLI 调用时：

  - human output: TerminalReporter
  - json output: 先用 NoopReporter，最后输出 summary JSON
  - 未来 GUI: ChannelReporter

  ———

  避免的设计错误

  不要做这些：

  1. 不要把 String 文案直接作为事件主体
     例如 Event::Log("Found foo.jpg")
  2. 不要让 core 持有 ProgressBar
  3. 不要让 core API 直接依赖 mpsc::Sender
  4. 不要为了 GUI 一次性把 verify/recheck/update/add 全迁走
  5. 不要先做复杂状态机或事件总线框架

  ———

  实现任务单

  给 kimi-code 的拆分任务如下。

  任务 1：建立 reporting 基础设施
  目标：
  在 svault-core 中新增最小 reporting 模块。

  要求：

  - 新增 reporting/mod.rs
  - 定义 Reporter trait
  - 定义 NoopReporter
  - 定义第一版 CoreEvent 及相关 enum
  - 保持类型简洁，不要过度设计

  验收：

  - cargo build 通过

  任务 2：改造本地 import 路径
  目标：
  让 svault-core/src/import/mod.rs 在扫描、分类、复制阶段通过 reporter 发事件。

  要求：

  - 先覆盖扫描分类事件
  - 再覆盖复制阶段总体进度和每文件开始/结束
  - 不要求第一版实现字节级 copy progress
  - 去掉核心流程里对 indicatif 和终端样式的直接依赖，至少在主路径上切掉

  验收：

  - cargo test -p svault-core
  - import 相关 E2E collect 不回归

  任务 3：改造 VFS import 路径
  目标：
  让 svault-core/src/import/vfs_import.rs 也走同一 reporter

  要求：

  - 事件类型保持一致
  - MTP / VFS 场景下也能发 ItemDiscovered / ItemClassified
  - copy 阶段同样先做总体+每文件开始/结束

  验收：

  - cargo build
  - 相关测试不回归

  任务 4：实现 CLI TerminalReporter
  目标：
  在 svault-cli 中新增终端渲染器，对 import 事件进行渲染。

  要求：

  - indicatif 只留在 CLI
  - human 模式显示进度条和 found/duplicate 状态
  - json 模式不要混入进度输出
  - 行为尽量贴近当前 CLI 体验，不要求完全逐字符一致

  验收：

  - svault import ... 人类模式可正常显示
  - --output json 不污染 stdout

  任务 5：补测试
  目标：
  覆盖 reporting 基础行为

  要求：

  - 至少加一个测试验证 core 会发出扫描分类事件
  - 至少加一个测试验证 import copy 阶段会发阶段事件
  - 可以用 test reporter 收集事件数组

  验收：

  - cargo test -p svault-core
  - 不引入 flaky test

  ———

  交付标准
  这一轮做完后，应该达到：

  - core 的 import 主路径不再直接控制终端展示
  - CLI 仍能正常显示进度
  - GUI 将来可通过实现 Reporter 获取扫描/查重/复制事件
  - clone 可以在这套抽象上继续做，不再把 CLI 逻辑写回 core

  ———

  最后的主管要求
  这次实现必须控制范围。不要顺手把 add/update/recheck/verify 一起重构。
  先把 import 主路径做对，证明这套边界成立，再扩展。

  如果需要，你完成第一版设计后再来找我，我会继续做第二轮 review，重点看事件粒度是否合理、CLI 适配层是否越界。