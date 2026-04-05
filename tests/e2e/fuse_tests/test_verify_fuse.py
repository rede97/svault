"""Verify 故障注入 FUSE 测试

验证 svault verify 在 IO 故障场景下的行为。
"""

from __future__ import annotations

import pytest

pytestmark = [pytest.mark.fuse, pytest.mark.slow, pytest.mark.verify]


class TestVerifyPauseScenarios:
    """Verify 暂停场景"""
    
    def test_verify_pause_resume(self) -> None:
        """验证暂停继续"""
        pytest.skip("待实现")
    
    def test_verify_partial_failure(self) -> None:
        """部分文件验证失败"""
        pytest.skip("待实现")
