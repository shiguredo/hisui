"""hisui e2e テスト補助ユーティリティ"""

import os
import shlex
import socket
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]


def build_hisui_command(binary_path: Path, *args: str) -> tuple[list[str], Path]:
    """hisui を cargo run 経由で起動するコマンドを返す"""
    _ = binary_path
    extra_args = shlex.split(os.environ.get("HISUI_E2E_CARGO_RUN_ARGS", ""))
    return (
        [
            "cargo",
            "run",
            "--quiet",
            *extra_args,
            "--bin",
            "hisui",
            "--",
            *args,
        ],
        REPO_ROOT,
    )


def reserve_ephemeral_port() -> tuple[int, socket.socket]:
    """空きポートを確保して、予約ソケットとともに返す"""
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.bind(("127.0.0.1", 0))
    port = int(sock.getsockname()[1])
    return port, sock
