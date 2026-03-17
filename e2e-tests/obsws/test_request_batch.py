"""obsws の RequestBatch に関する e2e テスト"""

import asyncio
import json
from pathlib import Path

import aiohttp

from helpers import (
    OBSWS_SUBPROTOCOL,
    ObswsServer,
    _identify_with_optional_password,
    _send_obsws_request,
    _send_obsws_request_batch,
    _write_test_png,
)
from hisui_server import reserve_ephemeral_port


def test_obsws_request_batch_prepares_stream_flow(binary_path: Path, tmp_path: Path):
    """obsws が RequestBatch で配信準備 request を順次実行できることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    rtmp_port, rtmp_sock = reserve_ephemeral_port()
    rtmp_sock.close()

    image_path = tmp_path / "batch-input.png"
    _write_test_png(image_path)

    async def _run_batch_flow():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            batch_response = await _send_obsws_request_batch(
                ws,
                request_id="batch-prepare-stream",
                halt_on_failure=True,
                execution_type=-1,
                requests=[
                    {
                        "requestType": "CreateScene",
                        "requestData": {"sceneName": "BatchScene"},
                    },
                    {
                        "requestType": "CreateInput",
                        "requestData": {
                            "sceneName": "BatchScene",
                            "inputName": "batch-image-input",
                            "inputKind": "image_source",
                            "inputSettings": {"file": str(image_path)},
                            "sceneItemEnabled": True,
                        },
                    },
                    {
                        "requestType": "SetCurrentProgramScene",
                        "requestData": {"sceneName": "BatchScene"},
                    },
                    {
                        "requestType": "SetStreamServiceSettings",
                        "requestData": {
                            "streamServiceType": "rtmp_custom",
                            "streamServiceSettings": {
                                "server": f"rtmp://127.0.0.1:{rtmp_port}/live",
                                "key": "batch-stream-key",
                            },
                        },
                    },
                ],
            )
            results = batch_response["d"]["results"]
            assert len(results) == 4
            for result in results:
                assert result["requestStatus"]["result"] is True

            start_stream_response = await _send_obsws_request(
                ws,
                request_type="StartStream",
                request_id="req-start-stream-after-batch",
            )
            assert start_stream_response["d"]["requestStatus"]["result"] is True

            stop_stream_response = await _send_obsws_request(
                ws,
                request_type="StopStream",
                request_id="req-stop-stream-after-batch",
            )
            assert stop_stream_response["d"]["requestStatus"]["result"] is True
            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False):
        asyncio.run(_run_batch_flow())


def test_obsws_request_batch_applies_set_input_settings(binary_path: Path):
    """obsws が RequestBatch で SetInputSettings を順次適用できることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    async def _run_batch_set_input_settings_flow():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            batch_response = await _send_obsws_request_batch(
                ws,
                request_id="batch-set-input-settings",
                halt_on_failure=True,
                execution_type=-1,
                requests=[
                    {
                        "requestType": "CreateInput",
                        "requestData": {
                            "sceneName": "Scene",
                            "inputName": "batch-set-input-settings-input",
                            "inputKind": "video_capture_device",
                            "inputSettings": {"device_id": "before-device"},
                            "sceneItemEnabled": True,
                        },
                    },
                    {
                        "requestType": "SetInputSettings",
                        "requestData": {
                            "inputName": "batch-set-input-settings-input",
                            "inputSettings": {"device_id": "after-device"},
                        },
                    },
                    {
                        "requestType": "GetInputSettings",
                        "requestData": {
                            "inputName": "batch-set-input-settings-input",
                        },
                    },
                ],
            )
            results = batch_response["d"]["results"]
            assert len(results) == 3
            assert results[0]["requestType"] == "CreateInput"
            assert results[0]["requestStatus"]["result"] is True
            assert results[1]["requestType"] == "SetInputSettings"
            assert results[1]["requestStatus"]["result"] is True
            assert results[2]["requestType"] == "GetInputSettings"
            assert results[2]["requestStatus"]["result"] is True
            assert (
                results[2]["responseData"]["inputSettings"]["device_id"] == "after-device"
            )
            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run_batch_set_input_settings_flow())


def test_obsws_request_batch_halt_on_failure_stops_after_set_input_settings_error(
    binary_path: Path,
):
    """obsws が RequestBatch の SetInputSettings エラーで後続 request を停止することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    async def _run_batch_set_input_settings_halt_flow():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            batch_response = await _send_obsws_request_batch(
                ws,
                request_id="batch-set-input-settings-halt",
                halt_on_failure=True,
                execution_type=-1,
                requests=[
                    {
                        "requestType": "CreateInput",
                        "requestData": {
                            "sceneName": "Scene",
                            "inputName": "batch-set-input-settings-halt-input",
                            "inputKind": "video_capture_device",
                            "inputSettings": {"device_id": "before-device"},
                            "sceneItemEnabled": True,
                        },
                    },
                    {
                        "requestType": "SetInputSettings",
                        "requestData": {
                            "inputName": "batch-set-input-settings-halt-input",
                            "inputSettings": {},
                            "overlay": "invalid",
                        },
                    },
                    {
                        "requestType": "GetInputSettings",
                        "requestData": {
                            "inputName": "batch-set-input-settings-halt-input",
                        },
                    },
                ],
            )
            results = batch_response["d"]["results"]
            assert len(results) == 2
            assert results[0]["requestType"] == "CreateInput"
            assert results[0]["requestStatus"]["result"] is True
            assert results[1]["requestType"] == "SetInputSettings"
            assert results[1]["requestStatus"]["result"] is False
            assert results[1]["requestStatus"]["code"] == 400

            get_input_settings_response = await _send_obsws_request(
                ws,
                request_type="GetInputSettings",
                request_id="req-get-input-settings-after-batch-halt",
                request_data={"inputName": "batch-set-input-settings-halt-input"},
            )
            assert get_input_settings_response["d"]["requestStatus"]["result"] is True
            assert (
                get_input_settings_response["d"]["responseData"]["inputSettings"][
                    "device_id"
                ]
                == "before-device"
            )
            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run_batch_set_input_settings_halt_flow())


def test_obsws_request_batch_rejects_unsupported_execution_type(binary_path: Path):
    """obsws が未対応 executionType の RequestBatch を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    async def _run_invalid_batch_flow():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            await ws.send_str(
                json.dumps(
                    {
                        "op": 8,
                        "d": {
                            "requestId": "batch-invalid-execution-type",
                            "executionType": 0,
                            "requests": [{"requestType": "GetVersion"}],
                        },
                    }
                )
            )
            response_msg = await ws.receive(timeout=5.0)
            assert response_msg.type == aiohttp.WSMsgType.TEXT
            response = json.loads(response_msg.data)
            assert response["op"] == 7
            assert response["d"]["requestType"] == "RequestBatch"
            status = response["d"]["requestStatus"]
            assert status["result"] is False
            assert status["code"] == 400
            assert status["comment"] == "Unsupported executionType field"
            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run_invalid_batch_flow())


def test_obsws_request_batch_halt_on_failure_stops_subsequent_requests(
    binary_path: Path,
):
    """obsws が RequestBatch の haltOnFailure で後続 request を停止することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    async def _run_halt_on_failure_flow():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)

            current_scene_response = await _send_obsws_request(
                ws,
                request_type="GetCurrentProgramScene",
                request_id="req-current-scene-before-batch",
            )
            assert current_scene_response["d"]["requestStatus"]["result"] is True
            current_scene_name = current_scene_response["d"]["responseData"][
                "currentProgramSceneName"
            ]

            batch_response = await _send_obsws_request_batch(
                ws,
                request_id="batch-halt-on-failure",
                halt_on_failure=True,
                execution_type=-1,
                requests=[
                    {
                        "requestType": "CreateScene",
                        "requestData": {"sceneName": "BatchHaltScene"},
                    },
                    {
                        "requestType": "CreateScene",
                        "requestData": {"sceneName": "BatchHaltScene"},
                    },
                    {
                        "requestType": "SetCurrentProgramScene",
                        "requestData": {"sceneName": "BatchHaltScene"},
                    },
                ],
            )
            results = batch_response["d"]["results"]
            assert len(results) == 2
            assert results[0]["requestType"] == "CreateScene"
            assert results[0]["requestStatus"]["result"] is True
            assert results[1]["requestType"] == "CreateScene"
            assert results[1]["requestStatus"]["result"] is False

            current_scene_after_response = await _send_obsws_request(
                ws,
                request_type="GetCurrentProgramScene",
                request_id="req-current-scene-after-batch",
            )
            assert current_scene_after_response["d"]["requestStatus"]["result"] is True
            assert (
                current_scene_after_response["d"]["responseData"][
                    "currentProgramSceneName"
                ]
                == current_scene_name
            )
            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run_halt_on_failure_flow())
