"""hybrid MP4 録画のクラッシュ耐性に関する e2e テスト"""

import asyncio
import json
import subprocess
import time
from pathlib import Path

import aiohttp
import pytest

from helpers import (
    OBSWS_SUBPROTOCOL,
    ObswsServer,
    _identify_with_optional_password,
    _send_obsws_request,
    _write_test_png,
)
from hisui_server import reserve_ephemeral_port


def _ffprobe_json(path: Path) -> dict:
    """ffprobe でファイルの基本情報を取得する。ffprobe がなければスキップする。"""
    try:
        result = subprocess.run(
            [
                "ffprobe",
                "-v",
                "error",
                "-show_format",
                "-show_streams",
                "-print_format",
                "json",
                str(path),
            ],
            capture_output=True,
            text=True,
            timeout=10,
        )
    except FileNotFoundError:
        pytest.skip("ffprobe not found")
    return result.returncode, json.loads(result.stdout) if result.stdout.strip() else {}, result.stderr


@pytest.mark.timeout(60)
def test_hybrid_mp4_sigkill_produces_readable_file(binary_path: Path, tmp_path: Path):
    """録画中に SIGKILL でプロセスを停止しても、出力ファイルが ffprobe で読めることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    image_path = tmp_path / "hybrid-kill-input.png"
    _write_test_png(image_path)

    record_dir = tmp_path / "recordings"
    record_dir.mkdir()

    async def _start_recording(server: ObswsServer):
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-hybrid-kill-input",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "hybrid-kill-input",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            start_response = await _send_obsws_request(
                ws,
                request_type="StartRecord",
                request_id="req-start-hybrid-kill-record",
            )
            assert start_response["d"]["requestStatus"]["result"] is True

            # 録画が進行するまで待つ
            for _ in range(50):
                status_response = await _send_obsws_request(
                    ws,
                    request_type="GetRecordStatus",
                    request_id="req-get-hybrid-kill-status",
                )
                status = status_response["d"]["responseData"]
                if status["outputActive"] is True and status.get("outputDuration", 0) > 0:
                    break
                await asyncio.sleep(0.1)

            # 数秒録画させる
            await asyncio.sleep(3.0)

            await ws.close()

    server = ObswsServer(
        binary_path,
        host=host,
        port=port,
        default_record_dir=record_dir,
        use_env=False,
    )
    server.start()
    try:
        asyncio.run(_start_recording(server))
    finally:
        # SIGKILL でプロセスを強制停止
        server.kill()

    # 録画ファイルを探す
    mp4_files = list(record_dir.glob("*.mp4"))
    assert len(mp4_files) > 0, f"録画ファイルが見つからない: {list(record_dir.iterdir())}"
    output_path = mp4_files[0]
    assert output_path.stat().st_size > 0, "録画ファイルが空"

    # ffprobe でファイルが読めることを確認
    returncode, probe_output, stderr = _ffprobe_json(output_path)
    print(f"ffprobe returncode={returncode}")
    print(f"ffprobe output={json.dumps(probe_output, indent=2)[:2000]}")
    print(f"ffprobe stderr={stderr[:1000]}")

    # クラッシュ後のファイルは fMP4 として読めるはず
    streams = probe_output.get("streams", [])
    assert len(streams) > 0, (
        f"ffprobe がストリームを検出できなかった: returncode={returncode}, stderr={stderr}"
    )


@pytest.mark.timeout(60)
def test_hybrid_mp4_normal_finalize_produces_valid_mp4(
    binary_path: Path, tmp_path: Path
):
    """正常終了時に hybrid MP4 が有効な標準 MP4 に変換されることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    image_path = tmp_path / "hybrid-normal-input.png"
    _write_test_png(image_path)

    async def _run(server: ObswsServer):
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-hybrid-normal-input",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "hybrid-normal-input",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            start_response = await _send_obsws_request(
                ws,
                request_type="StartRecord",
                request_id="req-start-hybrid-normal-record",
            )
            assert start_response["d"]["requestStatus"]["result"] is True

            # 録画が進行するまで待つ
            for _ in range(50):
                status_response = await _send_obsws_request(
                    ws,
                    request_type="GetRecordStatus",
                    request_id="req-get-hybrid-normal-status",
                )
                status = status_response["d"]["responseData"]
                if status["outputActive"] is True and status.get("outputDuration", 0) > 0:
                    break
                await asyncio.sleep(0.1)

            # 数秒録画
            await asyncio.sleep(2.0)

            stop_response = await _send_obsws_request(
                ws,
                request_type="StopRecord",
                request_id="req-stop-hybrid-normal-record",
            )
            assert stop_response["d"]["requestStatus"]["result"] is True
            output_path = Path(stop_response["d"]["responseData"]["outputPath"])

            await ws.close()
            return output_path

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        default_record_dir=tmp_path,
        use_env=False,
    ) as server:
        output_path = asyncio.run(_run(server))

    assert output_path.exists(), f"録画ファイルが存在しない: {output_path}"
    assert output_path.stat().st_size > 0, "録画ファイルが空"

    # ffprobe でファイルが読めることを確認
    returncode, probe_output, stderr = _ffprobe_json(output_path)
    print(f"ffprobe returncode={returncode}")
    print(f"ffprobe output={json.dumps(probe_output, indent=2)[:2000]}")
    if stderr:
        print(f"ffprobe stderr={stderr[:1000]}")

    assert returncode == 0, (
        f"ffprobe がファイルを読めなかった: stderr={stderr}"
    )
    streams = probe_output.get("streams", [])
    assert len(streams) > 0, "ストリームが見つからない"

    # 映像ストリームが存在することを確認
    video_streams = [s for s in streams if s.get("codec_type") == "video"]
    assert len(video_streams) > 0, f"映像ストリームが見つからない: streams={streams}"
