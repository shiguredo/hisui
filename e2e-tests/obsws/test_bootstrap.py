"""bootstrap エンドポイントの e2e テスト"""

import json
import os
import shlex
import subprocess
from pathlib import Path

from helpers import ObswsServer, _inspect_mp4
from hisui_server import REPO_ROOT, reserve_ephemeral_port


def _build_bootstrap_record_command(
    host: str,
    port: int,
    duration: int,
    output_path: str,
) -> tuple[list[str], Path]:
    """obsws_bootstrap を cargo run 経由で起動するコマンドを返す"""
    extra_args = shlex.split(os.environ.get("HISUI_E2E_CARGO_RUN_ARGS", ""))
    return (
        [
            "cargo",
            "run",
            "--quiet",
            *extra_args,
            "-p",
            "obsws_bootstrap",
            "--",
            "--host",
            host,
            "--port",
            str(port),
            "--duration",
            str(duration),
            "--output-path",
            output_path,
        ],
        REPO_ROOT,
    )


def test_bootstrap_receives_video_track(binary_path: Path, tmp_path: Path):
    """bootstrap で WebRTC 接続し、映像トラックが受信できることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()
    output_mp4 = tmp_path / "output.mp4"

    with ObswsServer(binary_path, host=host, port=port):
        cmd, cwd = _build_bootstrap_record_command(
            host, port, 5, str(output_mp4)
        )
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=60,
            cwd=cwd,
        )
        assert result.returncode == 0, (
            f"obsws_bootstrap failed: stderr={result.stderr}"
        )
        stats = json.loads(result.stdout)
        assert stats["video_tracks_received"] >= 1, (
            f"expected at least 1 video track, got {stats}"
        )
        assert stats["video_frames_received"] >= 1, (
            f"expected at least 1 video frame, got {stats}"
        )
        assert stats["video_width"] == 1920, (
            f"expected video_width=1920, got {stats}"
        )
        assert stats["video_height"] == 1080, (
            f"expected video_height=1080, got {stats}"
        )

    # MP4 ファイルの検証
    assert output_mp4.exists(), "output MP4 file should exist"
    inspect = _inspect_mp4(binary_path, output_mp4)
    assert inspect.get("format") == "mp4", (
        f"expected format=mp4, got {inspect.get('format')}"
    )
    assert inspect.get("video_codec") == "vp9", (
        f"expected video_codec=vp9, got {inspect.get('video_codec')}"
    )
    assert inspect.get("video_sample_count", 0) >= 1, (
        f"expected at least 1 video sample, got {inspect}"
    )
