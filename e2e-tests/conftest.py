"""hisui e2e テスト用 pytest fixtures"""

from pathlib import Path

import pytest

from hisui_server import REPO_ROOT


def _find_binary() -> Path:
    """hisui 実行に使うパス相当の値を返す"""
    return REPO_ROOT / "hisui"


@pytest.fixture(scope="session")
def binary_path() -> Path:
    """hisui バイナリのパス"""
    return _find_binary()
