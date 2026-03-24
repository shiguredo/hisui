"""bootstrap エンドポイントの e2e テスト"""

import json
import subprocess
from pathlib import Path

from hisui_server import REPO_ROOT, reserve_ephemeral_port

# ObswsServer は obsws/helpers.py にあるため sys.path を追加する
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "obsws"))
from helpers import ObswsServer


def test_bootstrap_receives_video_track(binary_path: Path):
    """bootstrap で WebRTC 接続し、映像トラックが受信できることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(binary_path, host=host, port=port):
        result = subprocess.run(
            [
                "cargo",
                "run",
                "-p",
                "bootstrap_client",
                "--",
                "--host",
                host,
                "--port",
                str(port),
                "--duration",
                "5",
            ],
            capture_output=True,
            text=True,
            timeout=60,
            cwd=REPO_ROOT,
        )
        assert result.returncode == 0, (
            f"bootstrap_client failed: stderr={result.stderr}"
        )
        stats = json.loads(result.stdout)
        assert stats["video_tracks_received"] >= 1, (
            f"expected at least 1 video track, got {stats}"
        )
