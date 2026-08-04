# 交接文档：Linux 环境收尾任务

> **致接手此工作的 pi agent**：本文档是你唯一的任务输入。
> 先完整阅读本文档，再读 `AGENTS.md` 和 `docs/ARCHITECTURE.md`，然后执行 §3 的任务清单。
>
> 写入时间：2026-08-04 · 交接自：Windows 环境的 pi agent · 当前 HEAD：`a94ccf3`

---

## 1. 当前状态（已完成，无需重做）

仓库刚完成一轮架构重构 + Clone/Sync 功能实现（提交 `54a40b5`、`493cc35`、`a94ccf3`）：

- **三 crate 分层**：`svault-core`（纯库）/ `svault-ui`（终端展现）/ `svault-cli`（薄入口）
- **统一事件接口**：core 进度通信只有 `svault-core/src/event.rs` 的 `Event` 枚举 + `EventSink` trait
- **Clone/Sync 已实现**：Beyond Compare 风格，hash 加速比对（`sync/diff.rs` 纯函数）
- **决策记录**：`docs/REFACTOR-2026-04.md` 记录了所有关键决策及被否决的方案
- **CI 守卫**：`.github/workflows/ci.yml` 含 clippy、fmt、R1/R2 架构 grep 守卫

**Windows 侧验证结果**（你不需要重复）：
- `cargo test --workspace`：153 单测全过
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告
- E2E 249 个测试中 206 通过 / 39 跳过；剩余 1 failed + 4 errors 全部为 Linux 专属，
  与重构前基线逐项比对完全相同（零回归）

## 2. 你的环境特有事项

- E2E 测试框架默认挂载 tmpfs RAMDisk（`/tmp/svault-ramdisk`），需要 sudo
- 部分测试需要 `exiftool`、`strace`、FUSE、loop 设备挂载 ext4/btrfs
- **绝对不要**在项目目录内运行 `svault init` / `svault import`（AGENTS.md 的 RAMDisk 规则）

## 3. 任务清单（按优先级）

### 任务 1：全套 E2E 基线验证 【核心任务】

```bash
git pull origin main
cargo build --workspace          # E2E 用 debug 二进制（含 scan 命令）
cd tests/e2e
sudo bash run.sh --verbose 2>&1 | tee /tmp/e2e-full.log
```

**验收标准**：全部 249 个测试通过（或只有环境性 skip）。
重点确认这些 Windows 上无法验证的测试：

| 测试文件 | 验证点 |
|----------|--------|
| `test_import_cross_fs.py` | 跨文件系统导入、reflink 探测（4 errors + 1 failed 待清零） |
| `test_import_interruption.py` | strace 信号注入中断恢复 |
| `fuse_tests/` | FUSE 故障注入（需单独跑 `run_fuse.sh`） |

若失败：先判断是环境问题（缺工具/权限）还是真实回归。
真实回归必须修复；环境问题记录在本文件 §5。

### 任务 2：跨文件系统实测（可选但推荐）

```bash
# 在真实 ext4 / btrfs 上跑（如有挂载点）
sudo bash run.sh --test-dir /mnt/ext4 -k "test_import" --cleanup
sudo bash run.sh --test-dir /mnt/btrfs --cleanup
```

**验收标准**：reflink 策略在 btrfs 上真正走 CoW（可用 `du` 验证空间占用）。

### 任务 3：CI 守卫有效性抽查

```bash
# R1：core 禁止终端依赖
! grep -E 'indicatif|console|rich_rust|clap' svault-core/Cargo.toml
# R2：core 禁止直接终端 IO（文档注释除外）
! grep -rn 'println!\|eprintln!\|read_line' svault-core/src --include='*.rs' | grep -v '///' | grep .
# 完整校验
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

**验收标准**：四条命令全部通过。同时观察 GitHub Actions 上 `a94ccf3`
之后的 CI 运行是否全绿（三个 OS）。

### 任务 4：真机场景抽查（release 二进制）

```bash
cargo build --release
# 在 RAMDisk 中构造两个 vault，验证 sync 的 reflink 路径
bash tests/setup_ramdisk.sh
cd /tmp/svault-ramdisk && mkdir -p src vaultA vaultB
# ... 导入后 sync，确认 btrfs/tmpfs 上的策略回退行为正常
```

## 4. 已知遗留事项（不是 bug，不要"顺手修"）

1. `sync --fix-moves` 未实现——moved 文件仅报告，是 v1 的刻意取舍
2. `verify --background-hash` 的 Summary 事件未统一（用 messages 输出）
3. turso 数据库引擎替换——**最低优先级**，见 ARCHITECTURE.md §6.3
4. clone 重复导出会重新复制（对方并行提交的 path+size 跳过优化未采纳，可作为后续小改进）
5. diff 引擎边缘：dest 同 identity 多路径时只保留一个索引项——v1 接受

## 5. 环境验证记录（由你填写）

| 项目 | 结果 | 备注 |
|------|------|------|
| 全套 E2E（249） | ⬜ | |
| cross_fs 4 errors + 1 failed 清零 | ⬜ | |
| fuse_tests | ⬜ | |
| CI 三 OS 全绿 | ⬜ | |
| btrfs reflink 实测 | ⬜ | |

## 6. 铁律提醒（违反=事故）

1. core 禁止终端依赖和直接 stdin/stdout 读写（CI grep 会拦，但别等 CI）
2. 永不删除用户文件——任何新功能不得引入删除路径
3. 进度通信用 `Event`/`EventSink`；交互用 `Interactor`；查询返回 Serialize 数据
4. 新功能先查 `docs/PARKED.md` 是否已被否决；文档与代码冲突时以代码为准并当场修文档
5. 测试必须在 RAMDisk 或 `--test-dir` 中进行
