"""obsws の配信・録画の開始/停止/トグルに関する e2e テスト"""

import asyncio
import concurrent.futures
import re
import time
from pathlib import Path

import aiohttp

from helpers import (
    OBSWS_SUBPROTOCOL,
    ObswsServer,
    _collect_obsws_metrics_snapshot,
    _collect_obsws_metrics_snapshot_async,
    _format_obsws_diagnostics,
    _http_get,
    _identify_with_optional_password,
    _inspect_mp4,
    _run_ffmpeg_rtmp_push,
    _run_ffmpeg_srt_push,
    _send_obsws_request,
    _start_ffmpeg_inbound_push,
    _start_ffmpeg_rtmp_receive,
    _wait_process_exit,
    _write_test_png,
)
from hisui_server import reserve_ephemeral_port

RTMP_LISTEN_RECEIVER_STARTUP_WAIT_SEC = 2.0


def _has_positive_metric(body: str, metric_prefix: str) -> bool:
    """Prometheus テキスト形式の body から metric_prefix に一致する行を探し、値が 0 より大きいか判定する"""
    for line in body.splitlines():
        if line.startswith(metric_prefix):
            match = re.search(r"\s(\d+(?:\.\d+)?)\s*$", line)
            if match and float(match.group(1)) > 0:
                return True
    return False


def test_obsws_toggle_stream_request(binary_path: Path, tmp_path: Path):
    """obsws が ToggleStream で配信状態を切り替えられることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()
    rtmp_port, rtmp_sock = reserve_ephemeral_port()
    rtmp_sock.close()

    image_path = tmp_path / "toggle-stream-input.png"
    _write_test_png(image_path)

    async def _run_toggle_stream_flow():
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
                request_id="req-create-toggle-stream-input",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "toggle-stream-input",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            set_stream_service_response = await _send_obsws_request(
                ws,
                request_type="SetStreamServiceSettings",
                request_id="req-set-toggle-stream-service",
                request_data={
                    "streamServiceType": "rtmp_custom",
                    "streamServiceSettings": {
                        "server": f"rtmp://127.0.0.1:{rtmp_port}/live",
                        "key": "toggle-stream-key",
                    },
                },
            )
            assert set_stream_service_response["d"]["requestStatus"]["result"] is True

            toggle_start_response = await _send_obsws_request(
                ws,
                request_type="ToggleStream",
                request_id="req-toggle-stream-start",
            )
            toggle_start_status = toggle_start_response["d"]["requestStatus"]
            assert toggle_start_status["result"] is True
            assert toggle_start_status["code"] == 100
            assert toggle_start_response["d"]["responseData"]["outputActive"] is True

            for _ in range(20):
                stream_status_response = await _send_obsws_request(
                    ws,
                    request_type="GetStreamStatus",
                    request_id="req-get-toggle-stream-status-on",
                )
                if stream_status_response["d"]["responseData"]["outputActive"] is True:
                    break
                await asyncio.sleep(0.1)
            else:
                raise AssertionError("stream did not become active after ToggleStream")

            toggle_stop_response = await _send_obsws_request(
                ws,
                request_type="ToggleStream",
                request_id="req-toggle-stream-stop",
            )
            toggle_stop_status = toggle_stop_response["d"]["requestStatus"]
            assert toggle_stop_status["result"] is True
            assert toggle_stop_status["code"] == 100
            assert toggle_stop_response["d"]["responseData"]["outputActive"] is False

            for _ in range(20):
                stream_status_response = await _send_obsws_request(
                    ws,
                    request_type="GetStreamStatus",
                    request_id="req-get-toggle-stream-status-off",
                )
                if stream_status_response["d"]["responseData"]["outputActive"] is False:
                    break
                await asyncio.sleep(0.1)
            else:
                raise AssertionError(
                    "stream did not become inactive after ToggleStream"
                )

            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run_toggle_stream_flow())


def test_obsws_toggle_record_request(binary_path: Path, tmp_path: Path):
    """obsws が ToggleRecord で録画状態を切り替えられることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    image_path = tmp_path / "toggle-record-input.png"
    _write_test_png(image_path)

    async def _run_toggle_record_flow():
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
                request_id="req-create-toggle-record-input",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "toggle-record-input",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            toggle_start_response = await _send_obsws_request(
                ws,
                request_type="ToggleRecord",
                request_id="req-toggle-record-start",
            )
            toggle_start_status = toggle_start_response["d"]["requestStatus"]
            assert toggle_start_status["result"] is True
            assert toggle_start_status["code"] == 100
            assert toggle_start_response["d"]["responseData"]["outputActive"] is True

            for _ in range(20):
                record_status_response = await _send_obsws_request(
                    ws,
                    request_type="GetRecordStatus",
                    request_id="req-get-toggle-record-status-on",
                )
                if record_status_response["d"]["responseData"]["outputActive"] is True:
                    break
                await asyncio.sleep(0.1)
            else:
                raise AssertionError("record did not become active after ToggleRecord")

            toggle_stop_response = await _send_obsws_request(
                ws,
                request_type="ToggleRecord",
                request_id="req-toggle-record-stop",
            )
            toggle_stop_status = toggle_stop_response["d"]["requestStatus"]
            assert toggle_stop_status["result"] is True
            assert toggle_stop_status["code"] == 100
            assert toggle_stop_response["d"]["responseData"]["outputActive"] is False

            for _ in range(20):
                record_status_response = await _send_obsws_request(
                    ws,
                    request_type="GetRecordStatus",
                    request_id="req-get-toggle-record-status-off",
                )
                if record_status_response["d"]["responseData"]["outputActive"] is False:
                    break
                await asyncio.sleep(0.1)
            else:
                raise AssertionError(
                    "record did not become inactive after ToggleRecord"
                )

            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run_toggle_record_flow())


def test_obsws_start_record_with_multiple_audio_inputs(
    binary_path: Path,
    tmp_path: Path,
):
    """obsws が複数音声入力を合成して録画できることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()
    input_path = Path(__file__).resolve().parents[2] / "testdata" / "beep-aac-audio.mp4"

    async def _run(server: ObswsServer):
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            for index in range(2):
                create_input_response = await _send_obsws_request(
                    ws,
                    request_type="CreateInput",
                    request_id=f"req-create-audio-input-{index}",
                    request_data={
                        "sceneName": "Scene",
                        "inputName": f"audio-input-{index}",
                        "inputKind": "mp4_file_source",
                        "inputSettings": {
                            "path": str(input_path),
                            "loopPlayback": True,
                        },
                        "sceneItemEnabled": True,
                    },
                )
                assert create_input_response["d"]["requestStatus"]["result"] is True

            start_record_response = await _send_obsws_request(
                ws,
                request_type="StartRecord",
                request_id="req-start-record-multi-audio",
            )
            assert start_record_response["d"]["requestStatus"]["result"] is True

            for _ in range(50):
                record_status_response = await _send_obsws_request(
                    ws,
                    request_type="GetRecordStatus",
                    request_id="req-get-record-status-multi-audio",
                )
                record_status = record_status_response["d"]["responseData"]
                if (
                    record_status["outputActive"] is True
                    and (
                        record_status["outputBytes"] > 0
                        or record_status["outputDuration"] > 0
                    )
                ):
                    break

                status, body, _ = await _http_get(
                    f"http://{server.host}:{server.port}/metrics"
                )
                assert status == 200
                if _has_positive_metric(
                    body,
                    'hisui_total_audio_sample_count{processor_id="output:record:mp4_writer:0"',
                ):
                    break
                await asyncio.sleep(0.1)
            else:
                raise AssertionError(
                    "record did not make observable progress in time"
                )

            await asyncio.sleep(2.0)

            stop_record_response = await _send_obsws_request(
                ws,
                request_type="StopRecord",
                request_id="req-stop-record-multi-audio",
            )
            assert stop_record_response["d"]["requestStatus"]["result"] is True
            output_path = Path(stop_record_response["d"]["responseData"]["outputPath"])
            assert output_path.exists()
            assert output_path.stat().st_size > 0

            # StopRecord 後にメトリクスを取得（デバッグ用）
            status, body, _ = await _http_get(
                f"http://{server.host}:{server.port}/metrics"
            )
            metrics_snapshot = f"[/metrics] status={status}\n{body}"

            await ws.close()
            return output_path, metrics_snapshot

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        default_record_dir=tmp_path,
        use_env=False,
    ) as server:
        output_path, metrics_snapshot = asyncio.run(_run(server))

    inspect_output = _inspect_mp4(
        binary_path,
        output_path,
        required_keys=("video_codec", "video_sample_count"),
    )
    print(f"inspect_output={inspect_output}")
    print(f"metrics_snapshot:\n{metrics_snapshot}")
    assert inspect_output["format"] == "mp4"
    assert inspect_output.get("audio_codec") == "OPUS", (
        f"audio_codec mismatch: inspect_output={inspect_output}"
    )
    assert inspect_output.get("audio_sample_count", 0) > 0, (
        f"audio_sample_count missing or zero: inspect_output={inspect_output}"
    )
    assert output_path.stat().st_size > 0


def test_obsws_image_source_start_stream_to_rtmp(binary_path: Path, tmp_path: Path):
    """obsws で image_source を作成し StartStream で RTMP 配信できることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    rtmp_port, rtmp_sock = reserve_ephemeral_port()
    rtmp_sock.close()

    image_path = tmp_path / "input.png"
    output_path = tmp_path / "received.mp4"
    _write_test_png(image_path)

    output_url = f"rtmp://127.0.0.1:{rtmp_port}/live"
    stream_key = "obsws-stream"
    receive_url = f"{output_url}/{stream_key}"

    async def _run_start_stream_flow():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-image-input",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "obsws-image-input",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            create_input_status = create_input_response["d"]["requestStatus"]
            assert create_input_status["result"] is True

            set_stream_service_response = await _send_obsws_request(
                ws,
                request_type="SetStreamServiceSettings",
                request_id="req-set-stream-service",
                request_data={
                    "streamServiceType": "rtmp_custom",
                    "streamServiceSettings": {
                        "server": output_url,
                        "key": stream_key,
                    },
                },
            )
            set_stream_service_status = set_stream_service_response["d"][
                "requestStatus"
            ]
            assert set_stream_service_status["result"] is True

            start_stream_response = await _send_obsws_request(
                ws,
                request_type="StartStream",
                request_id="req-start-stream",
            )
            start_stream_status = start_stream_response["d"]["requestStatus"]
            assert start_stream_status["result"] is True

            for _ in range(20):
                stream_status_response = await _send_obsws_request(
                    ws,
                    request_type="GetStreamStatus",
                    request_id="req-get-stream-status",
                )
                if stream_status_response["d"]["responseData"]["outputActive"] is True:
                    break
                await asyncio.sleep(0.1)
            else:
                raise AssertionError("stream did not become active in time")

            # 受信側が接続してデータを取り込めるように少し待ってから停止する
            # 固定待機を意図的に採用する。
            # 環境差で不安定になった場合は、将来的に GetStreamStatus ポーリングへ置き換える。
            await asyncio.sleep(5.0)

            stop_stream_response = await _send_obsws_request(
                ws,
                request_type="StopStream",
                request_id="req-stop-stream",
            )
            stop_stream_status = stop_stream_response["d"]["requestStatus"]
            assert stop_stream_status["result"] is True

            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False) as server:

        def _run_start_stream_flow_sync() -> None:
            asyncio.run(_run_start_stream_flow())

        # 受信側が先に接続待機へ入れるよう、StartStream フローは別スレッドで並行実行する
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            ffmpeg_process = _start_ffmpeg_rtmp_receive(
                receive_url,
                output_path,
                with_audio=True,
                max_video_frames=None,
                listen=True,
                timeout_seconds=20,
            )
            try:
                time.sleep(RTMP_LISTEN_RECEIVER_STARTUP_WAIT_SEC)
                start_stream_future = executor.submit(_run_start_stream_flow_sync)
                start_stream_future.result(timeout=30.0)
                _wait_process_exit(ffmpeg_process, timeout=20.0)
            except Exception as e:
                # 失敗時の原因切り分け用にメトリクスを添付する。
                metrics_snapshot = _collect_obsws_metrics_snapshot(
                    server.host,
                    server.port,
                )
                raise AssertionError(
                    f"obsws rtmp stream test failed: {e}\nmetrics_snapshot:\n{metrics_snapshot}"
                ) from e
            finally:
                if ffmpeg_process.poll() is None:
                    ffmpeg_process.kill()
                    ffmpeg_process.communicate(timeout=5)

    assert output_path.exists(), "RTMP received output file must exist"
    assert output_path.stat().st_size > 0, "RTMP received output file must not be empty"
    inspect_output = _inspect_mp4(
        binary_path,
        output_path,
        required_keys=("video_codec", "video_sample_count"),
    )
    assert inspect_output["format"] == "mp4"
    assert inspect_output["video_codec"] == "H264"
    assert inspect_output["video_sample_count"] > 0
    # OBS 互換: 音声ソースがなくても常に音声トラック（無音 AAC）が含まれる
    assert inspect_output["audio_codec"] == "AAC"
    assert inspect_output["audio_sample_count"] > 0


def test_obsws_mp4_file_source_start_stream_to_rtmp_listen_mode(
    binary_path: Path,
    tmp_path: Path,
):
    """obsws で mp4_file_source を作成し StartStream で RTMP 配信できることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    rtmp_port, rtmp_sock = reserve_ephemeral_port()
    rtmp_sock.close()

    input_path = Path(__file__).resolve().parents[2] / "testdata" / "red-320x320-h264-aac.mp4"
    output_path = tmp_path / "received-av.mp4"

    output_url = f"rtmp://127.0.0.1:{rtmp_port}/live"
    stream_key = "obsws-stream"
    receive_url = f"{output_url}/{stream_key}"

    async def _run_start_stream_flow():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-mp4-input",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "obsws-mp4-input",
                    "inputKind": "mp4_file_source",
                    "inputSettings": {"path": str(input_path)},
                    "sceneItemEnabled": True,
                },
            )
            create_input_status = create_input_response["d"]["requestStatus"]
            assert create_input_status["result"] is True

            set_stream_service_response = await _send_obsws_request(
                ws,
                request_type="SetStreamServiceSettings",
                request_id="req-set-stream-service-mp4",
                request_data={
                    "streamServiceType": "rtmp_custom",
                    "streamServiceSettings": {
                        "server": output_url,
                        "key": stream_key,
                    },
                },
            )
            set_stream_service_status = set_stream_service_response["d"][
                "requestStatus"
            ]
            assert set_stream_service_status["result"] is True

            start_stream_response = await _send_obsws_request(
                ws,
                request_type="StartStream",
                request_id="req-start-stream-mp4",
            )
            start_stream_status = start_stream_response["d"]["requestStatus"]
            assert start_stream_status["result"] is True

            for _ in range(20):
                stream_status_response = await _send_obsws_request(
                    ws,
                    request_type="GetStreamStatus",
                    request_id="req-get-stream-status-mp4",
                )
                if stream_status_response["d"]["responseData"]["outputActive"] is True:
                    break
                await asyncio.sleep(0.1)
            else:
                raise AssertionError("stream did not become active in time")

            await asyncio.sleep(1.0)

            stop_stream_response = await _send_obsws_request(
                ws,
                request_type="StopStream",
                request_id="req-stop-stream-mp4",
            )
            stop_stream_status = stop_stream_response["d"]["requestStatus"]
            assert stop_stream_status["result"] is True

            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False) as server:

        def _run_start_stream_flow_sync() -> None:
            asyncio.run(_run_start_stream_flow())

        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            ffmpeg_process = _start_ffmpeg_rtmp_receive(
                receive_url,
                output_path,
                with_audio=True,
                max_video_frames=None,
                listen=True,
                timeout_seconds=20,
            )
            try:
                time.sleep(RTMP_LISTEN_RECEIVER_STARTUP_WAIT_SEC)
                start_stream_future = executor.submit(_run_start_stream_flow_sync)
                start_stream_future.result(timeout=30.0)
                _wait_process_exit(ffmpeg_process, timeout=20.0)
            except Exception as e:
                metrics_snapshot = _collect_obsws_metrics_snapshot(
                    server.host,
                    server.port,
                )
                raise AssertionError(
                    f"obsws rtmp stream test with listen mode failed: {e}\nmetrics_snapshot:\n{metrics_snapshot}"
                ) from e
            finally:
                if ffmpeg_process.poll() is None:
                    ffmpeg_process.kill()
                    ffmpeg_process.communicate(timeout=5)

    assert output_path.exists(), "RTMP received output file must exist"
    assert output_path.stat().st_size > 0, "RTMP received output file must not be empty"
    inspect_output = _inspect_mp4(
        binary_path,
        output_path,
        required_keys=("video_codec", "video_sample_count"),
    )
    assert inspect_output["format"] == "mp4"
    assert inspect_output["video_codec"] == "H264"
    assert inspect_output["audio_codec"] == "AAC"
    assert inspect_output["video_sample_count"] > 0
    assert inspect_output["audio_sample_count"] > 0


def test_obsws_multiple_audio_inputs_start_stream_to_rtmp_listen_mode(
    binary_path: Path,
    tmp_path: Path,
):
    """obsws で複数音声入力を合成して StartStream で RTMP 配信できることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    rtmp_port, rtmp_sock = reserve_ephemeral_port()
    rtmp_sock.close()

    input_path = Path(__file__).resolve().parents[2] / "testdata" / "beep-aac-audio.mp4"
    output_path = tmp_path / "received-audio-only.mp4"

    output_url = f"rtmp://127.0.0.1:{rtmp_port}/live"
    stream_key = "obsws-stream"
    receive_url = f"{output_url}/{stream_key}"

    async def _run_start_stream_flow():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            for index in range(2):
                create_input_response = await _send_obsws_request(
                    ws,
                    request_type="CreateInput",
                    request_id=f"req-create-audio-stream-input-{index}",
                    request_data={
                        "sceneName": "Scene",
                        "inputName": f"obsws-audio-input-{index}",
                        "inputKind": "mp4_file_source",
                        "inputSettings": {
                            "path": str(input_path),
                            "loopPlayback": True,
                        },
                        "sceneItemEnabled": True,
                    },
                )
                assert create_input_response["d"]["requestStatus"]["result"] is True

            set_stream_service_response = await _send_obsws_request(
                ws,
                request_type="SetStreamServiceSettings",
                request_id="req-set-stream-service-multi-audio",
                request_data={
                    "streamServiceType": "rtmp_custom",
                    "streamServiceSettings": {
                        "server": output_url,
                        "key": stream_key,
                    },
                },
            )
            assert set_stream_service_response["d"]["requestStatus"]["result"] is True

            start_stream_response = await _send_obsws_request(
                ws,
                request_type="StartStream",
                request_id="req-start-stream-multi-audio",
            )
            assert start_stream_response["d"]["requestStatus"]["result"] is True

            for _ in range(20):
                stream_status_response = await _send_obsws_request(
                    ws,
                    request_type="GetStreamStatus",
                    request_id="req-get-stream-status-multi-audio",
                )
                if stream_status_response["d"]["responseData"]["outputActive"] is True:
                    break
                await asyncio.sleep(0.1)
            else:
                raise AssertionError("stream did not become active in time")

            await asyncio.sleep(1.0)

            stop_stream_response = await _send_obsws_request(
                ws,
                request_type="StopStream",
                request_id="req-stop-stream-multi-audio",
            )
            assert stop_stream_response["d"]["requestStatus"]["result"] is True

            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False) as server:

        def _run_start_stream_flow_sync() -> None:
            asyncio.run(_run_start_stream_flow())

        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            ffmpeg_process = _start_ffmpeg_rtmp_receive(
                receive_url,
                output_path,
                with_audio=True,
                max_video_frames=None,
                listen=True,
                timeout_seconds=20,
            )
            try:
                time.sleep(RTMP_LISTEN_RECEIVER_STARTUP_WAIT_SEC)
                start_stream_future = executor.submit(_run_start_stream_flow_sync)
                start_stream_future.result(timeout=30.0)
                _wait_process_exit(ffmpeg_process, timeout=20.0)
            except Exception as e:
                metrics_snapshot = _collect_obsws_metrics_snapshot(
                    server.host,
                    server.port,
                )
                raise AssertionError(
                    f"obsws rtmp multi-audio stream test failed: {e}\nmetrics_snapshot:\n{metrics_snapshot}"
                ) from e
            finally:
                if ffmpeg_process.poll() is None:
                    ffmpeg_process.kill()
                    ffmpeg_process.communicate(timeout=5)

    assert output_path.exists(), "RTMP received output file must exist"
    assert output_path.stat().st_size > 0, "RTMP received output file must not be empty"
    inspect_output = _inspect_mp4(
        binary_path,
        output_path,
        required_keys=("video_codec", "video_sample_count"),
    )
    assert inspect_output["format"] == "mp4"
    assert inspect_output["audio_codec"] == "AAC"
    assert inspect_output["audio_sample_count"] > 0
    # OBS 互換: 映像ソースがなくても常に映像トラック（黒画面）が含まれる
    assert inspect_output["video_codec"] == "H264"
    assert inspect_output["video_sample_count"] > 0


def test_obsws_rtmp_inbound_start_record_and_inspect_output(
    binary_path: Path,
    tmp_path: Path,
):
    """obsws で rtmp_inbound を作成し StartRecord → ffmpeg RTMP push → StopRecord で録画できることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    rtmp_port, rtmp_sock = reserve_ephemeral_port()
    rtmp_sock.close()

    input_path = Path(__file__).resolve().parents[2] / "testdata" / "red-320x320-h264-aac.mp4"
    rtmp_url = f"rtmp://127.0.0.1:{rtmp_port}/live"
    stream_name = "inbound"
    rtmp_push_url = f"{rtmp_url}/{stream_name}"

    async def _run():
        timeout = aiohttp.ClientTimeout(total=30.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-rtmp-inbound",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "rtmp-inbound-input",
                    "inputKind": "rtmp_inbound",
                    "inputSettings": {
                        "inputUrl": rtmp_url,
                        "streamName": stream_name,
                    },
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            start_record_response = await _send_obsws_request(
                ws,
                request_type="StartRecord",
                request_id="req-start-record-rtmp-inbound",
            )
            assert start_record_response["d"]["requestStatus"]["result"] is True

            # ffmpeg RTMP push をバックグラウンドで開始する（無限ループ）
            loop = asyncio.get_event_loop()
            ffmpeg_process = await loop.run_in_executor(
                None,
                lambda: _start_ffmpeg_inbound_push(input_path, rtmp_push_url, "flv"),
            )
            metrics_snapshots: dict[str, str] = {}
            try:
                # mp4_writer に映像サンプルが書き込まれるまで待機する
                for _ in range(30):
                    status, body, _ = await _http_get(
                        f"http://{host}:{ws_port}/metrics"
                    )
                    if status == 200 and _has_positive_metric(
                        body,
                        'hisui_total_video_sample_count{processor_id="output:record:mp4_writer:0"',
                    ):
                        break
                    await asyncio.sleep(0.2)
                else:
                    raise AssertionError(
                        "record did not write video samples in time for rtmp_inbound"
                    )

                await asyncio.sleep(0.5)
                metrics_snapshots["before_stop_record"] = await _collect_obsws_metrics_snapshot_async(
                    host,
                    ws_port,
                )

                stop_record_response = await _send_obsws_request(
                    ws,
                    request_type="StopRecord",
                    request_id="req-stop-record-rtmp-inbound",
                )
                assert stop_record_response["d"]["requestStatus"]["result"] is True
                output_path = Path(stop_record_response["d"]["responseData"]["outputPath"])
                metrics_snapshots["after_stop_record"] = await _collect_obsws_metrics_snapshot_async(
                    host,
                    ws_port,
                )
            finally:
                ffmpeg_process.kill()
                ffmpeg_process.communicate(timeout=5)

            await ws.close()
            return output_path, metrics_snapshots

    with ObswsServer(
        binary_path,
        host=host,
        port=ws_port,
        default_record_dir=tmp_path,
        use_env=False,
    ):
        output_path, metrics_snapshots = asyncio.run(_run())

    assert output_path.exists()
    assert output_path.stat().st_size > 0
    inspect_output = _inspect_mp4(
        binary_path,
        output_path,
        required_keys=("video_codec", "video_sample_count"),
        diagnostics_text=_format_obsws_diagnostics(
            metrics_snapshots=metrics_snapshots,
        ),
    )
    assert inspect_output["format"] == "mp4"
    assert inspect_output["video_codec"] == "H264"
    assert inspect_output["video_sample_count"] > 0


def test_obsws_srt_inbound_start_record_and_inspect_output(
    binary_path: Path,
    tmp_path: Path,
):
    """obsws で srt_inbound を作成し StartRecord → ffmpeg SRT push → StopRecord で録画できることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    srt_port, srt_sock = reserve_ephemeral_port()
    srt_sock.close()

    input_path = Path(__file__).resolve().parents[2] / "testdata" / "red-320x320-h264-aac.mp4"
    srt_url = f"srt://127.0.0.1:{srt_port}"

    async def _run():
        timeout = aiohttp.ClientTimeout(total=30.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-srt-inbound",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "srt-inbound-input",
                    "inputKind": "srt_inbound",
                    "inputSettings": {"inputUrl": srt_url},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            start_record_response = await _send_obsws_request(
                ws,
                request_type="StartRecord",
                request_id="req-start-record-srt-inbound",
            )
            assert start_record_response["d"]["requestStatus"]["result"] is True

            # ffmpeg SRT push をバックグラウンドで開始する（無限ループ）
            loop = asyncio.get_event_loop()
            ffmpeg_process = await loop.run_in_executor(
                None,
                lambda: _start_ffmpeg_inbound_push(input_path, srt_url, "mpegts"),
            )
            metrics_snapshots: dict[str, str] = {}
            try:
                # mp4_writer に映像サンプルが書き込まれるまで待機する
                for _ in range(30):
                    status, body, _ = await _http_get(
                        f"http://{host}:{ws_port}/metrics"
                    )
                    if status == 200 and _has_positive_metric(
                        body,
                        'hisui_total_video_sample_count{processor_id="output:record:mp4_writer:0"',
                    ):
                        break
                    await asyncio.sleep(0.2)
                else:
                    raise AssertionError(
                        "record did not write video samples in time for srt_inbound"
                    )

                await asyncio.sleep(0.5)
                metrics_snapshots["before_stop_record"] = await _collect_obsws_metrics_snapshot_async(
                    host,
                    ws_port,
                )

                stop_record_response = await _send_obsws_request(
                    ws,
                    request_type="StopRecord",
                    request_id="req-stop-record-srt-inbound",
                )
                assert stop_record_response["d"]["requestStatus"]["result"] is True
                output_path = Path(stop_record_response["d"]["responseData"]["outputPath"])
                metrics_snapshots["after_stop_record"] = await _collect_obsws_metrics_snapshot_async(
                    host,
                    ws_port,
                )
            finally:
                ffmpeg_process.kill()
                ffmpeg_process.communicate(timeout=5)

            await ws.close()
            return output_path, metrics_snapshots

    with ObswsServer(
        binary_path,
        host=host,
        port=ws_port,
        default_record_dir=tmp_path,
        use_env=False,
    ):
        output_path, metrics_snapshots = asyncio.run(_run())

    assert output_path.exists()
    assert output_path.stat().st_size > 0
    inspect_output = _inspect_mp4(
        binary_path,
        output_path,
        required_keys=("video_codec", "video_sample_count"),
        diagnostics_text=_format_obsws_diagnostics(
            metrics_snapshots=metrics_snapshots,
        ),
    )
    assert inspect_output["format"] == "mp4"
    assert inspect_output["video_codec"] == "H264"
    assert inspect_output["video_sample_count"] > 0


def test_obsws_srt_inbound_with_stream_id(
    binary_path: Path,
    tmp_path: Path,
):
    """obsws で srt_inbound に streamId を指定して録画できることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    srt_port, srt_sock = reserve_ephemeral_port()
    srt_sock.close()

    input_path = Path(__file__).resolve().parents[2] / "testdata" / "red-320x320-h264-aac.mp4"
    stream_id = "test-stream-id"
    srt_listen_url = f"srt://127.0.0.1:{srt_port}"
    srt_push_url = f"srt://127.0.0.1:{srt_port}?streamid={stream_id}"

    async def _run():
        timeout = aiohttp.ClientTimeout(total=30.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-srt-inbound-sid",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "srt-inbound-with-sid",
                    "inputKind": "srt_inbound",
                    "inputSettings": {
                        "inputUrl": srt_listen_url,
                        "streamId": stream_id,
                    },
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            start_record_response = await _send_obsws_request(
                ws,
                request_type="StartRecord",
                request_id="req-start-record-srt-inbound-sid",
            )
            assert start_record_response["d"]["requestStatus"]["result"] is True

            # ffmpeg SRT push をバックグラウンドで開始する（無限ループ）
            loop = asyncio.get_event_loop()
            ffmpeg_process = await loop.run_in_executor(
                None,
                lambda: _start_ffmpeg_inbound_push(input_path, srt_push_url, "mpegts"),
            )
            metrics_snapshots: dict[str, str] = {}
            try:
                # mp4_writer に映像サンプルが書き込まれるまで待機する
                for _ in range(30):
                    status, body, _ = await _http_get(
                        f"http://{host}:{ws_port}/metrics"
                    )
                    if status == 200 and _has_positive_metric(
                        body,
                        'hisui_total_video_sample_count{processor_id="output:record:mp4_writer:0"',
                    ):
                        break
                    await asyncio.sleep(0.2)
                else:
                    raise AssertionError(
                        "record did not write video samples in time for srt_inbound with stream_id"
                    )

                await asyncio.sleep(0.5)
                metrics_snapshots["before_stop_record"] = await _collect_obsws_metrics_snapshot_async(
                    host,
                    ws_port,
                )

                stop_record_response = await _send_obsws_request(
                    ws,
                    request_type="StopRecord",
                    request_id="req-stop-record-srt-inbound-sid",
                )
                assert stop_record_response["d"]["requestStatus"]["result"] is True
                output_path = Path(stop_record_response["d"]["responseData"]["outputPath"])
                metrics_snapshots["after_stop_record"] = await _collect_obsws_metrics_snapshot_async(
                    host,
                    ws_port,
                )
            finally:
                ffmpeg_process.kill()
                ffmpeg_process.communicate(timeout=5)

            await ws.close()
            return output_path, metrics_snapshots

    with ObswsServer(
        binary_path,
        host=host,
        port=ws_port,
        default_record_dir=tmp_path,
        use_env=False,
    ):
        output_path, metrics_snapshots = asyncio.run(_run())

    assert output_path.exists()
    assert output_path.stat().st_size > 0
    inspect_output = _inspect_mp4(
        binary_path,
        output_path,
        required_keys=("video_codec", "video_sample_count"),
        diagnostics_text=_format_obsws_diagnostics(
            metrics_snapshots=metrics_snapshots,
        ),
    )
    assert inspect_output["format"] == "mp4"
    assert inspect_output["video_codec"] == "H264"
    assert inspect_output["video_sample_count"] > 0


def test_obsws_rtmp_inbound_start_stream_to_rtmp(
    binary_path: Path,
    tmp_path: Path,
):
    """obsws で rtmp_inbound を作成し StartStream で RTMP 配信できることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    rtmp_inbound_port, rtmp_inbound_sock = reserve_ephemeral_port()
    rtmp_inbound_sock.close()
    rtmp_outbound_port, rtmp_outbound_sock = reserve_ephemeral_port()
    rtmp_outbound_sock.close()

    input_path = Path(__file__).resolve().parents[2] / "testdata" / "red-320x320-h264-aac.mp4"
    output_path = tmp_path / "received-rtmp-inbound-stream.mp4"
    rtmp_inbound_url = f"rtmp://127.0.0.1:{rtmp_inbound_port}/live"
    rtmp_inbound_stream_name = "inbound"
    rtmp_inbound_push_url = f"{rtmp_inbound_url}/{rtmp_inbound_stream_name}"
    output_url = f"rtmp://127.0.0.1:{rtmp_outbound_port}/live"
    stream_key = "obsws-stream"
    receive_url = f"{output_url}/{stream_key}"

    async def _run_start_stream_flow():
        timeout = aiohttp.ClientTimeout(total=30.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            # rtmp_inbound 入力を作成する
            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-rtmp-inbound-stream",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "rtmp-inbound-stream-input",
                    "inputKind": "rtmp_inbound",
                    "inputSettings": {
                        "inputUrl": rtmp_inbound_url,
                        "streamName": rtmp_inbound_stream_name,
                    },
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            # 配信先の RTMP サービス設定を行う
            set_stream_service_response = await _send_obsws_request(
                ws,
                request_type="SetStreamServiceSettings",
                request_id="req-set-stream-service-rtmp-inbound",
                request_data={
                    "streamServiceType": "rtmp_custom",
                    "streamServiceSettings": {
                        "server": output_url,
                        "key": stream_key,
                    },
                },
            )
            assert set_stream_service_response["d"]["requestStatus"]["result"] is True

            # 配信を開始する
            start_stream_response = await _send_obsws_request(
                ws,
                request_type="StartStream",
                request_id="req-start-stream-rtmp-inbound",
            )
            assert start_stream_response["d"]["requestStatus"]["result"] is True

            for _ in range(20):
                stream_status_response = await _send_obsws_request(
                    ws,
                    request_type="GetStreamStatus",
                    request_id="req-get-stream-status-rtmp-inbound",
                )
                if stream_status_response["d"]["responseData"]["outputActive"] is True:
                    break
                await asyncio.sleep(0.1)
            else:
                raise AssertionError("stream did not become active in time")

            # ffmpeg で RTMP inbound 側にメディアを push する
            loop = asyncio.get_event_loop()
            await loop.run_in_executor(
                None,
                lambda: _run_ffmpeg_rtmp_push(input_path, rtmp_inbound_push_url),
            )

            await asyncio.sleep(1.0)

            # 配信を停止する
            stop_stream_response = await _send_obsws_request(
                ws,
                request_type="StopStream",
                request_id="req-stop-stream-rtmp-inbound",
            )
            assert stop_stream_response["d"]["requestStatus"]["result"] is True

            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False) as server:

        def _run_start_stream_flow_sync() -> None:
            asyncio.run(_run_start_stream_flow())

        # 受信側が先に接続待機へ入れるよう、StartStream フローは別スレッドで並行実行する
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            ffmpeg_process = _start_ffmpeg_rtmp_receive(
                receive_url,
                output_path,
                with_audio=True,
                max_video_frames=None,
                listen=True,
                timeout_seconds=30,
            )
            try:
                time.sleep(RTMP_LISTEN_RECEIVER_STARTUP_WAIT_SEC)
                start_stream_future = executor.submit(_run_start_stream_flow_sync)
                start_stream_future.result(timeout=40.0)
                _wait_process_exit(ffmpeg_process, timeout=20.0)
            except Exception as e:
                metrics_snapshot = _collect_obsws_metrics_snapshot(
                    server.host,
                    server.port,
                )
                raise AssertionError(
                    f"obsws rtmp_inbound start_stream test failed: {e}\nmetrics_snapshot:\n{metrics_snapshot}"
                ) from e
            finally:
                if ffmpeg_process.poll() is None:
                    ffmpeg_process.kill()
                    ffmpeg_process.communicate(timeout=5)

    assert output_path.exists(), "RTMP received output file must exist"
    assert output_path.stat().st_size > 0, "RTMP received output file must not be empty"
    inspect_output = _inspect_mp4(binary_path, output_path)
    assert inspect_output["format"] == "mp4"
    assert inspect_output["video_codec"] == "H264"
    assert inspect_output["video_sample_count"] > 0


def test_obsws_srt_inbound_start_stream_to_rtmp(
    binary_path: Path,
    tmp_path: Path,
):
    """obsws で srt_inbound を作成し StartStream で RTMP 配信できることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    srt_inbound_port, srt_inbound_sock = reserve_ephemeral_port()
    srt_inbound_sock.close()
    rtmp_outbound_port, rtmp_outbound_sock = reserve_ephemeral_port()
    rtmp_outbound_sock.close()

    input_path = Path(__file__).resolve().parents[2] / "testdata" / "red-320x320-h264-aac.mp4"
    output_path = tmp_path / "received-srt-inbound-stream.mp4"
    srt_inbound_url = f"srt://127.0.0.1:{srt_inbound_port}"
    output_url = f"rtmp://127.0.0.1:{rtmp_outbound_port}/live"
    stream_key = "obsws-stream"
    receive_url = f"{output_url}/{stream_key}"

    async def _run_start_stream_flow():
        timeout = aiohttp.ClientTimeout(total=30.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            # srt_inbound 入力を作成する
            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-srt-inbound-stream",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "srt-inbound-stream-input",
                    "inputKind": "srt_inbound",
                    "inputSettings": {"inputUrl": srt_inbound_url},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            # 配信先の RTMP サービス設定を行う
            set_stream_service_response = await _send_obsws_request(
                ws,
                request_type="SetStreamServiceSettings",
                request_id="req-set-stream-service-srt-inbound",
                request_data={
                    "streamServiceType": "rtmp_custom",
                    "streamServiceSettings": {
                        "server": output_url,
                        "key": stream_key,
                    },
                },
            )
            assert set_stream_service_response["d"]["requestStatus"]["result"] is True

            # 配信を開始する
            start_stream_response = await _send_obsws_request(
                ws,
                request_type="StartStream",
                request_id="req-start-stream-srt-inbound",
            )
            assert start_stream_response["d"]["requestStatus"]["result"] is True

            for _ in range(20):
                stream_status_response = await _send_obsws_request(
                    ws,
                    request_type="GetStreamStatus",
                    request_id="req-get-stream-status-srt-inbound",
                )
                if stream_status_response["d"]["responseData"]["outputActive"] is True:
                    break
                await asyncio.sleep(0.1)
            else:
                raise AssertionError("stream did not become active in time")

            # ffmpeg で SRT inbound 側にメディアを push する
            loop = asyncio.get_event_loop()
            await loop.run_in_executor(
                None,
                lambda: _run_ffmpeg_srt_push(input_path, srt_inbound_url),
            )

            await asyncio.sleep(1.0)

            # 配信を停止する
            stop_stream_response = await _send_obsws_request(
                ws,
                request_type="StopStream",
                request_id="req-stop-stream-srt-inbound",
            )
            assert stop_stream_response["d"]["requestStatus"]["result"] is True

            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False) as server:

        def _run_start_stream_flow_sync() -> None:
            asyncio.run(_run_start_stream_flow())

        # 受信側が先に接続待機へ入れるよう、StartStream フローは別スレッドで並行実行する
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            ffmpeg_process = _start_ffmpeg_rtmp_receive(
                receive_url,
                output_path,
                with_audio=True,
                max_video_frames=None,
                listen=True,
                timeout_seconds=30,
            )
            try:
                time.sleep(RTMP_LISTEN_RECEIVER_STARTUP_WAIT_SEC)
                start_stream_future = executor.submit(_run_start_stream_flow_sync)
                start_stream_future.result(timeout=40.0)
                _wait_process_exit(ffmpeg_process, timeout=20.0)
            except Exception as e:
                metrics_snapshot = _collect_obsws_metrics_snapshot(
                    server.host,
                    server.port,
                )
                raise AssertionError(
                    f"obsws srt_inbound start_stream test failed: {e}\nmetrics_snapshot:\n{metrics_snapshot}"
                ) from e
            finally:
                if ffmpeg_process.poll() is None:
                    ffmpeg_process.kill()
                    ffmpeg_process.communicate(timeout=5)

    assert output_path.exists(), "RTMP received output file must exist"
    assert output_path.stat().st_size > 0, "RTMP received output file must not be empty"
    inspect_output = _inspect_mp4(binary_path, output_path)
    assert inspect_output["format"] == "mp4"
    assert inspect_output["video_codec"] == "H264"
    assert inspect_output["video_sample_count"] > 0


def test_obsws_hls_start_stop_output(binary_path: Path, tmp_path: Path):
    """obsws が StartOutput/StopOutput で HLS 出力を開始・停止できることを確認する。
    停止後に生成ファイルが削除されることも確認する。"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    image_path = tmp_path / "hls-input.png"
    _write_test_png(image_path)
    hls_dir = tmp_path / "hls-output"
    hls_dir.mkdir()

    async def _run_hls_flow():
        timeout = aiohttp.ClientTimeout(total=30.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            # 入力ソースを作成
            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-hls-input",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "hls-input",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            # GetOutputList に hls が含まれることを確認
            output_list_response = await _send_obsws_request(
                ws,
                request_type="GetOutputList",
                request_id="req-get-output-list-hls",
            )
            assert output_list_response["d"]["requestStatus"]["result"] is True
            outputs = output_list_response["d"]["responseData"]["outputs"]
            hls_output = [o for o in outputs if o["outputName"] == "hls"]
            assert len(hls_output) == 1
            assert hls_output[0]["outputKind"] == "hls_output"

            # HLS 設定を行う
            set_settings_response = await _send_obsws_request(
                ws,
                request_type="SetOutputSettings",
                request_id="req-set-hls-settings",
                request_data={
                    "outputName": "hls",
                    "outputSettings": {
                        "destination": {"type": "filesystem", "directory": str(hls_dir)},
                    },
                },
            )
            assert set_settings_response["d"]["requestStatus"]["result"] is True

            # 設定を取得して確認
            get_settings_response = await _send_obsws_request(
                ws,
                request_type="GetOutputSettings",
                request_id="req-get-hls-settings",
                request_data={"outputName": "hls"},
            )
            assert get_settings_response["d"]["requestStatus"]["result"] is True
            settings = get_settings_response["d"]["responseData"]["outputSettings"]
            assert settings["destination"]["type"] == "filesystem"
            assert settings["destination"]["directory"] == str(hls_dir)

            # HLS 出力を開始
            start_response = await _send_obsws_request(
                ws,
                request_type="StartOutput",
                request_id="req-start-hls",
                request_data={"outputName": "hls"},
            )
            assert start_response["d"]["requestStatus"]["result"] is True

            # アクティブになることを確認
            for _ in range(20):
                status_response = await _send_obsws_request(
                    ws,
                    request_type="GetOutputStatus",
                    request_id="req-get-hls-status-active",
                    request_data={"outputName": "hls"},
                )
                if status_response["d"]["responseData"]["outputActive"] is True:
                    break
                await asyncio.sleep(0.1)
            else:
                raise AssertionError("HLS did not become active after StartOutput")

            # セグメントが生成されるまでポーリングで待つ
            playlist_path = hls_dir / "playlist.m3u8"
            for _ in range(100):
                if playlist_path.exists() and list(hls_dir.glob("segment-*.ts")):
                    break
                await asyncio.sleep(0.2)
            else:
                raise AssertionError(
                    f"playlist.m3u8 or .ts segments were not generated within timeout. "
                    f"Files in {hls_dir}: {list(hls_dir.iterdir())}"
                )

            # playlist.m3u8 が存在することを確認
            assert playlist_path.exists(), "playlist.m3u8 must exist"

            # .ts セグメントファイルが存在することを確認
            ts_files = list(hls_dir.glob("segment-*.ts"))
            assert len(ts_files) > 0, "at least one .ts segment must exist"

            # 二重起動がエラーになることを確認
            double_start_response = await _send_obsws_request(
                ws,
                request_type="StartOutput",
                request_id="req-start-hls-double",
                request_data={"outputName": "hls"},
            )
            assert double_start_response["d"]["requestStatus"]["result"] is False

            # HLS 出力を停止
            stop_response = await _send_obsws_request(
                ws,
                request_type="StopOutput",
                request_id="req-stop-hls",
                request_data={"outputName": "hls"},
            )
            assert stop_response["d"]["requestStatus"]["result"] is True

            # 非アクティブになることを確認
            for _ in range(20):
                status_response = await _send_obsws_request(
                    ws,
                    request_type="GetOutputStatus",
                    request_id="req-get-hls-status-inactive",
                    request_data={"outputName": "hls"},
                )
                if status_response["d"]["responseData"]["outputActive"] is False:
                    break
                await asyncio.sleep(0.1)
            else:
                raise AssertionError("HLS did not become inactive after StopOutput")

            # 停止後にファイルが削除されていることを確認
            assert not playlist_path.exists(), "playlist.m3u8 must be deleted after stop"
            ts_files_after = list(hls_dir.glob("segment-*.ts"))
            assert (
                len(ts_files_after) == 0
            ), "all .ts segments must be deleted after stop"

            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run_hls_flow())


def test_obsws_hls_toggle_output(binary_path: Path, tmp_path: Path):
    """obsws が ToggleOutput で HLS 出力を on/off できることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    image_path = tmp_path / "hls-toggle-input.png"
    _write_test_png(image_path)
    hls_dir = tmp_path / "hls-toggle-output"
    hls_dir.mkdir()

    async def _run_hls_toggle_flow():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            # 入力ソースを作成
            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-hls-toggle-input",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "hls-toggle-input",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            # HLS 設定
            set_settings_response = await _send_obsws_request(
                ws,
                request_type="SetOutputSettings",
                request_id="req-set-hls-toggle-settings",
                request_data={
                    "outputName": "hls",
                    "outputSettings": {"destination": {"type": "filesystem", "directory": str(hls_dir)}},
                },
            )
            assert set_settings_response["d"]["requestStatus"]["result"] is True

            # ToggleOutput で開始
            toggle_start_response = await _send_obsws_request(
                ws,
                request_type="ToggleOutput",
                request_id="req-toggle-hls-start",
                request_data={"outputName": "hls"},
            )
            assert toggle_start_response["d"]["requestStatus"]["result"] is True
            assert (
                toggle_start_response["d"]["responseData"]["outputActive"] is True
            )

            # アクティブ確認
            for _ in range(20):
                status_response = await _send_obsws_request(
                    ws,
                    request_type="GetOutputStatus",
                    request_id="req-get-hls-toggle-status-on",
                    request_data={"outputName": "hls"},
                )
                if status_response["d"]["responseData"]["outputActive"] is True:
                    break
                await asyncio.sleep(0.1)

            # ToggleOutput で停止
            toggle_stop_response = await _send_obsws_request(
                ws,
                request_type="ToggleOutput",
                request_id="req-toggle-hls-stop",
                request_data={"outputName": "hls"},
            )
            assert toggle_stop_response["d"]["requestStatus"]["result"] is True
            assert (
                toggle_stop_response["d"]["responseData"]["outputActive"] is False
            )

            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run_hls_toggle_flow())


def test_obsws_hls_start_without_directory_fails(binary_path: Path, tmp_path: Path):
    """destination 未設定で HLS StartOutput がエラーになることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    image_path = tmp_path / "hls-nodir-input.png"
    _write_test_png(image_path)

    async def _run_hls_nodir_flow():
        timeout = aiohttp.ClientTimeout(total=10.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            # 入力ソースを作成
            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-hls-nodir-input",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "hls-nodir-input",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            # destination を設定せずに StartOutput
            start_response = await _send_obsws_request(
                ws,
                request_type="StartOutput",
                request_id="req-start-hls-nodir",
                request_data={"outputName": "hls"},
            )
            assert start_response["d"]["requestStatus"]["result"] is False

            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run_hls_nodir_flow())


def test_obsws_hls_fmp4_start_stop_output(binary_path: Path, tmp_path: Path):
    """obsws が fMP4 形式の HLS 出力を開始・停止できることを確認する。
    init.mp4 と .m4s セグメントが生成され、停止後に削除されることを確認する。"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    image_path = tmp_path / "hls-fmp4-input.png"
    _write_test_png(image_path)
    hls_dir = tmp_path / "hls-fmp4-output"
    hls_dir.mkdir()

    async def _run_hls_fmp4_flow():
        timeout = aiohttp.ClientTimeout(total=30.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            # 入力ソースを作成
            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-hls-fmp4-input",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "hls-fmp4-input",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            # fMP4 形式で HLS 設定を行う
            set_settings_response = await _send_obsws_request(
                ws,
                request_type="SetOutputSettings",
                request_id="req-set-hls-fmp4-settings",
                request_data={
                    "outputName": "hls",
                    "outputSettings": {
                        "destination": {"type": "filesystem", "directory": str(hls_dir)},
                        "segmentFormat": "fmp4",
                    },
                },
            )
            assert set_settings_response["d"]["requestStatus"]["result"] is True

            # 設定を取得して segmentFormat が fmp4 であることを確認
            get_settings_response = await _send_obsws_request(
                ws,
                request_type="GetOutputSettings",
                request_id="req-get-hls-fmp4-settings",
                request_data={"outputName": "hls"},
            )
            assert get_settings_response["d"]["requestStatus"]["result"] is True
            settings = get_settings_response["d"]["responseData"]["outputSettings"]
            assert settings["segmentFormat"] == "fmp4"

            # HLS 出力を開始
            start_response = await _send_obsws_request(
                ws,
                request_type="StartOutput",
                request_id="req-start-hls-fmp4",
                request_data={"outputName": "hls"},
            )
            assert start_response["d"]["requestStatus"]["result"] is True

            # セグメントが生成されるまでポーリングで待つ
            init_path = hls_dir / "init.mp4"
            playlist_path = hls_dir / "playlist.m3u8"
            for _ in range(100):
                if (
                    init_path.exists()
                    and playlist_path.exists()
                    and list(hls_dir.glob("segment-*.m4s"))
                ):
                    break
                await asyncio.sleep(0.2)
            else:
                raise AssertionError(
                    f"fMP4 HLS files were not generated within timeout. "
                    f"Files in {hls_dir}: {list(hls_dir.iterdir())}"
                )

            # init.mp4 が存在することを確認
            assert init_path.exists(), "init.mp4 must exist for fMP4 HLS"

            # playlist.m3u8 が存在することを確認
            assert playlist_path.exists(), "playlist.m3u8 must exist"

            # playlist に EXT-X-MAP が含まれることを確認
            playlist_content = playlist_path.read_text()
            assert '#EXT-X-MAP:URI="init.mp4"' in playlist_content, (
                "playlist must contain EXT-X-MAP for fMP4"
            )
            assert "#EXT-X-VERSION:7" in playlist_content, (
                "playlist must use version 7 for fMP4"
            )

            # .m4s セグメントファイルが存在することを確認
            m4s_files = list(hls_dir.glob("segment-*.m4s"))
            assert len(m4s_files) > 0, "at least one .m4s segment must exist"

            # HLS 出力を停止
            stop_response = await _send_obsws_request(
                ws,
                request_type="StopOutput",
                request_id="req-stop-hls-fmp4",
                request_data={"outputName": "hls"},
            )
            assert stop_response["d"]["requestStatus"]["result"] is True

            # 停止を待つ
            for _ in range(20):
                status_response = await _send_obsws_request(
                    ws,
                    request_type="GetOutputStatus",
                    request_id="req-get-hls-fmp4-status-inactive",
                    request_data={"outputName": "hls"},
                )
                if status_response["d"]["responseData"]["outputActive"] is False:
                    break
                await asyncio.sleep(0.1)

            # 停止後にファイルが削除されていることを確認
            assert not init_path.exists(), "init.mp4 must be deleted after stop"
            assert not playlist_path.exists(), "playlist.m3u8 must be deleted after stop"
            m4s_files_after = list(hls_dir.glob("segment-*.m4s"))
            assert (
                len(m4s_files_after) == 0
            ), "all .m4s segments must be deleted after stop"

            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run_hls_fmp4_flow())


def test_obsws_hls_abr_start_stop_output(binary_path: Path, tmp_path: Path):
    """ABR 設定での HLS 出力を開始・停止できることを確認する。
    マスタープレイリストとバリアントサブディレクトリが生成・削除されることを確認する。"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    image_path = tmp_path / "hls-abr-input.png"
    _write_test_png(image_path)
    hls_dir = tmp_path / "hls-abr-output"
    hls_dir.mkdir()

    async def _run_hls_abr_flow():
        timeout = aiohttp.ClientTimeout(total=30.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            # 入力ソースを作成
            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-hls-abr-input",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "hls-abr-input",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            # ABR 設定（2 バリアント）を行う
            set_settings_response = await _send_obsws_request(
                ws,
                request_type="SetOutputSettings",
                request_id="req-set-hls-abr-settings",
                request_data={
                    "outputName": "hls",
                    "outputSettings": {
                        "destination": {"type": "filesystem", "directory": str(hls_dir)},
                        "variants": [
                            {"videoBitrate": 2000000, "audioBitrate": 128000},
                            {"videoBitrate": 800000, "audioBitrate": 96000},
                        ],
                    },
                },
            )
            assert set_settings_response["d"]["requestStatus"]["result"] is True

            # 設定を取得して variants が反映されていることを確認
            get_settings_response = await _send_obsws_request(
                ws,
                request_type="GetOutputSettings",
                request_id="req-get-hls-abr-settings",
                request_data={"outputName": "hls"},
            )
            assert get_settings_response["d"]["requestStatus"]["result"] is True
            settings = get_settings_response["d"]["responseData"]["outputSettings"]
            assert len(settings["variants"]) == 2
            assert settings["variants"][0]["videoBitrate"] == 2000000
            assert settings["variants"][1]["videoBitrate"] == 800000

            # HLS 出力を開始
            start_response = await _send_obsws_request(
                ws,
                request_type="StartOutput",
                request_id="req-start-hls-abr",
                request_data={"outputName": "hls"},
            )
            assert start_response["d"]["requestStatus"]["result"] is True

            # マスタープレイリストとバリアントディレクトリが生成されるまで待つ
            master_playlist_path = hls_dir / "playlist.m3u8"
            variant_0_dir = hls_dir / "variant_0"
            variant_1_dir = hls_dir / "variant_1"
            variant_0_playlist = variant_0_dir / "playlist.m3u8"
            for _ in range(100):
                if (
                    master_playlist_path.exists()
                    and variant_0_dir.exists()
                    and variant_1_dir.exists()
                    and variant_0_playlist.exists()
                    and list(variant_0_dir.glob("segment-*.ts"))
                ):
                    break
                await asyncio.sleep(0.2)
            else:
                files = list(hls_dir.rglob("*"))
                raise AssertionError(
                    f"ABR HLS files were not generated within timeout. "
                    f"Files: {files}"
                )

            # マスタープレイリストに EXT-X-STREAM-INF が含まれることを確認
            master_content = master_playlist_path.read_text()
            assert "#EXT-X-STREAM-INF" in master_content, (
                "master playlist must contain EXT-X-STREAM-INF"
            )
            assert "variant_0/playlist.m3u8" in master_content
            assert "variant_1/playlist.m3u8" in master_content

            # 各バリアントディレクトリにセグメントが存在することを確認
            assert list(variant_0_dir.glob("segment-*.ts")), (
                "variant_0 must have .ts segments"
            )
            assert list(variant_1_dir.glob("segment-*.ts")), (
                "variant_1 must have .ts segments"
            )

            # HLS 出力を停止
            stop_response = await _send_obsws_request(
                ws,
                request_type="StopOutput",
                request_id="req-stop-hls-abr",
                request_data={"outputName": "hls"},
            )
            assert stop_response["d"]["requestStatus"]["result"] is True

            # 停止を待つ
            for _ in range(20):
                status_response = await _send_obsws_request(
                    ws,
                    request_type="GetOutputStatus",
                    request_id="req-get-hls-abr-status-inactive",
                    request_data={"outputName": "hls"},
                )
                if status_response["d"]["responseData"]["outputActive"] is False:
                    break
                await asyncio.sleep(0.1)

            # 停止後にファイルが削除されていることを確認
            assert not master_playlist_path.exists(), (
                "master playlist must be deleted after stop"
            )
            # バリアントディレクトリ内のファイルも削除されていることを確認
            assert not list(variant_0_dir.glob("*")), (
                "variant_0 files must be deleted after stop"
            )
            assert not list(variant_1_dir.glob("*")), (
                "variant_1 files must be deleted after stop"
            )

            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run_hls_abr_flow())


def test_obsws_hls_variants_validation(binary_path: Path, tmp_path: Path):
    """HLS variants のバリデーションエラーを確認する。"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    async def _run_validation_flow():
        timeout = aiohttp.ClientTimeout(total=10.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            # 空の variants 配列はエラー
            resp = await _send_obsws_request(
                ws,
                request_type="SetOutputSettings",
                request_id="req-hls-empty-variants",
                request_data={
                    "outputName": "hls",
                    "outputSettings": {"variants": []},
                },
            )
            assert resp["d"]["requestStatus"]["result"] is False

            # videoBitrate が 0 はエラー
            resp = await _send_obsws_request(
                ws,
                request_type="SetOutputSettings",
                request_id="req-hls-zero-video-bitrate",
                request_data={
                    "outputName": "hls",
                    "outputSettings": {
                        "variants": [{"videoBitrate": 0, "audioBitrate": 128000}],
                    },
                },
            )
            assert resp["d"]["requestStatus"]["result"] is False

            # width のみ指定（height なし）はエラー
            resp = await _send_obsws_request(
                ws,
                request_type="SetOutputSettings",
                request_id="req-hls-width-only",
                request_data={
                    "outputName": "hls",
                    "outputSettings": {
                        "variants": [
                            {
                                "videoBitrate": 2000000,
                                "audioBitrate": 128000,
                                "width": 1280,
                            }
                        ],
                    },
                },
            )
            assert resp["d"]["requestStatus"]["result"] is False

            # 奇数 width はエラー
            resp = await _send_obsws_request(
                ws,
                request_type="SetOutputSettings",
                request_id="req-hls-odd-width",
                request_data={
                    "outputName": "hls",
                    "outputSettings": {
                        "variants": [
                            {
                                "videoBitrate": 2000000,
                                "audioBitrate": 128000,
                                "width": 1281,
                                "height": 720,
                            }
                        ],
                    },
                },
            )
            assert resp["d"]["requestStatus"]["result"] is False

            # 正常な variants は成功
            resp = await _send_obsws_request(
                ws,
                request_type="SetOutputSettings",
                request_id="req-hls-valid-variants",
                request_data={
                    "outputName": "hls",
                    "outputSettings": {
                        "variants": [
                            {
                                "videoBitrate": 2000000,
                                "audioBitrate": 128000,
                                "width": 1280,
                                "height": 720,
                            }
                        ],
                    },
                },
            )
            assert resp["d"]["requestStatus"]["result"] is True

            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run_validation_flow())
