"""bootstrap エンドポイントの e2e テスト"""

import json
import os
import shlex
import signal
import subprocess
from pathlib import Path

from helpers import ObswsServer, _inspect_mp4
from hisui_server import REPO_ROOT, reserve_ephemeral_port

BOOTSTRAP_TIMEOUT_SECONDS = 60.0


def _build_bootstrap_command(
    host: str,
    port: int,
    duration: int,
    input_mp4_path: str,
    output_path: str,
) -> tuple[list[str], Path]:
    """obsws_bootstrap を cargo run 経由で起動するコマンドを返す"""
    # HISUI_E2E_CARGO_RUN_ARGS には --features fdk-aac など hisui 本体用の
    # オプションが含まれるため、obsws_bootstrap では --release のみ使う
    extra_args = shlex.split(os.environ.get("HISUI_E2E_CARGO_RUN_ARGS", ""))
    release_args = ["--release"] if "--release" in extra_args else []
    return (
        [
            "cargo",
            "run",
            "--quiet",
            *release_args,
            "-p",
            "obsws_bootstrap",
            "--",
            "--verbose",
            "--host",
            host,
            "--port",
            str(port),
            "--duration",
            str(duration),
            "--input-mp4-path",
            input_mp4_path,
            "--output-path",
            output_path,
        ],
        REPO_ROOT,
    )


def _format_process_failure(result: subprocess.CompletedProcess[str]) -> str:
    """異常終了した subprocess の内容を整形する"""
    details = [f"returncode={result.returncode}"]
    if result.returncode < 0:
        try:
            signal_name = signal.Signals(-result.returncode).name
            details.append(f"signal={signal_name}")
        except ValueError:
            pass
    details.append(f"stdout={result.stdout}")
    details.append(f"stderr={result.stderr}")
    return ", ".join(details)


def _run_bootstrap_command(
    cmd: list[str], cwd: Path
) -> subprocess.CompletedProcess[str]:
    """obsws_bootstrap を実行し、pytest-timeout より前に結果を回収する"""
    process = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        cwd=cwd,
    )
    try:
        stdout, stderr = process.communicate(timeout=BOOTSTRAP_TIMEOUT_SECONDS)
    except subprocess.TimeoutExpired as e:
        process.kill()
        stdout, stderr = process.communicate()
        raise subprocess.TimeoutExpired(
            cmd=e.cmd,
            timeout=e.timeout,
            output=stdout,
            stderr=stderr,
        ) from e
    return subprocess.CompletedProcess(
        cmd,
        process.returncode,
        stdout,
        stderr,
    )


def test_bootstrap_receives_video_track(binary_path: Path, tmp_path: Path):
    """bootstrap で WebRTC 接続し、映像トラックが受信できることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()
    input_mp4 = (
        Path(__file__).resolve().parents[2] / "testdata" / "red-320x320-h264-aac.mp4"
    )
    output_mp4 = tmp_path / "output.mp4"

    server = ObswsServer(binary_path, host=host, port=port)
    result = None
    try:
        with server:
            cmd, cwd = _build_bootstrap_command(
                host, port, 5, str(input_mp4), str(output_mp4)
            )
            result = _run_bootstrap_command(cmd, cwd)
            assert result.returncode == 0, (
                "obsws_bootstrap failed: "
                f"{_format_process_failure(result)}"
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
    except subprocess.TimeoutExpired as e:
        raise AssertionError(
            "obsws_bootstrap timed out: "
            f"stdout={e.stdout}, stderr={e.stderr}, {server.diagnostics()}"
        ) from e
    except Exception as e:
        bootstrap_details = ""
        if result is not None:
            bootstrap_details = (
                f" obsws_bootstrap {_format_process_failure(result)},"
            )
        raise AssertionError(f"{e}.{bootstrap_details} {server.diagnostics()}") from e

    # MP4 ファイルの検証
    assert output_mp4.exists(), "output MP4 file should exist"
    inspect = _inspect_mp4(binary_path, output_mp4)
    assert inspect.get("format") == "mp4", (
        f"expected format=mp4, got {inspect.get('format')}"
    )
    assert inspect.get("video_codec") == "VP9", (
        f"expected video_codec=VP9, got {inspect.get('video_codec')}"
    )
    assert inspect.get("video_sample_count", 0) >= 1, (
        f"expected at least 1 video sample, got {inspect}"
    )
