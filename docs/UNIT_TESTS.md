# Svault 测试跟踪文档

> 本文档跟踪所有单元测试和集成测试的状态，随时更新。
>
> 最后更新：2026-08-09（事件溯源移除 + 会话日志布局）

---

## 测试概览

| 类型 | 数量 | 通过 | 失败 | 跳过 |
|------|------|------|------|------|
| 单元测试 (svault-core) | 154 | 154 | 0 | 0 |
| 单元测试 (svault-ui / svault-cli) | 6 | 6 | 0 | 2 ignored |
| Python E2E 测试 (Linux) | 165 执行 | 165 | 0 | 环境性 skip |
| FUSE 故障注入 (Linux, `fuse_tests/`) | 23 | 19 | 0 | 4（P2 待设施） |

> **2026-08-09（二）事件溯源移除 + 会话日志：**
> - 删 events 表/verify-chain（伪需求，PARKED §A2）：-9 单测、-4 E2E
> - 会话日志布局 `.svault/sessions/<kind>/<ts-id>/`：plan.json fail-fast +
>   staging/ + manifest 原子写（BUG-3/4 修复）；reconcile 只报告不删
> - 单测：session 模块 6（路径/原子写/reconcile 四态）+ session_id 唯一性
> - E2E：disk_full D1/D2 重写（复制 ENOSPC=逐文件失败 exit 0）、
>   TestStagingReconcile 改写、recheck/path_compatibility 迁新布局

> **2026-08-09 import staging 原子提交（OPEN-3 闭环）：**
> - import 改走 staging → fsync → hash → 整批入库 → commit 后 rename；
>   不变量"最终路径可见 ⟹ 已完整复制+哈希+入库"
> - 启动对账：补 rename（已入库）/ 清残留（未入库）
> - +7 单测、+2 E2E（TestStagingReconcile）
> - 判据更新：failure-handling.md G1 精确化 / G7 重写 / §3.1 / §5

> **2026-08-06 E2E 套件重设计（220 → 167）：**
> - 删除 60 个冗余/零价值用例（五组，评估记录见会话与提交 `test: redesign`）；
>   修复 `test_import_dedup.py` 同名类遮蔽（3 个从不运行的死测试）
> - 合并：`test_chaos.py` → `test_import.py`（边界组）、`test_background_hash.py` → `test_verify.py`
> - 加固 6 个弱断言（config 错误信息 ×2、空格路径真实往返、hardlink inode、
>   中断后事件链校验、截断 JPEG 退出码）
> - **发现并修复真实产品缺陷**：scan 协议转义与 files-from 分词不对称——
>   空格文件名无法经 `scan | import --files-from` 导入（`pipe.rs`/`import.rs`），+3 单测 +1 E2E
> - 套件宪法（一测试一契约 / 禁止弱断言 / 覆盖归属唯一）见 tests/e2e/README.md

> **2026-08-05 FUSE 故障注入落地（08-06 精简）：**
> - 判据单一事实源：[docs/failure-handling.md](./failure-handling.md) §8
> - 基础设施：corrupt action / 运行时规则管理 / corrupt_sequence（INFRA-1/2/4）+ 6 个自验测试
> - 场景测试 11 个：P0×5（暂停中断、EIO 隔离、recheck 中断、损坏不可检出演示）、
>   P1×4（多文件暂停、源中途修改、不稳定读取、EAGAIN 内核重试）、
>   P2×2（空文件边界×2）
> - 08-06 删除 16 个无必要测试（永不实现 4 / 已有等价覆盖 7 / 冗余 5），理由见 §8.3
> - 修复 BUG-1：哈希 IO 错误误计 duplicate（`insert.rs`），+2 回归单测（分类 + manifest Failed 契约）

> **2026-04-16 架构重构变更：**
> - 移除 `test_history.py`（history 命令已移除，见 docs/PARKED.md）
> - `test_path_compatibility.py` 改用 `db dump` 查询（替代 history）
> - core 单元测试以 `cargo test -p svault-core -- --list` 为准；
>   下方逐测试表格冻结于重构前，仅保留 hash/config 等未变动模块的参考价值。

> **注意：** E2E 需在 Linux/macOS 运行（RAMDisk）。

---

---

## 逐测试清单（不再手维护）

手维护的逐测试表格必然腐烂（2026-04 重构前的冻结表格已于 2026-08-09 删除）。
权威清单以工具输出为准：

```bash
cargo test -p svault-core -- --list          # core 单元测试
cd tests/e2e && .venv/bin/pytest --collect-only -q   # E2E 用例
```

E2E 文件分工与套件宪法见 [tests/e2e/README.md](../tests/e2e/README.md)；
测试环境操作（RAMDisk、exiftool、FUSE 前置、新增用例规范）见
`.claude/skills/svault-testing/SKILL.md`。

## 运行测试

```bash
cargo test --workspace                       # Rust 单元测试
cd tests/e2e && bash run.sh                  # E2E（RAMDisk，默认排除 FUSE）
cd tests/e2e && bash run.sh --fuse           # 含 FUSE 故障注入
cd tests/e2e && bash run.sh -k "test_raw"    # 特定用例
```
