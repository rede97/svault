"""Recheck 故障注入 FUSE 测试

验证 svault recheck 在 IO 故障场景下的行为。

判据来源：docs/failure-handling.md §8。关键行为契约：
- recheck 只读 DB、不改文件，恒退出码 0（不一致只写报告，§3.3）
- 报告写 .svault/sessions/recheck/<ts-id>/report.json，status 为 Debug 格式
- recheck 无断点状态：中断后重跑生成完整新报告（§8.1 P0-4）
"""

from __future__ import annotations

import json
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest

# fuse_tests 是包（含 __init__.py），需手动把本目录加入 sys.path
sys.path.insert(0, str(Path(__file__).parent))

from conftest import VaultEnv, create_minimal_jpeg
from fault_inject_fs import FaultInjectedFS, FaultRule

pytestmark = [pytest.mark.fuse, pytest.mark.slow, pytest.mark.recheck]


def _create_padded_jpeg(source_dir: Path, name: str, size: int) -> Path:
    """创建填充到指定字节的 JPEG"""
    path = source_dir / name
    create_minimal_jpeg(path, name)
    current = path.stat().st_size
    if current < size:
        with open(path, "ab") as f:
            f.write(b"\xff" * (size - current))
    return path


def _wait_paused(fs: FaultInjectedFS, path: str, timeout: float = 20.0) -> bool:
    """轮询等待指定文件进入暂停状态"""
    deadline = time.time() + timeout
    while time.time() < deadline:
        if fs.is_paused(path):
            return True
        time.sleep(0.05)
    return False


def _recheck_reports(vault: VaultEnv) -> list[Path]:
    """列出全部 recheck 报告（sessions/recheck/<ts-id>/report.json）"""
    root = vault.vault_dir / ".svault" / "sessions" / "recheck"
    if not root.exists():
        return []
    return sorted(root.glob("*/report.json"))


class TestRecheckPauseScenarios:
    """Recheck 暂停场景"""

    @pytest.mark.interrupt
    def test_recheck_pause_at_half_files(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """校验中途 SIGTERM：无状态可续，重跑生成完整新报告

        判据（failure-handling.md §8.1 P0-4，改写版）：
        1. 校验到一半时暂停，进程存活；SIGTERM 后非零退出
        2. 被杀的运行不产生报告（报告只在全部校验完成后写入）
        3. 清除故障重跑：exit 0，报告完整且全部条目 status=Ok
        """
        vault, fuse_mount, fs = vault_with_fuse_source
        for i in range(4):
            _create_padded_jpeg(vault.source_dir, f"rc{i}.jpg", 4 * 1024)

        # 正常导入（经 FUSE 挂载点，manifest 记录 source_root=fuse_mount）
        result = vault.import_dir(fuse_mount)
        assert result.returncode == 0, f"导入失败: {result.stderr}"
        assert len(vault.db_files()) == 4

        # 在第 3 个文件（rc2.jpg）校验时暂停，只触发一次
        fs.add_rule(
            FaultRule(
                path="/rc2.jpg",
                offset=100,
                action="pause",
                pause_event=threading.Event(),
                trigger_count=1,
            )
        )

        proc = subprocess.Popen(
            [str(vault.binary), "recheck", str(fuse_mount)],
            cwd=vault.vault_dir,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        assert _wait_paused(fs, "/rc2.jpg"), "recheck 未在 rc2.jpg 处触发暂停"
        assert proc.poll() is None, "暂停期间 recheck 进程应存活"

        proc.terminate()  # SIGTERM
        proc.wait(timeout=15)
        for pipe in (proc.stdout, proc.stderr):
            if pipe is not None:
                pipe.close()  # 避免 filterwarnings=error 下 ResourceWarning 误伤
        assert proc.returncode != 0, "SIGTERM 应使进程非零退出"
        fs.resume()

        # 被杀的运行不产生报告（报告在全部校验完成后才写入）
        assert _recheck_reports(vault) == [], "中断的 recheck 不应留下报告"

        # 重跑：规则已耗尽（trigger_count=1），正常完成
        retry = vault.run("recheck", str(fuse_mount), check=False)
        assert retry.returncode == 0, (
            f"recheck 恒 exit 0（不一致只写报告）: rc={retry.returncode} stderr={retry.stderr}"
        )

        reports = _recheck_reports(vault)
        assert len(reports) == 1, "重跑后应有一份完整报告"
        report = json.loads(reports[0].read_text())
        assert len(report["files"]) == 4, "报告应覆盖全部 4 个文件"
        statuses = {f["status"] for f in report["files"]}
        assert statuses == {"Ok"}, f"未修改的文件应全部 Ok，实际: {statuses}"

    def test_recheck_source_modified_during_check(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """校验过程中源文件被修改：检测到变化并正确报告

        判据（failure-handling.md §8.2 P1、§3.3）：
        1. recheck 读取源文件期间该文件被改写（写操作使缓存页作废，
           后续读取得到新内容）
        2. recheck 恒 exit 0；报告中该文件 status=SourceModified
        """
        vault, fuse_mount, fs = vault_with_fuse_source
        victim = _create_padded_jpeg(vault.source_dir, "victim.jpg", 8 * 1024)

        result = vault.import_dir(fuse_mount)
        assert result.returncode == 0, f"导入失败: {result.stderr}"

        # 在 recheck 读取 victim.jpg 时暂停
        fs.add_rule(
            FaultRule(
                path="/victim.jpg",
                offset=100,
                action="pause",
                pause_event=threading.Event(),
                trigger_count=1,
            )
        )

        proc = subprocess.Popen(
            [str(vault.binary), "recheck", str(fuse_mount)],
            cwd=vault.vault_dir,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        assert _wait_paused(fs, "/victim.jpg"), "recheck 未在 victim.jpg 处触发暂停"

        # 校验中途修改源文件（重写使页缓存作废，后续读取得到新内容）
        modified = bytearray(victim.read_bytes())
        modified[2048:2064] = b"\x00" * 16
        victim.write_bytes(bytes(modified))

        fs.resume()
        proc.wait(timeout=60)
        for pipe in (proc.stdout, proc.stderr):
            if pipe is not None:
                pipe.close()  # 避免 filterwarnings=error 下 ResourceWarning 误伤
        assert proc.returncode == 0, (
            f"recheck 恒 exit 0（不一致只写报告）: rc={proc.returncode}"
        )

        reports = _recheck_reports(vault)
        assert len(reports) == 1
        report = json.loads(reports[0].read_text())
        assert len(report["files"]) == 1
        assert report["files"][0]["status"] == "SourceModified", (
            f"校验中途修改源文件应被检出，实际: {report['files'][0]['status']}"
        )


class TestRecheckVaultErrors:
    """Vault 文件读取错误"""

    def test_recheck_vault_file_eio(self) -> None:
        """vault 文件 EIO"""
        pytest.skip("待实现（P2，vault 侧注入：dm-flakey，见 failure-handling.md §8.4 INFRA-3）")

