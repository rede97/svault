"""Import 故障注入 FUSE 测试

验证 svault import 在精确控制的 IO 故障场景下的行为。

判据来源：docs/failure-handling.md §8（故障注入测试判据的单一事实源）。
关键行为契约：
- 无重试/无超时/无信号处理（G2），恢复靠幂等重跑（§5）
- 逐文件失败隔离（G3），import 部分失败退出码 0（G4）
- manifest 只覆盖进入 Stage E 的文件（§6）

依赖：
- fusepy
- FUSE 内核模块

运行：
    ./run_fuse.sh -v -k test_import
"""

from __future__ import annotations

import errno
import json
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest

# 与 conftest 相同的 sys.path 处理：fuse_tests 是包（含 __init__.py），
# pytest 不会把本目录加入 sys.path，需手动加入以便导入 fault_inject_fs
sys.path.insert(0, str(Path(__file__).parent))

from conftest import VaultEnv, create_minimal_jpeg, create_minimal_mp4
from fault_inject_fs import FaultInjectedFS, FaultRule

# 标记所有测试需要 FUSE
pytestmark = [pytest.mark.fuse, pytest.mark.slow]


def _create_padded_jpeg(source_dir: Path, name: str, size: int) -> Path:
    """创建填充到指定字节的 JPEG（媒体扩展名 + 足够体量触发偏移规则）"""
    path = source_dir / name
    create_minimal_jpeg(path, name)
    current = path.stat().st_size
    if current < size:
        with open(path, "ab") as f:
            f.write(b"\xff" * (size - current))
    return path


def _vault_data_files(vault: VaultEnv) -> list[Path]:
    """vault 中的数据文件（排除 svault.toml 等配置文件）"""
    return [f for f in vault.get_vault_files() if f.name != "svault.toml"]


def _start_import(vault: VaultEnv, source: Path) -> subprocess.Popen[str]:
    """异步启动 import（供暂停/中断场景驱动）"""
    return subprocess.Popen(
        [str(vault.binary), "--yes", "import", str(source)],
        cwd=vault.vault_dir,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


def _wait_paused(fs: FaultInjectedFS, path: str, timeout: float = 20.0) -> bool:
    """轮询等待指定文件进入暂停状态"""
    deadline = time.time() + timeout
    while time.time() < deadline:
        if fs.is_paused(path):
            return True
        time.sleep(0.05)
    return False


def _read_manifests(vault: VaultEnv) -> list[dict]:
    """读取全部 import manifest（JSON）"""
    manifests_dir = vault.vault_dir / ".svault" / "manifests"
    if not manifests_dir.exists():
        return []
    return [
        json.loads(p.read_text())
        for p in sorted(manifests_dir.glob("import-*.json"))
    ]


class TestImportPauseScenarios:
    """Import 暂停场景测试"""

    @pytest.mark.interrupt
    def test_import_pause_at_25_percent(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """在 25% 处暂停导入并 SIGTERM，验证中断一致性与幂等重跑

        判据（failure-handling.md §8.1 P0-1）：
        1. 暂停于源读取阶段时进程存活
        2. SIGTERM 后：DB 无该文件记录、vault 无残留文件（暂停发生在
           Stage B CRC，复制未开始）
        3. 故障耗尽后重跑 import 幂等完成
        4. 最终 verify 通过，vault 内容与源一致
        """
        vault, fuse_mount, fs = vault_with_fuse_source
        _create_padded_jpeg(vault.source_dir, "pause25.jpg", 10 * 1024)

        fs.add_rule(
            FaultRule(
                path="/pause25.jpg",
                offset=2560,  # 10KB 的 25%
                action="pause",
                pause_event=threading.Event(),
                trigger_count=1,  # 只暂停第一次，重跑不再触发
            )
        )

        proc = _start_import(vault, fuse_mount)
        assert _wait_paused(fs, "/pause25.jpg"), "导入未在 25% 偏移处触发暂停"
        assert proc.poll() is None, "暂停期间导入进程应存活"

        proc.terminate()  # SIGTERM
        proc.wait(timeout=15)
        assert proc.returncode != 0, "SIGTERM 应使进程非零退出"
        fs.resume()  # 释放 FUSE 线程中残留的暂停

        # 中断点状态：Stage E 未到达 → DB 无记录；复制未开始 → vault 无文件
        assert vault.db_files() == [], "中断后 DB 不应有任何文件记录"
        assert _vault_data_files(vault) == [], "暂停于复制前，vault 不应有文件"

        # 幂等重跑（规则 trigger_count=1 已耗尽）
        result = vault.import_dir(fuse_mount)
        assert result.returncode == 0, f"重跑失败: {result.stderr}"

        files = vault.db_files()
        assert len(files) == 1, "重跑后 DB 应有且仅有一条记录"

        verify = vault.run("verify", check=False)
        assert verify.returncode == 0, f"verify 应通过: {verify.stderr}"

        vault_files = _vault_data_files(vault)
        assert len(vault_files) == 1
        assert vault_files[0].read_bytes() == (vault.source_dir / "pause25.jpg").read_bytes(), (
            "vault 副本内容必须与源一致"
        )

    @pytest.mark.interrupt
    def test_import_pause_at_50_resume(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """在 50% 处暂停后释放，验证导入无中断自行完成

        判据（failure-handling.md §8.1 P0-2）：
        1. 暂停期间进程存活；释放 pause 后 import 正常完成（exit 0）
        2. DB 一条记录、manifest 完整、vault 副本哈希/内容与源一致
        """
        vault, fuse_mount, fs = vault_with_fuse_source
        _create_padded_jpeg(vault.source_dir, "pause50.jpg", 10 * 1024)

        fs.add_rule(
            FaultRule(
                path="/pause50.jpg",
                offset=5120,  # 10KB 的 50%
                action="pause",
                pause_event=threading.Event(),
                trigger_count=1,
            )
        )

        proc = _start_import(vault, fuse_mount)
        assert _wait_paused(fs, "/pause50.jpg"), "导入未在 50% 偏移处触发暂停"
        assert proc.poll() is None, "暂停期间导入进程应存活"

        fs.resume()
        proc.wait(timeout=60)
        assert proc.returncode == 0, (
            f"释放暂停后导入应自行完成: rc={proc.returncode} stderr={proc.stderr.read()}"
        )

        files = vault.db_files()
        assert len(files) == 1, "完成后 DB 应有一条记录"

        manifests = _read_manifests(vault)
        assert len(manifests) == 1, "应生成一份 manifest"
        assert len(manifests[0]["files"]) == 1, "manifest 应包含该文件"
        assert manifests[0]["files"][0]["status"] == "added"

        vault_files = _vault_data_files(vault)
        assert len(vault_files) == 1
        assert vault_files[0].read_bytes() == (vault.source_dir / "pause50.jpg").read_bytes()

    def test_import_pause_multiple_files(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """多文件场景下复制阶段暂停 + SIGTERM：孤儿文件改名重复制（OPEN-3 强锁定）

        判据（failure-handling.md §8.2 P1、§5 恢复矩阵、OPEN-3 现行行为）：
        1. 暂停发生在 **Stage C 复制阶段**（200KB 文件 @100KB——JPEG CRC 只读
           头部 64KB，该偏移在 Stage B 读取范围之外，必然在复制时触发）
        2. 等待其余 9 个文件复制完成后 SIGTERM：DB 为空（整批事务未到达），
           vault 有 9 个完整孤儿 + 1 个截断半成品
        3. 重跑：10 个全部入库；10 个孤儿均改名 .1 重复制 → vault 共 20 个
           数据文件；multi_4.jpg 保持截断而 multi_4.1.jpg 完整
        """
        vault, fuse_mount, fs = vault_with_fuse_source
        file_size = 200 * 1024
        # 用 MP4 而非 JPEG：kamadak-exif 的 read_from_file 会整读 JPEG（EXIF 阶段
        # 即触发暂停）；MP4 无 EXIF 整读，CRC 为 Head+Tail(64KB)，100KB 是
        # 读取盲区，暂停必然发生在 Stage C 顺序复制
        for i in range(10):
            mp4 = vault.source_dir / f"multi_{i}.mp4"
            create_minimal_mp4(mp4, f"MULTI_{i}")
            with open(mp4, "ab") as f:
                f.write(b"\xff" * (file_size - mp4.stat().st_size))

        fs.add_rule(
            FaultRule(
                path="/multi_4.mp4",
                offset=132 * 1024,  # CRC 盲区 [128KB, 136KB)：head 合并读 128KB、
                # tail 从 136KB 起 → 只在 Stage C 复制时触发
                action="pause",
                pause_event=threading.Event(),
                trigger_count=1,
            )
        )

        proc = _start_import(vault, fuse_mount)
        assert _wait_paused(fs, "/multi_4.mp4"), "导入未在 multi_4.mp4 复制阶段触发暂停"
        assert proc.poll() is None, "暂停期间导入进程应存活"

        # 等其余 9 个文件复制完成（multi_4 暂停不阻塞 rayon 其他 worker）
        deadline = time.time() + 60
        while time.time() < deadline:
            others = [
                f for f in _vault_data_files(vault) if f.name != "multi_4.mp4"
            ]
            if len(others) == 9:
                break
            time.sleep(0.1)
        else:
            proc.kill()
            fs.resume()
            pytest.fail("其余 9 个文件未在 60s 内复制完成")
        copied_before_kill = 9

        proc.terminate()  # SIGTERM
        proc.wait(timeout=15)
        assert proc.returncode != 0
        fs.resume()

        # Stage E 整批单事务未到达 → DB 为空（§5 恢复矩阵）
        assert vault.db_files() == [], "整批事务未提交，DB 应为空"

        # 中断现场：9 个完整孤儿 + multi_4.jpg 截断半成品
        partial = [
            p for p in vault.vault_dir.rglob("multi_4.mp4") if ".svault" not in p.parts
        ]
        assert len(partial) == 1 and partial[0].stat().st_size < file_size, (
            "multi_4.mp4 应是截断半成品（OPEN-3：半成品不清理）"
        )

        # 幂等重跑：全部入库；10 个孤儿（含半成品）改名 .1 重复制
        result = vault.import_dir(fuse_mount)
        assert result.returncode == 0, f"重跑失败: {result.stderr}"
        assert len(vault.db_files()) == 10, "重跑后 10 个文件应全部入库"

        vault_files = _vault_data_files(vault)
        assert len(vault_files) == 10 + 10, (
            f"OPEN-3：10 孤儿 + 10 改名重复制 = 20，实际 {len(vault_files)}"
        )
        full_recopies = [f for f in vault_files if ".1." in f.name]
        assert len(full_recopies) == copied_before_kill + 1, (
            "9 个完整孤儿 + 1 个半成品均应改名 .1 重复制"
        )

        verify = vault.run("verify", check=False)
        assert verify.returncode == 0, f"verify 应通过: {verify.stderr}"


class TestImportErrorInjection:
    """Import 错误注入测试"""

    def test_import_eio_at_offset(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """特定偏移量 EIO：逐文件失败隔离 + 幂等重跑补齐

        判据（failure-handling.md §8.1 P0-3，G3/G4/§6）：
        1. 故障文件读取 EIO → 跳过继续，**退出码 0**（import 部分失败不 bail）
        2. 其余文件正常导入；故障文件不进 DB、不进 manifest
           （manifest 只覆盖进入 Stage E 的文件）
        3. 清除故障后重跑：故障文件成功导入，最终 verify 通过
        """
        vault, fuse_mount, fs = vault_with_fuse_source
        _create_padded_jpeg(vault.source_dir, "ok1.jpg", 10 * 1024)
        _create_padded_jpeg(vault.source_dir, "ok2.jpg", 10 * 1024)
        _create_padded_jpeg(vault.source_dir, "bad.jpg", 10 * 1024)

        fs.add_rule(
            FaultRule(
                path="/bad.jpg",
                offset=5120,
                action="error",
                error_code=errno.EIO,
            )
        )

        result = vault.import_dir(fuse_mount, check=False)
        assert result.returncode == 0, (
            f"单文件 EIO 不应导致整批失败（G3/G4）: rc={result.returncode} stderr={result.stderr}"
        )

        files = vault.db_files()
        assert len(files) == 2, "两个正常文件应入库"
        assert vault.find_file_in_db("bad.jpg") == [], "EIO 文件不应入库"

        # manifest 只覆盖进入 Stage E 的文件（§6 口径）
        manifests = _read_manifests(vault)
        assert len(manifests) == 1
        manifest_names = [
            Path(f["src_path"]).name for f in manifests[0]["files"]
        ]
        assert sorted(manifest_names) == ["ok1.jpg", "ok2.jpg"], (
            "EIO 文件（Stage B 失败）不应出现在 manifest 中"
        )

        # 清除故障后重跑：故障文件补齐
        fs.clear_rules()
        result = vault.import_dir(fuse_mount, check=False)
        assert result.returncode == 0, f"重跑失败: {result.stderr}"
        assert len(vault.db_files()) == 3, "重跑后三个文件应全部入库"

        verify = vault.run("verify", check=False)
        assert verify.returncode == 0, f"verify 应通过: {verify.stderr}"

    def test_import_eagain_error(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """EAGAIN 瞬态故障：被内核 FUSE 客户端透明重试吸收，导入成功

        判据（failure-handling.md §8.2，由 eagain_retry 改写并按实测修正）：
        svault 自身无重试层（G2），但 FUSE 内核客户端对 EAGAIN 会透明重试
        请求——前 3 次注入的 EAGAIN 被内核吸收，第 4 次（规则耗尽）成功：
        1. 注入 3 次 EAGAIN（fs.stats.error_count == 3 证明故障确实发生）
        2. import exit 0，全部文件入库，verify 通过
        """
        vault, fuse_mount, fs = vault_with_fuse_source
        _create_padded_jpeg(vault.source_dir, "ok.jpg", 10 * 1024)
        _create_padded_jpeg(vault.source_dir, "flaky.jpg", 10 * 1024)

        fs.add_rule(
            FaultRule(
                path="/flaky.jpg",
                offset=5120,
                action="error",
                error_code=errno.EAGAIN,
                trigger_count=3,
            )
        )

        result = vault.import_dir(fuse_mount, check=False)
        assert result.returncode == 0, (
            f"瞬态 EAGAIN 应被内核重试吸收: rc={result.returncode} stderr={result.stderr}"
        )
        assert fs.get_stats().error_count == 3, "3 次 EAGAIN 应确实被注入"
        assert len(vault.db_files()) == 2, "两个文件应全部入库"

        verify = vault.run("verify", check=False)
        assert verify.returncode == 0, f"verify 应通过: {verify.stderr}"


class TestImportCorruptionDetection:
    """Import 数据损坏检测测试"""

    def test_import_corrupt_at_offset(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """传输中数据被篡改的检测

        验证点：
        1. 哈希不匹配检测
        """
        pytest.skip("待实现（P2，依赖 vault 侧注入：dm-flakey 或字节翻转，见 failure-handling.md §8.4 INFRA-3）")

    def test_import_truncated_file(
        self,
        vault_with_fuse_source: tuple,
    ) -> None:
        """文件被截断的处理

        验证点：
        1. EOF 处理
        """
        pytest.skip("待实现（P2，需 FaultInjectedFS 新增 truncate action：返回比 stat 更短的数据）")
