"""obsws e2e テストの共有ヘルパー・定数・補助クラス"""

import asyncio
import base64
import hashlib
import json
import os
import shutil
import signal
import socket
import subprocess
import time
from pathlib import Path

import aiohttp
import pytest

OBSWS_SUBPROTOCOL = "obswebsocket.json"
OBSWS_EVENT_SUB_SCENES = 1 << 2
OBSWS_EVENT_SUB_INPUTS = 1 << 3
OBSWS_EVENT_SUB_OUTPUTS = 1 << 6
OBSWS_EVENT_SUB_SCENE_ITEMS = 1 << 7


class ObswsServer:
    """obsws サブコマンドプロセスを管理するテスト補助クラス"""

    def __init__(
        self,
        binary_path: Path,
        *,
        host: str,
        port: int,
        password: str | None = None,
        default_record_dir: Path | None = None,
        use_env: bool = False,
    ):
        self.binary_path = binary_path
        self.host = host
        self.port = port
        self.password = password
        self.default_record_dir = default_record_dir
        self.use_env = use_env
        self._process: subprocess.Popen[None] | None = None

    def __enter__(self):
        return self.start()

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.stop()

    def start(self):
        if self._process is not None:
            raise RuntimeError("obsws server is already started")

        cmd = [str(self.binary_path), "--verbose", "--experimental", "obsws"]
        env = os.environ.copy()
        openh264_path = env.get("HISUI_OPENH264_PATH")
        if self.use_env:
            env["HISUI_OBSWS_HOST"] = self.host
            env["HISUI_OBSWS_PORT"] = str(self.port)
            if self.password is not None:
                env["HISUI_OBSWS_PASSWORD"] = self.password
            if self.default_record_dir is not None:
                env["HISUI_DEFAULT_RECORD_DIR"] = str(self.default_record_dir)
        else:
            cmd.extend(
                [
                    "--host",
                    self.host,
                    "--port",
                    str(self.port),
                ]
            )
            if self.password is not None:
                cmd.extend(["--password", self.password])
            if self.default_record_dir is not None:
                cmd.extend(["--default-record-dir", str(self.default_record_dir)])
            if openh264_path:
                cmd.extend(["--openh264", openh264_path])

        self._process = subprocess.Popen(cmd, env=env)
        self._wait_until_listening()
        return self

    def stop(self):
        process = self._process
        if process is None:
            return
        if process.poll() is None:
            process.send_signal(signal.SIGTERM)
            try:
                process.wait(timeout=5.0)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=3.0)
        self._process = None

    def _wait_until_listening(self, timeout: float = 10.0):
        deadline = time.time() + timeout
        while time.time() < deadline:
            process = self._process
            if process is not None and process.poll() is not None:
                raise AssertionError(
                    f"obsws process exited before listening: returncode={process.returncode}"
                )
            if _is_port_open(self.host, self.port):
                return
            time.sleep(0.1)
        raise AssertionError(
            f"obsws server did not start listening in time: {self.host}:{self.port}"
        )


def _is_port_open(host: str, port: int) -> bool:
    try:
        with socket.create_connection((host, port), timeout=0.5):
            return True
    except OSError:
        return False


def _start_ffmpeg_rtmp_receive(
    receive_url: str,
    output_path: Path,
    *,
    with_audio: bool,
    max_video_frames: int | None,
    listen: bool = False,
    timeout_seconds: int | None = None,
    startup_timeout: float = 10.0,
) -> subprocess.Popen[str]:
    ffmpeg_path = shutil.which("ffmpeg")
    if ffmpeg_path is None:
        pytest.skip("ffmpeg is required for obsws RTMP stream test")

    cmd = [
        ffmpeg_path,
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
    ]
    if listen:
        cmd.extend(["-listen", "1"])
    if timeout_seconds is not None:
        cmd.extend(["-timeout", str(timeout_seconds)])
    cmd.extend(["-i", receive_url])
    if max_video_frames is not None:
        cmd.extend(["-frames:v", str(max_video_frames)])
    if not with_audio:
        cmd.append("-an")
    cmd.extend(
        [
            "-c",
            "copy",
            "-f",
            "mp4",
            str(output_path),
        ]
    )

    if listen:
        return subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

    deadline = time.time() + startup_timeout
    last_stderr = ""
    while time.time() < deadline:
        process = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        time.sleep(0.2)
        if process.poll() is None:
            return process
        stdout, stderr = process.communicate(timeout=5)
        last_stderr = f"stdout={stdout}, stderr={stderr}"
        time.sleep(0.1)

    raise AssertionError(
        f"failed to start ffmpeg receiver within timeout: url={receive_url}, details={last_stderr}"
    )


def _wait_process_exit(
    process: subprocess.Popen[str], timeout: float
) -> tuple[str, str]:
    try:
        return_code = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired as e:
        process.kill()
        stdout, stderr = process.communicate(timeout=5)
        raise AssertionError(
            f"process timed out: timeout={timeout}, stdout={stdout}, stderr={stderr}"
        ) from e

    stdout, stderr = process.communicate(timeout=5)
    assert return_code == 0, (
        f"process exited with non-zero code: returncode={return_code}, stdout={stdout}, stderr={stderr}"
    )
    return stdout, stderr


def _inspect_mp4(binary_path: Path, path: Path) -> dict[str, object]:
    result = subprocess.run(
        [str(binary_path), "inspect", str(path)],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, (
        f"hisui inspect failed: returncode={result.returncode}, stderr={result.stderr}"
    )
    output = json.loads(result.stdout)
    assert isinstance(output, dict), "inspect output must be a JSON object"
    return output


def _write_test_png(path: Path) -> None:
    # 16x16 PNG（赤）の固定データ
    path.write_bytes(
        base64.b64decode(
            "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAGUlEQVR42mP4z8DwnxLMMGrAqAGjBgwXAwAwxP4QisZM5QAAAABJRU5ErkJggg=="
        )
    )


async def _connect_websocket(url: str):
    timeout = aiohttp.ClientTimeout(total=10.0)
    async with aiohttp.ClientSession(timeout=timeout) as session:
        ws = await session.ws_connect(url, protocols=[OBSWS_SUBPROTOCOL])
        await ws.close()


async def _http_get(url: str):
    timeout = aiohttp.ClientTimeout(total=10.0)
    async with aiohttp.ClientSession(timeout=timeout) as session:
        async with session.get(url) as response:
            return response.status, await response.text(), response.headers


def _collect_obsws_metrics_snapshot(host: str, port: int) -> str:
    endpoint = "/metrics"
    url = f"http://{host}:{port}{endpoint}"
    try:
        status, body, _ = asyncio.run(_http_get(url))
        return f"[{endpoint}] status={status}\n{body}"
    except Exception as e:
        return f"[{endpoint}] failed to fetch metrics: {e}"


async def _connect_and_exchange_identify(url: str):
    timeout = aiohttp.ClientTimeout(total=10.0)
    async with aiohttp.ClientSession(timeout=timeout) as session:
        ws = await session.ws_connect(url, protocols=[OBSWS_SUBPROTOCOL])

        hello_msg = await ws.receive(timeout=5.0)
        assert hello_msg.type == aiohttp.WSMsgType.TEXT
        hello = json.loads(hello_msg.data)
        assert hello["op"] == 0
        hello_data = hello["d"]
        assert hello_data["rpcVersion"] == 1

        await ws.send_str(json.dumps({"op": 1, "d": {"rpcVersion": 1}}))
        identified_msg = await ws.receive(timeout=5.0)
        assert identified_msg.type == aiohttp.WSMsgType.TEXT
        identified = json.loads(identified_msg.data)
        assert identified["op"] == 2
        assert identified["d"]["negotiatedRpcVersion"] == 1

        await ws.close()


async def _identify_with_optional_password(
    ws: aiohttp.ClientWebSocketResponse,
    password: str | None,
    event_subscriptions: int | None = None,
):
    hello_msg = await ws.receive(timeout=5.0)
    assert hello_msg.type == aiohttp.WSMsgType.TEXT
    hello = json.loads(hello_msg.data)
    assert hello["op"] == 0
    hello_data = hello["d"]
    assert hello_data["rpcVersion"] == 1

    identify_data: dict[str, object] = {"rpcVersion": 1}
    # OBS WebSocket プロトコルでは eventSubscriptions 省略時のデフォルトが All のため、
    # イベントを購読しないテストでは明示的に 0 を送信する。
    identify_data["eventSubscriptions"] = event_subscriptions if event_subscriptions is not None else 0
    if password is not None:
        authentication = hello_data["authentication"]
        identify_data["authentication"] = _build_obsws_authentication(
            password=password,
            salt=authentication["salt"],
            challenge=authentication["challenge"],
        )

    await ws.send_str(json.dumps({"op": 1, "d": identify_data}))
    identified_msg = await ws.receive(timeout=5.0)
    assert identified_msg.type == aiohttp.WSMsgType.TEXT
    identified = json.loads(identified_msg.data)
    assert identified["op"] == 2
    assert identified["d"]["negotiatedRpcVersion"] == 1


async def _send_obsws_request(
    ws: aiohttp.ClientWebSocketResponse,
    request_type: str,
    request_id: str,
    request_data: dict[str, object] | None = None,
):
    data: dict[str, object] = {
        "requestType": request_type,
        "requestId": request_id,
    }
    if request_data is not None:
        data["requestData"] = request_data

    await ws.send_str(json.dumps({"op": 6, "d": data}))
    response_msg = await ws.receive(timeout=5.0)
    assert response_msg.type == aiohttp.WSMsgType.TEXT
    response = json.loads(response_msg.data)
    assert response["op"] == 7
    assert response["d"]["requestType"] == request_type
    assert response["d"]["requestId"] == request_id
    return response


async def _send_obsws_request_batch(
    ws: aiohttp.ClientWebSocketResponse,
    *,
    request_id: str,
    requests: list[dict[str, object]],
    halt_on_failure: bool = False,
    execution_type: int = -1,
):
    data: dict[str, object] = {
        "requestId": request_id,
        "haltOnFailure": halt_on_failure,
        "executionType": execution_type,
        "requests": requests,
    }
    await ws.send_str(json.dumps({"op": 8, "d": data}))
    response_msg = await ws.receive(timeout=5.0)
    assert response_msg.type == aiohttp.WSMsgType.TEXT
    response = json.loads(response_msg.data)
    assert response["op"] == 9
    assert response["d"]["requestId"] == request_id
    return response


async def _expect_stream_state_changed_event(
    ws: aiohttp.ClientWebSocketResponse,
    *,
    output_active: bool,
):
    # hisui は中間状態イベント (STARTING/STOPPING) を最終状態イベント (STARTED/STOPPED) の
    # 前に送信するため、中間状態を消費してから最終状態を検証する。
    intermediate_state = (
        "OBS_WEBSOCKET_OUTPUT_STARTING" if output_active else "OBS_WEBSOCKET_OUTPUT_STOPPING"
    )
    intermediate_event = await _expect_obsws_event(
        ws,
        event_type="StreamStateChanged",
        event_intent=OBSWS_EVENT_SUB_OUTPUTS,
    )
    assert intermediate_event["d"]["eventData"]["outputState"] == intermediate_state

    event = await _expect_obsws_event(
        ws,
        event_type="StreamStateChanged",
        event_intent=OBSWS_EVENT_SUB_OUTPUTS,
    )
    assert event["d"]["eventData"]["outputActive"] is output_active
    expected_output_state = (
        "OBS_WEBSOCKET_OUTPUT_STARTED" if output_active else "OBS_WEBSOCKET_OUTPUT_STOPPED"
    )
    assert event["d"]["eventData"]["outputState"] == expected_output_state


async def _expect_record_state_changed_event(
    ws: aiohttp.ClientWebSocketResponse,
    *,
    output_active: bool,
    output_state: str | None = None,
):
    event = await _expect_obsws_event(
        ws,
        event_type="RecordStateChanged",
        event_intent=OBSWS_EVENT_SUB_OUTPUTS,
    )
    assert event["d"]["eventData"]["outputActive"] is output_active
    if output_state is not None:
        assert event["d"]["eventData"]["outputState"] == output_state
    # outputPath は常に存在する（録画中でなければ null）
    assert "outputPath" in event["d"]["eventData"]
    return event


async def _expect_input_settings_changed_event(
    ws: aiohttp.ClientWebSocketResponse,
    *,
    input_name: str,
):
    event = await _expect_obsws_event(
        ws,
        event_type="InputSettingsChanged",
        event_intent=OBSWS_EVENT_SUB_INPUTS,
    )
    assert event["d"]["eventData"]["inputName"] == input_name
    return event


async def _expect_input_name_changed_event(
    ws: aiohttp.ClientWebSocketResponse,
    *,
    input_name: str,
    old_input_name: str,
):
    event = await _expect_obsws_event(
        ws,
        event_type="InputNameChanged",
        event_intent=OBSWS_EVENT_SUB_INPUTS,
    )
    assert event["d"]["eventData"]["inputName"] == input_name
    assert event["d"]["eventData"]["oldInputName"] == old_input_name
    return event


async def _expect_scene_item_enable_state_changed_event(
    ws: aiohttp.ClientWebSocketResponse,
    *,
    scene_name: str,
    scene_item_id: int,
    scene_item_enabled: bool,
):
    event = await _expect_obsws_event(
        ws,
        event_type="SceneItemEnableStateChanged",
        event_intent=OBSWS_EVENT_SUB_SCENE_ITEMS,
    )
    assert event["d"]["eventData"]["sceneName"] == scene_name
    assert isinstance(event["d"]["eventData"]["sceneUuid"], str)
    assert event["d"]["eventData"]["sceneUuid"] != ""
    assert event["d"]["eventData"]["sceneItemId"] == scene_item_id
    assert event["d"]["eventData"]["sceneItemEnabled"] is scene_item_enabled
    return event


async def _expect_scene_item_lock_state_changed_event(
    ws: aiohttp.ClientWebSocketResponse,
    *,
    scene_name: str,
    scene_item_id: int,
    scene_item_locked: bool,
):
    event = await _expect_obsws_event(
        ws,
        event_type="SceneItemLockStateChanged",
        event_intent=OBSWS_EVENT_SUB_SCENE_ITEMS,
    )
    assert event["d"]["eventData"]["sceneName"] == scene_name
    assert isinstance(event["d"]["eventData"]["sceneUuid"], str)
    assert event["d"]["eventData"]["sceneUuid"] != ""
    assert event["d"]["eventData"]["sceneItemId"] == scene_item_id
    assert event["d"]["eventData"]["sceneItemLocked"] is scene_item_locked
    return event


async def _expect_scene_item_transform_changed_event(
    ws: aiohttp.ClientWebSocketResponse,
    *,
    scene_name: str,
    scene_item_id: int,
):
    event = await _expect_obsws_event(
        ws,
        event_type="SceneItemTransformChanged",
        event_intent=OBSWS_EVENT_SUB_SCENE_ITEMS,
    )
    assert event["d"]["eventData"]["sceneName"] == scene_name
    assert isinstance(event["d"]["eventData"]["sceneUuid"], str)
    assert event["d"]["eventData"]["sceneUuid"] != ""
    assert event["d"]["eventData"]["sceneItemId"] == scene_item_id
    return event


async def _expect_obsws_event(
    ws: aiohttp.ClientWebSocketResponse,
    *,
    event_type: str,
    event_intent: int,
):
    event_msg = await ws.receive(timeout=5.0)
    assert event_msg.type == aiohttp.WSMsgType.TEXT
    event = json.loads(event_msg.data)
    assert event["op"] == 5
    event_data = event["d"]
    assert event_data["eventType"] == event_type
    assert event_data["eventIntent"] == event_intent
    return event


async def _assert_no_message_within(
    ws: aiohttp.ClientWebSocketResponse,
    *,
    timeout: float,
):
    with pytest.raises(asyncio.TimeoutError):
        await ws.receive(timeout=timeout)


def _build_obsws_authentication(password: str, salt: str, challenge: str) -> str:
    secret = base64.b64encode(
        hashlib.sha256(f"{password}{salt}".encode("utf-8")).digest()
    ).decode("utf-8")
    return base64.b64encode(
        hashlib.sha256(f"{secret}{challenge}".encode("utf-8")).digest()
    ).decode("utf-8")


async def _connect_and_exchange_identify_with_password(url: str, password: str):
    timeout = aiohttp.ClientTimeout(total=10.0)
    async with aiohttp.ClientSession(timeout=timeout) as session:
        ws = await session.ws_connect(url, protocols=[OBSWS_SUBPROTOCOL])
        await _identify_with_optional_password(ws, password)
        await ws.close()


async def _connect_and_send_invalid_password_auth(url: str):
    timeout = aiohttp.ClientTimeout(total=10.0)
    async with aiohttp.ClientSession(timeout=timeout) as session:
        ws = await session.ws_connect(url, protocols=[OBSWS_SUBPROTOCOL])

        hello_msg = await ws.receive(timeout=5.0)
        assert hello_msg.type == aiohttp.WSMsgType.TEXT
        hello = json.loads(hello_msg.data)
        assert hello["op"] == 0
        assert "authentication" in hello["d"]

        await ws.send_str(
            json.dumps(
                {
                    "op": 1,
                    "d": {
                        "rpcVersion": 1,
                        "authentication": "invalid-authentication",
                    },
                }
            )
        )
        close_msg = await ws.receive(timeout=5.0)
        assert close_msg.type in {
            aiohttp.WSMsgType.CLOSE,
            aiohttp.WSMsgType.CLOSING,
            aiohttp.WSMsgType.CLOSED,
        }
        assert ws.close_code == 4009
        await ws.close()


async def _connect_identify_and_request(
    url: str,
    request_type: str,
    request_id: str,
    *,
    request_data: dict[str, object] | None = None,
    password: str | None = None,
):
    timeout = aiohttp.ClientTimeout(total=10.0)
    async with aiohttp.ClientSession(timeout=timeout) as session:
        ws = await session.ws_connect(url, protocols=[OBSWS_SUBPROTOCOL])
        await _identify_with_optional_password(ws, password)
        response = await _send_obsws_request(
            ws,
            request_type=request_type,
            request_id=request_id,
            request_data=request_data,
        )
        await ws.close()
        return response


async def _connect_and_send_missing_password_auth(url: str):
    timeout = aiohttp.ClientTimeout(total=10.0)
    async with aiohttp.ClientSession(timeout=timeout) as session:
        ws = await session.ws_connect(url, protocols=[OBSWS_SUBPROTOCOL])

        hello_msg = await ws.receive(timeout=5.0)
        assert hello_msg.type == aiohttp.WSMsgType.TEXT
        hello = json.loads(hello_msg.data)
        assert hello["op"] == 0
        assert "authentication" in hello["d"]

        await ws.send_str(json.dumps({"op": 1, "d": {"rpcVersion": 1}}))
        close_msg = await ws.receive(timeout=5.0)
        assert close_msg.type in {
            aiohttp.WSMsgType.CLOSE,
            aiohttp.WSMsgType.CLOSING,
            aiohttp.WSMsgType.CLOSED,
        }
        assert ws.close_code == 4009
        await ws.close()


async def _connect_and_expect_close_code(
    url: str,
    message: dict[str, object],
    expected_close_code: int,
):
    timeout = aiohttp.ClientTimeout(total=10.0)
    async with aiohttp.ClientSession(timeout=timeout) as session:
        ws = await session.ws_connect(url, protocols=[OBSWS_SUBPROTOCOL])
        hello_msg = await ws.receive(timeout=5.0)
        assert hello_msg.type == aiohttp.WSMsgType.TEXT
        await ws.send_str(json.dumps(message))
        close_msg = await ws.receive(timeout=5.0)
        assert close_msg.type in {
            aiohttp.WSMsgType.CLOSE,
            aiohttp.WSMsgType.CLOSING,
            aiohttp.WSMsgType.CLOSED,
        }
        assert ws.close_code == expected_close_code
        await ws.close()


async def _connect_and_send_duplicate_identify(url: str):
    timeout = aiohttp.ClientTimeout(total=10.0)
    async with aiohttp.ClientSession(timeout=timeout) as session:
        ws = await session.ws_connect(url, protocols=[OBSWS_SUBPROTOCOL])
        await _identify_with_optional_password(ws, None)
        await ws.send_str(json.dumps({"op": 1, "d": {"rpcVersion": 1}}))
        close_msg = await ws.receive(timeout=5.0)
        assert close_msg.type in {
            aiohttp.WSMsgType.CLOSE,
            aiohttp.WSMsgType.CLOSING,
            aiohttp.WSMsgType.CLOSED,
        }
        assert ws.close_code == 4008
        await ws.close()


async def _connect_identify_and_expect_close_code(
    url: str,
    message: dict[str, object],
    expected_close_code: int,
):
    timeout = aiohttp.ClientTimeout(total=10.0)
    async with aiohttp.ClientSession(timeout=timeout) as session:
        ws = await session.ws_connect(url, protocols=[OBSWS_SUBPROTOCOL])
        await _identify_with_optional_password(ws, None)
        await ws.send_str(json.dumps(message))
        close_msg = await ws.receive(timeout=5.0)
        assert close_msg.type in {
            aiohttp.WSMsgType.CLOSE,
            aiohttp.WSMsgType.CLOSING,
            aiohttp.WSMsgType.CLOSED,
        }
        assert ws.close_code == expected_close_code
        await ws.close()


async def _setup_stream_input_and_service(
    ws: aiohttp.ClientWebSocketResponse,
    *,
    image_path: Path,
    output_url: str,
    stream_key: str,
):
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
    set_stream_service_status = set_stream_service_response["d"]["requestStatus"]
    assert set_stream_service_status["result"] is True


async def _connect_identify_and_send_reidentify_then_request(url: str):
    timeout = aiohttp.ClientTimeout(total=10.0)
    async with aiohttp.ClientSession(timeout=timeout) as session:
        ws = await session.ws_connect(url, protocols=[OBSWS_SUBPROTOCOL])
        await _identify_with_optional_password(ws, None)
        await ws.send_str(json.dumps({"op": 3, "d": {"eventSubscriptions": 1023}}))
        reidentified_msg = await ws.receive(timeout=5.0)
        assert reidentified_msg.type == aiohttp.WSMsgType.TEXT
        reidentified = json.loads(reidentified_msg.data)
        assert reidentified["op"] == 2
        assert reidentified["d"]["negotiatedRpcVersion"] == 1
        response = await _send_obsws_request(
            ws,
            request_type="GetVersion",
            request_id="req-after-reidentify",
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is True
        await ws.close()
