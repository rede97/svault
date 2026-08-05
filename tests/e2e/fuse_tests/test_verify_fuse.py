"""Verify 故障注入 FUSE 测试

验证 svault verify 在 IO 故障场景下的行为。

判据来源：docs/failure-handling.md。

**范围说明（2026-08-05）**：verify 只读取 vault 文件与 DB（§3.2），
不接触源目录——源侧 FUSE 对 verify 无注入面。vault 侧故障注入设施
（INFRA-3）已评估取消；verify 的故障行为已由主套件覆盖：
- 部分文件验证失败（missing/hash mismatch/io_error → exit 1）：
  `tests/e2e/test_verify.py::test_verify_multiple_corruptions` 等
- bit flip 检测：`test_verify_detects_bit_flip`
"""

from __future__ import annotations

import pytest

pytestmark = [pytest.mark.fuse, pytest.mark.slow, pytest.mark.verify]


class TestVerifyPauseScenarios:
    """Verify 暂停场景"""

    def test_verify_pause_resume(self) -> None:
        """验证暂停继续"""
        pytest.skip(
            "不适用：verify 不读源目录，源侧 FUSE 无注入面；"
            "vault 侧注入设施已取消（failure-handling.md §8.4 INFRA-3）"
        )

    def test_verify_partial_failure(self) -> None:
        """部分文件验证失败"""
        pytest.skip(
            "已覆盖：test_verify.py::test_verify_multiple_corruptions"
            "（部分失败 → exit 1，failure-handling.md §3.2/G4）"
        )
