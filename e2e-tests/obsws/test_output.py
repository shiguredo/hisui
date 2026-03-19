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
    _format_obsws_diagnostics,
    _http_get,
    _identify_with_optional_password,
    _inspect_mp4,
    _print_obsws_diagnostics,
    _run_ffmpeg_rtmp_push,
    _run_ffmpeg_srt_push,
    _send_obsws_request,
    _start_ffmpeg_inbound_push,
    _start_ffmpeg_rtmp_receive,
    _wait_process_exit,
    _write_test_png,
)
from hisui_server import reserve_ephemeral_port


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


def test_obsws_pause_resume_record_request(binary_path: Path, tmp_path: Path):
    """obsws が PauseRecord / ResumeRecord request を処理できることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    image_path = tmp_path / "pause-resume-record-input.png"
    _write_test_png(image_path)

    async def _run_pause_resume_record_flow(server: ObswsServer):
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
                request_id="req-create-pause-resume-record-input",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "pause-resume-record-input",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            start_record_response = await _send_obsws_request(
                ws,
                request_type="StartRecord",
                request_id="req-start-record-for-pause-resume",
            )
            assert start_record_response["d"]["requestStatus"]["result"] is True

            pause_record_response = await _send_obsws_request(
                ws,
                request_type="PauseRecord",
                request_id="req-pause-record",
            )
            assert pause_record_response["d"]["requestStatus"]["result"] is True

            for _ in range(20):
                record_status_response = await _send_obsws_request(
                    ws,
                    request_type="GetRecordStatus",
                    request_id="req-get-record-status-paused",
                )
                if record_status_response["d"]["responseData"]["outputPaused"] is True:
                    break
                await asyncio.sleep(0.1)
            else:
                raise AssertionError("record did not become paused after PauseRecord")

            resume_record_response = await _send_obsws_request(
                ws,
                request_type="ResumeRecord",
                request_id="req-resume-record",
            )
            assert resume_record_response["d"]["requestStatus"]["result"] is True

            for _ in range(20):
                record_status_response = await _send_obsws_request(
                    ws,
                    request_type="GetRecordStatus",
                    request_id="req-get-record-status-resumed",
                )
                if record_status_response["d"]["responseData"]["outputPaused"] is False:
                    break
                await asyncio.sleep(0.1)
            else:
                raise AssertionError("record did not resume after ResumeRecord")

            await asyncio.sleep(0.3)
            status, body, _ = await _http_get(
                f"http://{server.host}:{server.port}/metrics"
            )
            assert status == 200
            assert "hisui_total_keyframe_wait_dropped_audio_sample_count" in body
            assert "hisui_total_keyframe_wait_dropped_video_frame_count" in body

            stop_record_response = await _send_obsws_request(
                ws,
                request_type="StopRecord",
                request_id="req-stop-record-after-pause-resume",
            )
            assert stop_record_response["d"]["requestStatus"]["result"] is True
            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False) as server:
        asyncio.run(_run_pause_resume_record_flow(server))


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
                    'hisui_total_audio_sample_count{processor_id="obsws:record:0:mp4_writer"',
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
                time.sleep(0.5)
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
                time.sleep(0.5)
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
                time.sleep(0.5)
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
                        'hisui_total_video_sample_count{processor_id="obsws:record:0:mp4_writer"',
                    ):
                        break
                    await asyncio.sleep(0.2)
                else:
                    raise AssertionError(
                        "record did not write video samples in time for rtmp_inbound"
                    )

                await asyncio.sleep(0.5)
                metrics_snapshots["before_stop_record"] = _collect_obsws_metrics_snapshot(
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
                metrics_snapshots["after_stop_record"] = _collect_obsws_metrics_snapshot(
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
    _print_obsws_diagnostics(
        inspect_output=inspect_output,
        metrics_snapshots=metrics_snapshots,
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
                        'hisui_total_video_sample_count{processor_id="obsws:record:0:mp4_writer"',
                    ):
                        break
                    await asyncio.sleep(0.2)
                else:
                    raise AssertionError(
                        "record did not write video samples in time for srt_inbound"
                    )

                await asyncio.sleep(0.5)
                metrics_snapshots["before_stop_record"] = _collect_obsws_metrics_snapshot(
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
                metrics_snapshots["after_stop_record"] = _collect_obsws_metrics_snapshot(
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
    _print_obsws_diagnostics(
        inspect_output=inspect_output,
        metrics_snapshots=metrics_snapshots,
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
                        'hisui_total_video_sample_count{processor_id="obsws:record:0:mp4_writer"',
                    ):
                        break
                    await asyncio.sleep(0.2)
                else:
                    raise AssertionError(
                        "record did not write video samples in time for srt_inbound with stream_id"
                    )

                await asyncio.sleep(0.5)
                metrics_snapshots["before_stop_record"] = _collect_obsws_metrics_snapshot(
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
                metrics_snapshots["after_stop_record"] = _collect_obsws_metrics_snapshot(
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
    _print_obsws_diagnostics(
        inspect_output=inspect_output,
        metrics_snapshots=metrics_snapshots,
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
                time.sleep(0.5)
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
                time.sleep(0.5)
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
