"""obsws サブコマンドの e2e テスト"""

import asyncio
import base64
import concurrent.futures
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

from hisui_server import reserve_ephemeral_port

OBSWS_SUBPROTOCOL = "obswebsocket.json"
OBSWS_EVENT_SUB_SCENES = 1 << 2
OBSWS_EVENT_SUB_INPUTS = 1 << 3
OBSWS_EVENT_SUB_OUTPUTS = 1 << 6


class ObswsServer:
    """obsws サブコマンドプロセスを管理するテスト補助クラス"""

    def __init__(
        self,
        binary_path: Path,
        *,
        host: str,
        port: int,
        http_host: str | None = None,
        http_port: int | None = None,
        password: str | None = None,
        default_record_dir: Path | None = None,
        use_env: bool = False,
    ):
        self.binary_path = binary_path
        self.host = host
        self.port = port
        self.http_host = http_host or host
        if http_port is None:
            reserved_http_port, reserved_http_sock = reserve_ephemeral_port()
            reserved_http_sock.close()
            self.http_port = reserved_http_port
        else:
            self.http_port = http_port
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
        if self.use_env:
            env["HISUI_OBSWS_HOST"] = self.host
            env["HISUI_OBSWS_PORT"] = str(self.port)
            env["HISUI_OBSWS_HTTP_LISTEN_ADDRESS"] = self.http_host
            env["HISUI_OBSWS_HTTP_PORT"] = str(self.http_port)
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
                    "--http-listen-address",
                    self.http_host,
                    "--http-port",
                    str(self.http_port),
                ]
            )
            if self.password is not None:
                cmd.extend(["--password", self.password])
            if self.default_record_dir is not None:
                cmd.extend(["--default-record-dir", str(self.default_record_dir)])

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
            ws_ready = _is_port_open(self.host, self.port)
            http_ready = _is_port_open(self.http_host, self.http_port)
            if ws_ready and http_ready:
                return
            time.sleep(0.1)
        raise AssertionError(
            "obsws server did not start listening in time: "
            f"ws={self.host}:{self.port}, http={self.http_host}:{self.http_port}"
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
    max_video_frames: int | None,
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
    cmd.extend(["-i", receive_url])
    if max_video_frames is not None:
        cmd.extend(["-frames:v", str(max_video_frames)])
    cmd.extend(
        [
            "-an",
            "-c",
            "copy",
            "-f",
            "mp4",
            str(output_path),
        ]
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


def _collect_obsws_metrics_snapshot(http_host: str, http_port: int) -> str:
    endpoint = "/metrics"
    url = f"http://{http_host}:{http_port}{endpoint}"
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
    if event_subscriptions is not None:
        identify_data["eventSubscriptions"] = event_subscriptions
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
    execution_type: int = 0,
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
    event = await _expect_obsws_event(
        ws,
        event_type="StreamStateChanged",
        event_intent=OBSWS_EVENT_SUB_OUTPUTS,
    )
    assert event["d"]["eventData"]["outputActive"] is output_active


async def _expect_record_state_changed_event(
    ws: aiohttp.ClientWebSocketResponse,
    *,
    output_active: bool,
    output_paused: bool | None = None,
):
    event = await _expect_obsws_event(
        ws,
        event_type="RecordStateChanged",
        event_intent=OBSWS_EVENT_SUB_OUTPUTS,
    )
    assert event["d"]["eventData"]["outputActive"] is output_active
    if output_paused is not None:
        assert event["d"]["eventData"]["outputPaused"] is output_paused
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
        event_intent=OBSWS_EVENT_SUB_SCENES,
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
        event_intent=OBSWS_EVENT_SUB_SCENES,
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
        event_intent=OBSWS_EVENT_SUB_SCENES,
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


def test_obsws_hello_and_identify_flow(binary_path: Path):
    """obsws が Hello / Identify / Identified を処理できることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        asyncio.run(_connect_and_exchange_identify(f"ws://{host}:{port}/"))


def test_obsws_accepts_websocket_connection_with_env_vars(binary_path: Path):
    """obsws が環境変数指定でも websocket 接続を受け付けることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=True,
    ):
        asyncio.run(_connect_websocket(f"ws://{host}:{port}/"))


def test_obsws_http_ok_endpoint(binary_path: Path):
    """obsws が HTTP /.ok エンドポイントを公開することを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    http_port, http_sock = reserve_ephemeral_port()
    http_sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=ws_port,
        http_port=http_port,
        use_env=False,
    ) as server:
        status, _, _ = asyncio.run(
            _http_get(f"http://{server.http_host}:{server.http_port}/.ok")
        )
        assert status == 204


def test_obsws_http_metrics_endpoint(binary_path: Path):
    """obsws が HTTP /metrics エンドポイントを公開することを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    http_port, http_sock = reserve_ephemeral_port()
    http_sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=ws_port,
        http_port=http_port,
        use_env=False,
    ) as server:
        status, body, headers = asyncio.run(
            _http_get(f"http://{server.http_host}:{server.http_port}/metrics")
        )
        assert status == 200
        assert headers.get("Content-Type") == "text/plain; version=0.0.4; charset=utf-8"
        assert "# TYPE hisui_tokio_num_workers gauge" in body


def test_obsws_http_metrics_json_endpoint(binary_path: Path):
    """obsws が HTTP /metrics?format=json を返すことを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    http_port, http_sock = reserve_ephemeral_port()
    http_sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=ws_port,
        http_port=http_port,
        use_env=False,
    ) as server:
        status, body, headers = asyncio.run(
            _http_get(
                f"http://{server.http_host}:{server.http_port}/metrics?format=json"
            )
        )
        assert status == 200
        assert headers.get("Content-Type") == "application/json; charset=utf-8"
        assert '"name":"hisui_tokio_num_workers"' in body


def test_obsws_rejects_connection_without_subprotocol(binary_path: Path):
    """obsws が必須 subprotocol なしの接続を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    async def _connect_without_subprotocol(url: str):
        timeout = aiohttp.ClientTimeout(total=10.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            with pytest.raises(aiohttp.WSServerHandshakeError):
                await session.ws_connect(url)

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        asyncio.run(_connect_without_subprotocol(f"ws://{host}:{port}/"))


def test_obsws_accepts_authenticated_connection(binary_path: Path):
    """obsws が password 指定時に認証成功で接続継続することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        password="test-password",
        use_env=False,
    ):
        asyncio.run(
            _connect_and_exchange_identify_with_password(
                f"ws://{host}:{port}/",
                "test-password",
            )
        )


def test_obsws_rejects_authenticated_connection_with_invalid_auth(binary_path: Path):
    """obsws が password 指定時に認証失敗を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        password="test-password",
        use_env=False,
    ):
        asyncio.run(_connect_and_send_invalid_password_auth(f"ws://{host}:{port}/"))


def test_obsws_rejects_authenticated_connection_without_auth(binary_path: Path):
    """obsws が password 指定時に authentication 欠落を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        password="test-password",
        use_env=False,
    ):
        asyncio.run(_connect_and_send_missing_password_auth(f"ws://{host}:{port}/"))


def test_obsws_get_version_request(binary_path: Path):
    """obsws が GetVersion request に応答することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetVersion",
                request_id="req-get-version",
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is True
        assert status["code"] == 100
        response_data = response["d"]["responseData"]
        assert response_data["rpcVersion"] == 1
        assert "GetVersion" in response_data["availableRequests"]
        assert "GetInputList" in response_data["availableRequests"]
        assert "GetInputKindList" in response_data["availableRequests"]
        assert "GetInputSettings" in response_data["availableRequests"]
        assert "SetInputSettings" in response_data["availableRequests"]
        assert "SetInputName" in response_data["availableRequests"]
        assert "GetInputDefaultSettings" in response_data["availableRequests"]
        assert "CreateInput" in response_data["availableRequests"]
        assert "RemoveInput" in response_data["availableRequests"]
        assert "RemoveScene" in response_data["availableRequests"]
        assert "GetSceneList" in response_data["availableRequests"]
        assert "GetSceneItemId" in response_data["availableRequests"]
        assert "GetSceneItemEnabled" in response_data["availableRequests"]
        assert "SetSceneItemEnabled" in response_data["availableRequests"]
        assert "GetSceneItemLocked" in response_data["availableRequests"]
        assert "SetSceneItemLocked" in response_data["availableRequests"]
        assert "GetSceneItemBlendMode" in response_data["availableRequests"]
        assert "SetSceneItemBlendMode" in response_data["availableRequests"]
        assert "GetSceneItemTransform" in response_data["availableRequests"]
        assert "SetSceneItemTransform" in response_data["availableRequests"]
        assert "SetStreamServiceSettings" in response_data["availableRequests"]
        assert "StartStream" in response_data["availableRequests"]
        assert "ToggleStream" in response_data["availableRequests"]
        assert "GetRecordDirectory" in response_data["availableRequests"]
        assert "SetRecordDirectory" in response_data["availableRequests"]
        assert "GetRecordStatus" in response_data["availableRequests"]
        assert "StartRecord" in response_data["availableRequests"]
        assert "ToggleRecord" in response_data["availableRequests"]
        assert "StopRecord" in response_data["availableRequests"]
        assert "PauseRecord" in response_data["availableRequests"]
        assert "ResumeRecord" in response_data["availableRequests"]
        assert "ToggleRecordPause" in response_data["availableRequests"]
        supported_image_formats = response_data["supportedImageFormats"]
        assert isinstance(supported_image_formats, list)
        assert "png" in supported_image_formats


def test_obsws_get_stats_request(binary_path: Path):
    """obsws が GetStats request に応答することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetStats",
                request_id="req-get-stats",
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is True
        assert status["code"] == 100
        response_data = response["d"]["responseData"]
        assert response_data["webSocketSessionIncomingMessages"] >= 2
        assert response_data["webSocketSessionOutgoingMessages"] >= 2


def test_obsws_get_and_set_record_directory_request(binary_path: Path, tmp_path: Path):
    """obsws が GetRecordDirectory / SetRecordDirectory request に応答することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()
    default_record_dir = tmp_path / "default-records"
    updated_record_dir = tmp_path / "updated-records"

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        default_record_dir=default_record_dir,
        use_env=False,
    ):
        get_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetRecordDirectory",
                request_id="req-get-record-dir-1",
            )
        )
        get_status = get_response["d"]["requestStatus"]
        assert get_status["result"] is True
        assert get_response["d"]["responseData"]["recordDirectory"] == str(
            default_record_dir
        )

        set_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetRecordDirectory",
                request_id="req-set-record-dir-1",
                request_data={"recordDirectory": str(updated_record_dir)},
            )
        )
        set_status = set_response["d"]["requestStatus"]
        assert set_status["result"] is True

        get_response_after_update = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetRecordDirectory",
                request_id="req-get-record-dir-2",
            )
        )
        get_status_after_update = get_response_after_update["d"]["requestStatus"]
        assert get_status_after_update["result"] is True
        assert get_response_after_update["d"]["responseData"]["recordDirectory"] == str(
            updated_record_dir
        )


def test_obsws_get_record_status_request(binary_path: Path):
    """obsws が GetRecordStatus request に応答することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetRecordStatus",
                request_id="req-get-record-status",
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is True
        assert status["code"] == 100
        response_data = response["d"]["responseData"]
        assert response_data["outputActive"] is False
        assert response_data["outputPaused"] is False


def test_obsws_get_canvas_list_request(binary_path: Path):
    """obsws が GetCanvasList request に応答することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetCanvasList",
                request_id="req-get-canvas-list",
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is True
        assert status["code"] == 100
        response_data = response["d"]["responseData"]
        assert isinstance(response_data["canvases"], list)


def test_obsws_get_input_list_request(binary_path: Path):
    """obsws が GetInputList request に応答することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputList",
                request_id="req-get-input-list",
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is True
        assert status["code"] == 100
        response_data = response["d"]["responseData"]
        assert isinstance(response_data["inputs"], list)


def test_obsws_get_input_kind_list_request(binary_path: Path):
    """obsws が GetInputKindList request に応答することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputKindList",
                request_id="req-get-input-kind-list",
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is True
        assert status["code"] == 100
        response_data = response["d"]["responseData"]
        assert isinstance(response_data["inputKinds"], list)
        assert "video_capture_device" in response_data["inputKinds"]


def test_obsws_set_input_name_request(binary_path: Path):
    """obsws が SetInputName request に応答して入力名を変更できることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        create_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="CreateInput",
                request_id="req-create-input-for-set-name",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "obsws-set-name-input",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                },
            )
        )
        assert create_response["d"]["requestStatus"]["result"] is True

        set_name_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetInputName",
                request_id="req-set-input-name",
                request_data={
                    "inputName": "obsws-set-name-input",
                    "newInputName": "obsws-set-name-input-renamed",
                },
            )
        )
        set_name_status = set_name_response["d"]["requestStatus"]
        assert set_name_status["result"] is True
        assert set_name_status["code"] == 100

        old_name_get_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputSettings",
                request_id="req-get-input-settings-old-name",
                request_data={"inputName": "obsws-set-name-input"},
            )
        )
        assert old_name_get_response["d"]["requestStatus"]["result"] is False
        assert old_name_get_response["d"]["requestStatus"]["code"] == 601

        renamed_get_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputSettings",
                request_id="req-get-input-settings-renamed",
                request_data={"inputName": "obsws-set-name-input-renamed"},
            )
        )
        assert renamed_get_response["d"]["requestStatus"]["result"] is True
        assert renamed_get_response["d"]["responseData"]["inputName"] == (
            "obsws-set-name-input-renamed"
        )


def test_obsws_get_input_default_settings_request(binary_path: Path):
    """obsws が GetInputDefaultSettings request に応答することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputDefaultSettings",
                request_id="req-get-input-default-settings",
                request_data={"inputKind": "video_capture_device"},
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is True
        assert status["code"] == 100
        response_data = response["d"]["responseData"]
        assert response_data["inputKind"] == "video_capture_device"
        assert response_data["defaultInputSettings"] == {}

        unsupported_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputDefaultSettings",
                request_id="req-get-input-default-settings-unsupported",
                request_data={"inputKind": "unsupported-kind"},
            )
        )
        unsupported_status = unsupported_response["d"]["requestStatus"]
        assert unsupported_status["result"] is False
        assert unsupported_status["code"] == 400


def test_obsws_get_input_settings_without_lookup_fields(binary_path: Path):
    """obsws が GetInputSettings で識別子欠落をエラー応答することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputSettings",
                request_id="req-get-input-settings",
                request_data={},
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is False
        assert status["code"] == 300


def test_obsws_set_input_settings_request(binary_path: Path):
    """obsws が SetInputSettings request に応答して入力設定を更新できることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        create_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="CreateInput",
                request_id="req-create-input-for-set-settings",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "obsws-set-settings-input",
                    "inputKind": "video_capture_device",
                    "inputSettings": {"device_id": "before-device"},
                    "sceneItemEnabled": True,
                },
            )
        )
        assert create_response["d"]["requestStatus"]["result"] is True
        input_uuid = create_response["d"]["responseData"]["inputUuid"]

        set_overlay_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetInputSettings",
                request_id="req-set-input-settings-overlay",
                request_data={
                    "inputUuid": input_uuid,
                    "inputSettings": {"device_id": "after-device"},
                },
            )
        )
        set_overlay_status = set_overlay_response["d"]["requestStatus"]
        assert set_overlay_status["result"] is True
        assert set_overlay_status["code"] == 100

        get_overlay_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputSettings",
                request_id="req-get-input-settings-after-overlay",
                request_data={"inputUuid": input_uuid},
            )
        )
        assert get_overlay_response["d"]["requestStatus"]["result"] is True
        assert (
            get_overlay_response["d"]["responseData"]["inputSettings"]["device_id"]
            == "after-device"
        )

        set_replace_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetInputSettings",
                request_id="req-set-input-settings-replace",
                request_data={
                    "inputName": "obsws-set-settings-input",
                    "inputSettings": {},
                    "overlay": False,
                },
            )
        )
        set_replace_status = set_replace_response["d"]["requestStatus"]
        assert set_replace_status["result"] is True
        assert set_replace_status["code"] == 100

        get_replace_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputSettings",
                request_id="req-get-input-settings-after-replace",
                request_data={"inputName": "obsws-set-settings-input"},
            )
        )
        assert get_replace_response["d"]["requestStatus"]["result"] is True
        assert (
            "device_id"
            not in get_replace_response["d"]["responseData"]["inputSettings"]
        )

        not_found_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetInputSettings",
                request_id="req-set-input-settings-not-found",
                request_data={
                    "inputName": "not-found-input",
                    "inputSettings": {},
                },
            )
        )
        not_found_status = not_found_response["d"]["requestStatus"]
        assert not_found_status["result"] is False
        assert not_found_status["code"] == 601


def test_obsws_set_input_settings_rejects_invalid_input_settings(binary_path: Path):
    """obsws が SetInputSettings で不正な inputSettings を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        create_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="CreateInput",
                request_id="req-create-input-invalid-set-settings",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "obsws-invalid-set-settings-input",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                },
            )
        )
        assert create_response["d"]["requestStatus"]["result"] is True

        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetInputSettings",
                request_id="req-set-input-settings-invalid",
                request_data={
                    "inputName": "obsws-invalid-set-settings-input",
                    "inputSettings": {"device_id": 1},
                },
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is False
        assert status["code"] == 400


def test_obsws_set_input_settings_rejects_missing_request_data(binary_path: Path):
    """obsws が SetInputSettings で requestData 欠落を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetInputSettings",
                request_id="req-set-input-settings-missing-request-data",
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is False
        assert status["code"] == 300


def test_obsws_set_input_settings_rejects_missing_lookup_fields(binary_path: Path):
    """obsws が SetInputSettings で識別子欠落を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetInputSettings",
                request_id="req-set-input-settings-missing-lookup",
                request_data={"inputSettings": {}},
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is False
        assert status["code"] == 300


def test_obsws_set_input_settings_rejects_missing_input_settings(binary_path: Path):
    """obsws が SetInputSettings で inputSettings 欠落を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        create_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="CreateInput",
                request_id="req-create-input-for-missing-input-settings",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "obsws-missing-input-settings-input",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                },
            )
        )
        assert create_response["d"]["requestStatus"]["result"] is True

        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetInputSettings",
                request_id="req-set-input-settings-missing-input-settings",
                request_data={"inputName": "obsws-missing-input-settings-input"},
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is False
        assert status["code"] == 300


def test_obsws_set_input_settings_rejects_invalid_overlay_type(binary_path: Path):
    """obsws が SetInputSettings で overlay 型不正を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        create_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="CreateInput",
                request_id="req-create-input-for-invalid-overlay",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "obsws-invalid-overlay-input",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                },
            )
        )
        assert create_response["d"]["requestStatus"]["result"] is True

        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetInputSettings",
                request_id="req-set-input-settings-invalid-overlay",
                request_data={
                    "inputName": "obsws-invalid-overlay-input",
                    "inputSettings": {},
                    "overlay": "invalid",
                },
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is False
        assert status["code"] == 400


def test_obsws_create_input_request(binary_path: Path):
    """obsws が CreateInput request に応答して入力を追加できることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        create_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="CreateInput",
                request_id="req-create-input",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "obsws-test-input",
                    "inputKind": "video_capture_device",
                    "inputSettings": {"device_id": "sample-device"},
                    "sceneItemEnabled": True,
                },
            )
        )
        create_status = create_response["d"]["requestStatus"]
        assert create_status["result"] is True
        assert create_status["code"] == 100
        input_uuid = create_response["d"]["responseData"]["inputUuid"]
        assert isinstance(input_uuid, str)
        assert input_uuid != ""

        list_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputList",
                request_id="req-get-input-list-after-create",
            )
        )
        list_status = list_response["d"]["requestStatus"]
        assert list_status["result"] is True
        names = [v["inputName"] for v in list_response["d"]["responseData"]["inputs"]]
        assert "obsws-test-input" in names

        settings_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputSettings",
                request_id="req-get-input-settings-after-create",
                request_data={"inputUuid": input_uuid},
            )
        )
        settings_status = settings_response["d"]["requestStatus"]
        assert settings_status["result"] is True
        assert settings_response["d"]["responseData"]["inputName"] == "obsws-test-input"
        assert (
            settings_response["d"]["responseData"]["inputSettings"]["device_id"]
            == "sample-device"
        )


def test_obsws_create_input_rejects_duplicate_name(binary_path: Path):
    """obsws が CreateInput で inputName 重複を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        first_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="CreateInput",
                request_id="req-create-input-first",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "duplicate-input",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                },
            )
        )
        assert first_response["d"]["requestStatus"]["result"] is True

        second_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="CreateInput",
                request_id="req-create-input-second",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "duplicate-input",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                },
            )
        )
        second_status = second_response["d"]["requestStatus"]
        assert second_status["result"] is False
        assert second_status["code"] == 602


def test_obsws_create_input_rejects_unsupported_scene_name(binary_path: Path):
    """obsws が CreateInput で未対応 sceneName を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="CreateInput",
                request_id="req-create-input-unsupported-scene",
                request_data={
                    "sceneName": "custom-scene",
                    "inputName": "scene-rejected",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                },
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is False
        assert status["code"] == 601


def test_obsws_create_input_rejects_unsupported_input_kind(binary_path: Path):
    """obsws が CreateInput で未対応 inputKind を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="CreateInput",
                request_id="req-create-input-unsupported-kind",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "kind-rejected",
                    "inputKind": "unsupported_kind",
                    "inputSettings": {},
                },
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is False
        assert status["code"] == 400


def test_obsws_remove_input_request(binary_path: Path):
    """obsws が RemoveInput request に応答して入力を削除できることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        create_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="CreateInput",
                request_id="req-create-for-remove",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "to-be-removed",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                },
            )
        )
        assert create_response["d"]["requestStatus"]["result"] is True

        remove_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="RemoveInput",
                request_id="req-remove-input",
                request_data={"inputName": "to-be-removed"},
            )
        )
        remove_status = remove_response["d"]["requestStatus"]
        assert remove_status["result"] is True
        assert remove_status["code"] == 100

        list_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputList",
                request_id="req-get-input-list-after-remove",
            )
        )
        list_status = list_response["d"]["requestStatus"]
        assert list_status["result"] is True
        names = [v["inputName"] for v in list_response["d"]["responseData"]["inputs"]]
        assert "to-be-removed" not in names


def test_obsws_remove_input_rejects_unknown_input(binary_path: Path):
    """obsws が RemoveInput で存在しない入力を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="RemoveInput",
                request_id="req-remove-input-not-found",
                request_data={"inputName": "not-found"},
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is False
        assert status["code"] == 601


def test_obsws_get_scene_item_id_request(binary_path: Path):
    """obsws が GetSceneItemId request に応答することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    async def _run():
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
                request_id="req-create-input-scene-item-id",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "scene-item-id-input",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemId",
                request_id="req-get-scene-item-id",
                request_data={
                    "sceneName": "Scene",
                    "sourceName": "scene-item-id-input",
                    "searchOffset": 0,
                },
            )
            status = response["d"]["requestStatus"]
            assert status["result"] is True
            assert status["code"] == 100
            scene_item_id = response["d"]["responseData"]["sceneItemId"]
            assert isinstance(scene_item_id, int)
            assert scene_item_id > 0
            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run())


def test_obsws_set_scene_item_enabled_controls_start_record_precondition(
    binary_path: Path, tmp_path: Path
):
    """obsws が SetSceneItemEnabled で StartRecord の前提入力を切り替えられることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()
    image_path = tmp_path / "set-scene-item-enabled-input.png"
    _write_test_png(image_path)

    async def _run():
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
                request_id="req-create-input-scene-item-enabled",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "scene-item-enabled-input",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            get_scene_item_id_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemId",
                request_id="req-get-scene-item-id-for-set",
                request_data={
                    "sceneName": "Scene",
                    "sourceName": "scene-item-enabled-input",
                    "searchOffset": 0,
                },
            )
            assert get_scene_item_id_response["d"]["requestStatus"]["result"] is True
            scene_item_id = get_scene_item_id_response["d"]["responseData"]["sceneItemId"]

            disable_response = await _send_obsws_request(
                ws,
                request_type="SetSceneItemEnabled",
                request_id="req-set-scene-item-disabled",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                    "sceneItemEnabled": False,
                },
            )
            assert disable_response["d"]["requestStatus"]["result"] is True

            start_record_error_response = await _send_obsws_request(
                ws,
                request_type="StartRecord",
                request_id="req-start-record-disabled-input",
            )
            start_record_error_status = start_record_error_response["d"]["requestStatus"]
            assert start_record_error_status["result"] is False
            assert start_record_error_status["code"] == 400

            enable_response = await _send_obsws_request(
                ws,
                request_type="SetSceneItemEnabled",
                request_id="req-set-scene-item-enabled",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                    "sceneItemEnabled": True,
                },
            )
            assert enable_response["d"]["requestStatus"]["result"] is True

            start_record_response = await _send_obsws_request(
                ws,
                request_type="StartRecord",
                request_id="req-start-record-enabled-input",
            )
            assert start_record_response["d"]["requestStatus"]["result"] is True
            assert start_record_response["d"]["responseData"]["outputActive"] is True
            assert start_record_response["d"]["responseData"]["outputPaused"] is False

            stop_record_response = await _send_obsws_request(
                ws,
                request_type="StopRecord",
                request_id="req-stop-record-enabled-input",
            )
            assert stop_record_response["d"]["requestStatus"]["result"] is True
            assert stop_record_response["d"]["responseData"]["outputPath"]
            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run())


def test_obsws_get_scene_item_enabled_request(binary_path: Path):
    """obsws が GetSceneItemEnabled request に応答することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    async def _run():
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
                request_id="req-create-input-get-scene-item-enabled",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "get-scene-item-enabled-input",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            get_scene_item_id_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemId",
                request_id="req-get-scene-item-id-for-get-enabled",
                request_data={
                    "sceneName": "Scene",
                    "sourceName": "get-scene-item-enabled-input",
                    "searchOffset": 0,
                },
            )
            assert get_scene_item_id_response["d"]["requestStatus"]["result"] is True
            scene_item_id = get_scene_item_id_response["d"]["responseData"]["sceneItemId"]

            get_enabled_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemEnabled",
                request_id="req-get-scene-item-enabled-true",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                },
            )
            assert get_enabled_response["d"]["requestStatus"]["result"] is True
            assert get_enabled_response["d"]["responseData"]["sceneItemEnabled"] is True

            set_disabled_response = await _send_obsws_request(
                ws,
                request_type="SetSceneItemEnabled",
                request_id="req-set-scene-item-enabled-false-for-get",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                    "sceneItemEnabled": False,
                },
            )
            assert set_disabled_response["d"]["requestStatus"]["result"] is True

            get_disabled_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemEnabled",
                request_id="req-get-scene-item-enabled-false",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                },
            )
            assert get_disabled_response["d"]["requestStatus"]["result"] is True
            assert get_disabled_response["d"]["responseData"]["sceneItemEnabled"] is False
            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run())


def test_obsws_scene_item_management_requests(binary_path: Path):
    """obsws の Scene Item 管理 request 一式が動作することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    async def _run():
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
                request_id="req-create-input-scene-item-management",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "scene-item-management-input",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                    "sceneItemEnabled": False,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True
            source_uuid = create_input_response["d"]["responseData"]["inputUuid"]

            create_scene_item_response = await _send_obsws_request(
                ws,
                request_type="CreateSceneItem",
                request_id="req-create-scene-item-1",
                request_data={
                    "sceneName": "Scene",
                    "sourceUuid": source_uuid,
                    "sceneItemEnabled": True,
                },
            )
            assert create_scene_item_response["d"]["requestStatus"]["result"] is True
            first_scene_item_id = create_scene_item_response["d"]["responseData"]["sceneItemId"]

            create_second_scene_item_response = await _send_obsws_request(
                ws,
                request_type="CreateSceneItem",
                request_id="req-create-scene-item-2",
                request_data={
                    "sceneName": "Scene",
                    "sourceUuid": source_uuid,
                    "sceneItemEnabled": True,
                },
            )
            assert create_second_scene_item_response["d"]["requestStatus"]["result"] is True
            second_scene_item_id = create_second_scene_item_response["d"]["responseData"][
                "sceneItemId"
            ]

            get_scene_item_list_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemList",
                request_id="req-get-scene-item-list",
                request_data={"sceneName": "Scene"},
            )
            assert get_scene_item_list_response["d"]["requestStatus"]["result"] is True
            scene_items = get_scene_item_list_response["d"]["responseData"]["sceneItems"]
            scene_item_ids = [item["sceneItemId"] for item in scene_items]
            assert first_scene_item_id in scene_item_ids
            assert second_scene_item_id in scene_item_ids

            get_scene_item_source_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemSource",
                request_id="req-get-scene-item-source",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": first_scene_item_id,
                },
            )
            assert get_scene_item_source_response["d"]["requestStatus"]["result"] is True
            assert (
                get_scene_item_source_response["d"]["responseData"]["sourceUuid"]
                == source_uuid
            )
            assert (
                get_scene_item_source_response["d"]["responseData"]["sourceName"]
                == "scene-item-management-input"
            )

            get_second_scene_item_index_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemIndex",
                request_id="req-get-scene-item-index-before",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": second_scene_item_id,
                },
            )
            assert get_second_scene_item_index_response["d"]["requestStatus"]["result"] is True
            assert (
                get_second_scene_item_index_response["d"]["responseData"]["sceneItemIndex"]
                == 2
            )

            set_scene_item_index_response = await _send_obsws_request(
                ws,
                request_type="SetSceneItemIndex",
                request_id="req-set-scene-item-index",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": second_scene_item_id,
                    "sceneItemIndex": 0,
                },
            )
            assert set_scene_item_index_response["d"]["requestStatus"]["result"] is True

            get_second_scene_item_index_after_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemIndex",
                request_id="req-get-scene-item-index-after",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": second_scene_item_id,
                },
            )
            assert (
                get_second_scene_item_index_after_response["d"]["responseData"][
                    "sceneItemIndex"
                ]
                == 0
            )

            remove_scene_item_response = await _send_obsws_request(
                ws,
                request_type="RemoveSceneItem",
                request_id="req-remove-scene-item",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": first_scene_item_id,
                },
            )
            assert remove_scene_item_response["d"]["requestStatus"]["result"] is True

            get_scene_item_list_after_remove_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemList",
                request_id="req-get-scene-item-list-after-remove",
                request_data={"sceneName": "Scene"},
            )
            scene_items_after_remove = get_scene_item_list_after_remove_response["d"][
                "responseData"
            ]["sceneItems"]
            scene_item_ids_after_remove = [
                item["sceneItemId"] for item in scene_items_after_remove
            ]
            assert first_scene_item_id not in scene_item_ids_after_remove
            assert second_scene_item_id in scene_item_ids_after_remove

            duplicate_scene_item_response = await _send_obsws_request(
                ws,
                request_type="DuplicateSceneItem",
                request_id="req-duplicate-scene-item",
                request_data={
                    "fromSceneName": "Scene",
                    "toSceneName": "Scene",
                    "sceneItemId": second_scene_item_id,
                },
            )
            assert duplicate_scene_item_response["d"]["requestStatus"]["result"] is True
            duplicated_scene_item_id = duplicate_scene_item_response["d"]["responseData"][
                "sceneItemId"
            ]
            assert duplicated_scene_item_id != second_scene_item_id
            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run())


def test_obsws_scene_item_locked_blend_mode_transform_requests(binary_path: Path):
    """obsws の Scene Item の lock / blend mode / transform request が動作することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    async def _run():
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
                request_id="req-create-input-scene-item-extra-requests",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "scene-item-extra-input",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            get_scene_item_id_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemId",
                request_id="req-get-scene-item-id-scene-item-extra-requests",
                request_data={
                    "sceneName": "Scene",
                    "sourceName": "scene-item-extra-input",
                    "searchOffset": 0,
                },
            )
            assert get_scene_item_id_response["d"]["requestStatus"]["result"] is True
            scene_item_id = get_scene_item_id_response["d"]["responseData"]["sceneItemId"]

            get_locked_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemLocked",
                request_id="req-get-scene-item-locked-before",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                },
            )
            assert get_locked_response["d"]["requestStatus"]["result"] is True
            assert get_locked_response["d"]["responseData"]["sceneItemLocked"] is False

            set_locked_response = await _send_obsws_request(
                ws,
                request_type="SetSceneItemLocked",
                request_id="req-set-scene-item-locked-true",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                    "sceneItemLocked": True,
                },
            )
            assert set_locked_response["d"]["requestStatus"]["result"] is True

            get_locked_after_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemLocked",
                request_id="req-get-scene-item-locked-after",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                },
            )
            assert get_locked_after_response["d"]["requestStatus"]["result"] is True
            assert get_locked_after_response["d"]["responseData"]["sceneItemLocked"] is True

            get_blend_mode_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemBlendMode",
                request_id="req-get-scene-item-blend-mode-before",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                },
            )
            assert get_blend_mode_response["d"]["requestStatus"]["result"] is True
            assert (
                get_blend_mode_response["d"]["responseData"]["sceneItemBlendMode"]
                == "OBS_BLEND_NORMAL"
            )

            set_blend_mode_response = await _send_obsws_request(
                ws,
                request_type="SetSceneItemBlendMode",
                request_id="req-set-scene-item-blend-mode",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                    "sceneItemBlendMode": "OBS_BLEND_ADDITIVE",
                },
            )
            assert set_blend_mode_response["d"]["requestStatus"]["result"] is True

            get_blend_mode_after_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemBlendMode",
                request_id="req-get-scene-item-blend-mode-after",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                },
            )
            assert get_blend_mode_after_response["d"]["requestStatus"]["result"] is True
            assert (
                get_blend_mode_after_response["d"]["responseData"]["sceneItemBlendMode"]
                == "OBS_BLEND_ADDITIVE"
            )

            set_transform_response = await _send_obsws_request(
                ws,
                request_type="SetSceneItemTransform",
                request_id="req-set-scene-item-transform",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                    "sceneItemTransform": {
                        "positionX": 12.5,
                        "positionY": 7.25,
                        "boundsType": "OBS_BOUNDS_STRETCH",
                    },
                },
            )
            assert set_transform_response["d"]["requestStatus"]["result"] is True

            get_transform_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemTransform",
                request_id="req-get-scene-item-transform-after",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                },
            )
            assert get_transform_response["d"]["requestStatus"]["result"] is True
            scene_item_transform = get_transform_response["d"]["responseData"][
                "sceneItemTransform"
            ]
            assert scene_item_transform["positionX"] == 12.5
            assert scene_item_transform["positionY"] == 7.25
            assert scene_item_transform["boundsType"] == "OBS_BOUNDS_STRETCH"
            await ws.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_run())


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
                execution_type=0,
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
                execution_type=0,
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
                execution_type=0,
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
                            "executionType": 1,
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
                execution_type=0,
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
            assert start_record_response["d"]["responseData"]["outputActive"] is True
            assert start_record_response["d"]["responseData"]["outputPaused"] is False

            pause_record_response = await _send_obsws_request(
                ws,
                request_type="PauseRecord",
                request_id="req-pause-record",
            )
            assert pause_record_response["d"]["requestStatus"]["result"] is True
            assert pause_record_response["d"]["responseData"]["outputActive"] is True
            assert pause_record_response["d"]["responseData"]["outputPaused"] is True

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
            assert resume_record_response["d"]["responseData"]["outputActive"] is True
            assert resume_record_response["d"]["responseData"]["outputPaused"] is False

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
                f"http://{server.http_host}:{server.http_port}/metrics"
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
            assert start_stream_response["d"]["responseData"]["outputActive"] is True

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
            start_stream_future = executor.submit(_run_start_stream_flow_sync)

            ffmpeg_process = _start_ffmpeg_rtmp_receive(
                receive_url,
                output_path,
                max_video_frames=None,
                startup_timeout=20.0,
            )
            try:
                start_stream_future.result(timeout=30.0)
                _wait_process_exit(ffmpeg_process, timeout=20.0)
            except Exception as e:
                # 失敗時の原因切り分け用にメトリクスを添付する。
                metrics_snapshot = _collect_obsws_metrics_snapshot(
                    server.http_host,
                    server.http_port,
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
    inspect_output = _inspect_mp4(binary_path, output_path)
    assert inspect_output["format"] == "mp4"
    assert inspect_output["video_codec"] == "H264"
    assert inspect_output["video_sample_count"] > 0


def test_obsws_unknown_request_type_returns_error(binary_path: Path):
    """obsws が未知 requestType をエラー応答することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="UnknownRequestType",
                request_id="req-unknown",
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is False
        assert status["code"] == 204


def test_obsws_rejects_request_before_identify(binary_path: Path):
    """obsws が Identify 前 Request を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(
            _connect_and_expect_close_code(
                f"ws://{host}:{port}/",
                {
                    "op": 6,
                    "d": {
                        "requestType": "GetVersion",
                        "requestId": "req-before-identify",
                    },
                },
                4007,
            )
        )


def test_obsws_rejects_duplicate_identify(binary_path: Path):
    """obsws が重複 Identify を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(_connect_and_send_duplicate_identify(f"ws://{host}:{port}/"))


def test_obsws_accepts_reidentify_after_identify(binary_path: Path):
    """obsws が Identify 後の Reidentify を受け付けて接続を継続することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(
            _connect_identify_and_send_reidentify_then_request(f"ws://{host}:{port}/")
        )


def test_obsws_rejects_reidentify_with_invalid_event_subscriptions(binary_path: Path):
    """obsws が Identify 後の不正な Reidentify payload を invalid payload として拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(
            _connect_identify_and_expect_close_code(
                f"ws://{host}:{port}/",
                {"op": 3, "d": {"eventSubscriptions": "invalid"}},
                1007,
            )
        )


def test_obsws_stream_events_are_sent_when_outputs_subscription_enabled(
    binary_path: Path, tmp_path: Path
):
    """obsws が Outputs 購読時に StartStream / StopStream のイベントを送ることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    rtmp_port, rtmp_sock = reserve_ephemeral_port()
    rtmp_sock.close()

    image_path = tmp_path / "event-enabled-input.png"
    _write_test_png(image_path)
    output_url = f"rtmp://127.0.0.1:{rtmp_port}/live"
    stream_key = "event-enabled-key"

    async def _run():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(
                ws,
                None,
                event_subscriptions=OBSWS_EVENT_SUB_OUTPUTS,
            )
            await _setup_stream_input_and_service(
                ws,
                image_path=image_path,
                output_url=output_url,
                stream_key=stream_key,
            )

            start_response = await _send_obsws_request(
                ws,
                request_type="StartStream",
                request_id="req-start-stream-event-enabled",
            )
            assert start_response["d"]["requestStatus"]["result"] is True
            assert start_response["d"]["responseData"]["outputActive"] is True
            await _expect_stream_state_changed_event(ws, output_active=True)

            stop_response = await _send_obsws_request(
                ws,
                request_type="StopStream",
                request_id="req-stop-stream-event-enabled",
            )
            assert stop_response["d"]["requestStatus"]["result"] is True
            await _expect_stream_state_changed_event(ws, output_active=False)
            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False):
        asyncio.run(_run())


def test_obsws_stream_events_follow_reidentify_updates(
    binary_path: Path, tmp_path: Path
):
    """obsws が Reidentify 後に更新した Outputs 購読設定でイベントを送ることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    rtmp_port, rtmp_sock = reserve_ephemeral_port()
    rtmp_sock.close()

    image_path = tmp_path / "event-reidentify-input.png"
    _write_test_png(image_path)
    output_url = f"rtmp://127.0.0.1:{rtmp_port}/live"
    stream_key = "event-reidentify-key"

    async def _run():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)
            await _setup_stream_input_and_service(
                ws,
                image_path=image_path,
                output_url=output_url,
                stream_key=stream_key,
            )

            start_response = await _send_obsws_request(
                ws,
                request_type="StartStream",
                request_id="req-start-stream-event-reidentify",
            )
            assert start_response["d"]["requestStatus"]["result"] is True
            await _assert_no_message_within(ws, timeout=0.5)

            await ws.send_str(
                json.dumps(
                    {"op": 3, "d": {"eventSubscriptions": OBSWS_EVENT_SUB_OUTPUTS}}
                )
            )
            identified_msg = await ws.receive(timeout=5.0)
            assert identified_msg.type == aiohttp.WSMsgType.TEXT
            identified = json.loads(identified_msg.data)
            assert identified["op"] == 2

            stop_response = await _send_obsws_request(
                ws,
                request_type="StopStream",
                request_id="req-stop-stream-event-reidentify",
            )
            assert stop_response["d"]["requestStatus"]["result"] is True
            await _expect_stream_state_changed_event(ws, output_active=False)
            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False):
        asyncio.run(_run())


def test_obsws_stream_events_are_not_sent_without_outputs_subscription(
    binary_path: Path, tmp_path: Path
):
    """obsws が Outputs 非購読時は StartStream / StopStream のイベントを送らないことを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    rtmp_port, rtmp_sock = reserve_ephemeral_port()
    rtmp_sock.close()

    image_path = tmp_path / "event-disabled-input.png"
    _write_test_png(image_path)
    output_url = f"rtmp://127.0.0.1:{rtmp_port}/live"
    stream_key = "event-disabled-key"

    async def _run():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)
            await _setup_stream_input_and_service(
                ws,
                image_path=image_path,
                output_url=output_url,
                stream_key=stream_key,
            )

            start_response = await _send_obsws_request(
                ws,
                request_type="StartStream",
                request_id="req-start-stream-event-disabled",
            )
            assert start_response["d"]["requestStatus"]["result"] is True
            await _assert_no_message_within(ws, timeout=0.5)

            stop_response = await _send_obsws_request(
                ws,
                request_type="StopStream",
                request_id="req-stop-stream-event-disabled",
            )
            assert stop_response["d"]["requestStatus"]["result"] is True
            await _assert_no_message_within(ws, timeout=0.5)
            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False):
        asyncio.run(_run())


def test_obsws_toggle_stream_events_are_sent_when_outputs_subscription_enabled(
    binary_path: Path, tmp_path: Path
):
    """obsws が Outputs 購読時に ToggleStream のイベントを送ることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    rtmp_port, rtmp_sock = reserve_ephemeral_port()
    rtmp_sock.close()

    image_path = tmp_path / "toggle-stream-event-input.png"
    _write_test_png(image_path)
    output_url = f"rtmp://127.0.0.1:{rtmp_port}/live"
    stream_key = "toggle-stream-event-key"

    async def _run():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(
                ws,
                None,
                event_subscriptions=OBSWS_EVENT_SUB_OUTPUTS,
            )
            await _setup_stream_input_and_service(
                ws,
                image_path=image_path,
                output_url=output_url,
                stream_key=stream_key,
            )

            start_response = await _send_obsws_request(
                ws,
                request_type="ToggleStream",
                request_id="req-toggle-stream-event-enabled-on",
            )
            assert start_response["d"]["requestStatus"]["result"] is True
            await _expect_stream_state_changed_event(ws, output_active=True)

            stop_response = await _send_obsws_request(
                ws,
                request_type="ToggleStream",
                request_id="req-toggle-stream-event-enabled-off",
            )
            assert stop_response["d"]["requestStatus"]["result"] is True
            await _expect_stream_state_changed_event(ws, output_active=False)
            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False):
        asyncio.run(_run())


def test_obsws_record_events_are_sent_when_outputs_subscription_enabled(
    binary_path: Path, tmp_path: Path
):
    """obsws が Outputs 購読時に StartRecord / StopRecord のイベントを送ることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()

    image_path = tmp_path / "record-event-input.png"
    _write_test_png(image_path)

    async def _run():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(
                ws,
                None,
                event_subscriptions=OBSWS_EVENT_SUB_OUTPUTS,
            )
            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-record-event-input",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "record-event-input",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            start_record_response = await _send_obsws_request(
                ws,
                request_type="StartRecord",
                request_id="req-start-record-event-enabled",
            )
            assert start_record_response["d"]["requestStatus"]["result"] is True
            assert start_record_response["d"]["responseData"]["outputActive"] is True
            await _expect_record_state_changed_event(
                ws,
                output_active=True,
                output_paused=False,
            )

            stop_record_response = await _send_obsws_request(
                ws,
                request_type="StopRecord",
                request_id="req-stop-record-event-enabled",
            )
            assert stop_record_response["d"]["requestStatus"]["result"] is True
            stop_event = await _expect_record_state_changed_event(
                ws,
                output_active=False,
                output_paused=False,
            )
            assert stop_event["d"]["eventData"]["outputPath"]
            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False):
        asyncio.run(_run())


def test_obsws_toggle_record_events_are_sent_when_outputs_subscription_enabled(
    binary_path: Path, tmp_path: Path
):
    """obsws が Outputs 購読時に ToggleRecord のイベントを送ることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()

    image_path = tmp_path / "toggle-record-event-input.png"
    _write_test_png(image_path)

    async def _run():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(
                ws,
                None,
                event_subscriptions=OBSWS_EVENT_SUB_OUTPUTS,
            )
            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-toggle-record-event-input",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "toggle-record-event-input",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            start_response = await _send_obsws_request(
                ws,
                request_type="ToggleRecord",
                request_id="req-toggle-record-event-enabled-on",
            )
            assert start_response["d"]["requestStatus"]["result"] is True
            await _expect_record_state_changed_event(
                ws,
                output_active=True,
                output_paused=False,
            )

            stop_response = await _send_obsws_request(
                ws,
                request_type="ToggleRecord",
                request_id="req-toggle-record-event-enabled-off",
            )
            assert stop_response["d"]["requestStatus"]["result"] is True
            stop_event = await _expect_record_state_changed_event(
                ws,
                output_active=False,
                output_paused=False,
            )
            assert stop_event["d"]["eventData"]["outputPath"]
            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False):
        asyncio.run(_run())


def test_obsws_record_pause_events_are_sent_when_outputs_subscription_enabled(
    binary_path: Path, tmp_path: Path
):
    """obsws が Outputs 購読時に Pause/Resume 系イベントを送ることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()

    image_path = tmp_path / "pause-record-event-input.png"
    _write_test_png(image_path)

    async def _run():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(
                ws,
                None,
                event_subscriptions=OBSWS_EVENT_SUB_OUTPUTS,
            )
            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-pause-record-event-input",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "pause-record-event-input",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            start_response = await _send_obsws_request(
                ws,
                request_type="StartRecord",
                request_id="req-start-record-pause-events",
            )
            assert start_response["d"]["requestStatus"]["result"] is True
            await _expect_record_state_changed_event(
                ws,
                output_active=True,
                output_paused=False,
            )

            toggle_pause_on_response = await _send_obsws_request(
                ws,
                request_type="ToggleRecordPause",
                request_id="req-toggle-record-pause-events-on",
            )
            assert toggle_pause_on_response["d"]["requestStatus"]["result"] is True
            assert toggle_pause_on_response["d"]["responseData"]["outputPaused"] is True
            await _expect_record_state_changed_event(
                ws,
                output_active=True,
                output_paused=True,
            )

            toggle_pause_off_response = await _send_obsws_request(
                ws,
                request_type="ToggleRecordPause",
                request_id="req-toggle-record-pause-events-off",
            )
            assert toggle_pause_off_response["d"]["requestStatus"]["result"] is True
            assert toggle_pause_off_response["d"]["responseData"]["outputPaused"] is False
            await _expect_record_state_changed_event(
                ws,
                output_active=True,
                output_paused=False,
            )

            pause_response = await _send_obsws_request(
                ws,
                request_type="PauseRecord",
                request_id="req-pause-record-events",
            )
            assert pause_response["d"]["requestStatus"]["result"] is True
            assert pause_response["d"]["responseData"]["outputPaused"] is True
            await _expect_record_state_changed_event(
                ws,
                output_active=True,
                output_paused=True,
            )

            resume_response = await _send_obsws_request(
                ws,
                request_type="ResumeRecord",
                request_id="req-resume-record-events",
            )
            assert resume_response["d"]["requestStatus"]["result"] is True
            assert resume_response["d"]["responseData"]["outputPaused"] is False
            await _expect_record_state_changed_event(
                ws,
                output_active=True,
                output_paused=False,
            )

            stop_response = await _send_obsws_request(
                ws,
                request_type="StopRecord",
                request_id="req-stop-record-pause-events",
            )
            assert stop_response["d"]["requestStatus"]["result"] is True
            stop_event = await _expect_record_state_changed_event(
                ws,
                output_active=False,
                output_paused=False,
            )
            assert stop_event["d"]["eventData"]["outputPath"]
            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False):
        asyncio.run(_run())


def test_obsws_scene_events_are_sent_when_scenes_subscription_enabled(
    binary_path: Path,
):
    """obsws が Scenes 購読時に Scene 関連イベントを送ることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()

    async def _run():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(
                ws,
                None,
                event_subscriptions=OBSWS_EVENT_SUB_SCENES,
            )
            create_scene_response = await _send_obsws_request(
                ws,
                request_type="CreateScene",
                request_id="req-create-scene-events",
                request_data={"sceneName": "Scene B"},
            )
            assert create_scene_response["d"]["requestStatus"]["result"] is True
            create_event = await _expect_obsws_event(
                ws,
                event_type="SceneCreated",
                event_intent=OBSWS_EVENT_SUB_SCENES,
            )
            assert create_event["d"]["eventData"]["sceneName"] == "Scene B"

            set_scene_response = await _send_obsws_request(
                ws,
                request_type="SetCurrentProgramScene",
                request_id="req-set-current-program-scene-events",
                request_data={"sceneName": "Scene B"},
            )
            assert set_scene_response["d"]["requestStatus"]["result"] is True
            set_scene_event = await _expect_obsws_event(
                ws,
                event_type="CurrentProgramSceneChanged",
                event_intent=OBSWS_EVENT_SUB_SCENES,
            )
            assert set_scene_event["d"]["eventData"]["sceneName"] == "Scene B"

            remove_scene_response = await _send_obsws_request(
                ws,
                request_type="RemoveScene",
                request_id="req-remove-scene-events",
                request_data={"sceneName": "Scene B"},
            )
            assert remove_scene_response["d"]["requestStatus"]["result"] is True
            remove_event = await _expect_obsws_event(
                ws,
                event_type="SceneRemoved",
                event_intent=OBSWS_EVENT_SUB_SCENES,
            )
            assert remove_event["d"]["eventData"]["sceneName"] == "Scene B"
            current_scene_event = await _expect_obsws_event(
                ws,
                event_type="CurrentProgramSceneChanged",
                event_intent=OBSWS_EVENT_SUB_SCENES,
            )
            assert current_scene_event["d"]["eventData"]["sceneName"] == "Scene"
            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False):
        asyncio.run(_run())


def test_obsws_input_events_are_sent_when_inputs_subscription_enabled(
    binary_path: Path, tmp_path: Path
):
    """obsws が Inputs 購読時に Input 関連イベントを送ることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()

    image_path = tmp_path / "input-event-input.png"
    updated_image_path = tmp_path / "input-event-updated-input.png"
    _write_test_png(image_path)
    _write_test_png(updated_image_path)

    async def _run():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(
                ws,
                None,
                event_subscriptions=OBSWS_EVENT_SUB_INPUTS,
            )
            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-input-events",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "input-event-camera",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True
            create_event = await _expect_obsws_event(
                ws,
                event_type="InputCreated",
                event_intent=OBSWS_EVENT_SUB_INPUTS,
            )
            assert create_event["d"]["eventData"]["inputName"] == "input-event-camera"

            set_input_name_response = await _send_obsws_request(
                ws,
                request_type="SetInputName",
                request_id="req-set-input-name-events",
                request_data={
                    "inputName": "input-event-camera",
                    "newInputName": "input-event-camera-renamed",
                },
            )
            assert set_input_name_response["d"]["requestStatus"]["result"] is True
            input_name_changed_event = await _expect_input_name_changed_event(
                ws,
                input_name="input-event-camera-renamed",
                old_input_name="input-event-camera",
            )
            assert input_name_changed_event["d"]["eventData"]["inputUuid"] == create_event["d"][
                "eventData"
            ]["inputUuid"]

            create_input_response_2 = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-input-events-2",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "input-event-camera-2",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response_2["d"]["requestStatus"]["result"] is True
            create_event_2 = await _expect_obsws_event(
                ws,
                event_type="InputCreated",
                event_intent=OBSWS_EVENT_SUB_INPUTS,
            )
            assert create_event_2["d"]["eventData"]["inputName"] == "input-event-camera-2"

            invalid_set_input_name_response = await _send_obsws_request(
                ws,
                request_type="SetInputName",
                request_id="req-set-input-name-events-invalid",
                request_data={
                    "inputName": "input-event-camera-renamed",
                    "newInputName": "input-event-camera-2",
                },
            )
            invalid_set_input_name_status = invalid_set_input_name_response["d"]["requestStatus"]
            assert invalid_set_input_name_status["result"] is False
            assert invalid_set_input_name_status["code"] == 602
            await _assert_no_message_within(ws, timeout=0.5)

            set_input_settings_response = await _send_obsws_request(
                ws,
                request_type="SetInputSettings",
                request_id="req-set-input-settings-events",
                request_data={
                    "inputName": "input-event-camera-renamed",
                    "inputSettings": {"file": str(updated_image_path)},
                },
            )
            assert set_input_settings_response["d"]["requestStatus"]["result"] is True
            input_settings_changed_event = await _expect_input_settings_changed_event(
                ws,
                input_name="input-event-camera-renamed",
            )
            assert input_settings_changed_event["d"]["eventData"]["inputKind"] == "image_source"
            assert input_settings_changed_event["d"]["eventData"]["inputSettings"] == {
                "file": str(updated_image_path)
            }

            invalid_set_input_settings_response = await _send_obsws_request(
                ws,
                request_type="SetInputSettings",
                request_id="req-set-input-settings-events-invalid",
                request_data={
                    "inputName": "input-event-camera-renamed",
                    "inputSettings": {"file": 1},
                },
            )
            invalid_set_input_settings_status = invalid_set_input_settings_response["d"][
                "requestStatus"
            ]
            assert invalid_set_input_settings_status["result"] is False
            assert invalid_set_input_settings_status["code"] == 400
            await _assert_no_message_within(ws, timeout=0.5)

            remove_input_response = await _send_obsws_request(
                ws,
                request_type="RemoveInput",
                request_id="req-remove-input-events",
                request_data={"inputName": "input-event-camera-renamed"},
            )
            assert remove_input_response["d"]["requestStatus"]["result"] is True
            remove_event = await _expect_obsws_event(
                ws,
                event_type="InputRemoved",
                event_intent=OBSWS_EVENT_SUB_INPUTS,
            )
            assert remove_event["d"]["eventData"]["inputName"] == "input-event-camera-renamed"
            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False):
        asyncio.run(_run())


def test_obsws_scene_events_follow_reidentify_updates(binary_path: Path):
    """obsws が Reidentify 後の Scenes 購読設定変更をイベント送信へ反映することを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()

    async def _run():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(ws, None)
            create_scene_response = await _send_obsws_request(
                ws,
                request_type="CreateScene",
                request_id="req-create-scene-reidentify",
                request_data={"sceneName": "Scene C"},
            )
            assert create_scene_response["d"]["requestStatus"]["result"] is True
            await _assert_no_message_within(ws, timeout=0.5)

            await ws.send_str(
                json.dumps(
                    {"op": 3, "d": {"eventSubscriptions": OBSWS_EVENT_SUB_SCENES}}
                )
            )
            identified_msg = await ws.receive(timeout=5.0)
            assert identified_msg.type == aiohttp.WSMsgType.TEXT
            identified = json.loads(identified_msg.data)
            assert identified["op"] == 2

            set_scene_response = await _send_obsws_request(
                ws,
                request_type="SetCurrentProgramScene",
                request_id="req-set-scene-reidentify",
                request_data={"sceneName": "Scene C"},
            )
            assert set_scene_response["d"]["requestStatus"]["result"] is True
            set_scene_event = await _expect_obsws_event(
                ws,
                event_type="CurrentProgramSceneChanged",
                event_intent=OBSWS_EVENT_SUB_SCENES,
            )
            assert set_scene_event["d"]["eventData"]["sceneName"] == "Scene C"
            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False):
        asyncio.run(_run())


def test_obsws_scene_item_enabled_events_are_sent_when_scenes_subscription_enabled(
    binary_path: Path, tmp_path: Path
):
    """obsws が Scenes 購読時に SetSceneItemEnabled のイベントを送ることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()

    image_path = tmp_path / "scene-item-event-input.png"
    _write_test_png(image_path)

    async def _run():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(
                ws,
                None,
                event_subscriptions=OBSWS_EVENT_SUB_SCENES,
            )

            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-input-scene-item-event",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "scene-item-event-input",
                    "inputKind": "image_source",
                    "inputSettings": {"file": str(image_path)},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            get_scene_item_id_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemId",
                request_id="req-get-scene-item-id-event",
                request_data={
                    "sceneName": "Scene",
                    "sourceName": "scene-item-event-input",
                    "searchOffset": 0,
                },
            )
            assert get_scene_item_id_response["d"]["requestStatus"]["result"] is True
            scene_item_id = get_scene_item_id_response["d"]["responseData"]["sceneItemId"]

            disable_response = await _send_obsws_request(
                ws,
                request_type="SetSceneItemEnabled",
                request_id="req-set-scene-item-disabled-event",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                    "sceneItemEnabled": False,
                },
            )
            assert disable_response["d"]["requestStatus"]["result"] is True
            await _expect_scene_item_enable_state_changed_event(
                ws,
                scene_name="Scene",
                scene_item_id=scene_item_id,
                scene_item_enabled=False,
            )

            disable_again_response = await _send_obsws_request(
                ws,
                request_type="SetSceneItemEnabled",
                request_id="req-set-scene-item-disabled-event-again",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                    "sceneItemEnabled": False,
                },
            )
            assert disable_again_response["d"]["requestStatus"]["result"] is True
            await _assert_no_message_within(ws, timeout=0.5)
            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False):
        asyncio.run(_run())


def test_obsws_scene_item_events_are_sent_when_scenes_subscription_enabled(
    binary_path: Path,
):
    """obsws が Scenes 購読時に Scene Item 作成・削除・並び替えイベントを送ることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()

    async def _run():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(
                ws,
                None,
                event_subscriptions=OBSWS_EVENT_SUB_SCENES,
            )

            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-input-scene-item-events",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "scene-item-events-input",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                    "sceneItemEnabled": False,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True
            source_uuid = create_input_response["d"]["responseData"]["inputUuid"]

            create_scene_item_first_response = await _send_obsws_request(
                ws,
                request_type="CreateSceneItem",
                request_id="req-create-scene-item-event-1",
                request_data={
                    "sceneName": "Scene",
                    "sourceUuid": source_uuid,
                    "sceneItemEnabled": True,
                },
            )
            assert create_scene_item_first_response["d"]["requestStatus"]["result"] is True
            first_scene_item_id = create_scene_item_first_response["d"]["responseData"][
                "sceneItemId"
            ]
            created_event_1 = await _expect_obsws_event(
                ws,
                event_type="SceneItemCreated",
                event_intent=OBSWS_EVENT_SUB_SCENES,
            )
            assert created_event_1["d"]["eventData"]["sceneItemId"] == first_scene_item_id

            create_scene_item_second_response = await _send_obsws_request(
                ws,
                request_type="CreateSceneItem",
                request_id="req-create-scene-item-event-2",
                request_data={
                    "sceneName": "Scene",
                    "sourceUuid": source_uuid,
                    "sceneItemEnabled": True,
                },
            )
            assert create_scene_item_second_response["d"]["requestStatus"]["result"] is True
            second_scene_item_id = create_scene_item_second_response["d"]["responseData"][
                "sceneItemId"
            ]
            created_event_2 = await _expect_obsws_event(
                ws,
                event_type="SceneItemCreated",
                event_intent=OBSWS_EVENT_SUB_SCENES,
            )
            assert created_event_2["d"]["eventData"]["sceneItemId"] == second_scene_item_id

            set_scene_item_index_response = await _send_obsws_request(
                ws,
                request_type="SetSceneItemIndex",
                request_id="req-set-scene-item-index-event",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": second_scene_item_id,
                    "sceneItemIndex": 0,
                },
            )
            assert set_scene_item_index_response["d"]["requestStatus"]["result"] is True
            reindexed_event_1 = await _expect_obsws_event(
                ws,
                event_type="SceneItemListReindexed",
                event_intent=OBSWS_EVENT_SUB_SCENES,
            )
            reindexed_ids_1 = [
                item["sceneItemId"] for item in reindexed_event_1["d"]["eventData"]["sceneItems"]
            ]
            assert reindexed_ids_1[0] == second_scene_item_id

            remove_scene_item_response = await _send_obsws_request(
                ws,
                request_type="RemoveSceneItem",
                request_id="req-remove-scene-item-event",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": second_scene_item_id,
                },
            )
            assert remove_scene_item_response["d"]["requestStatus"]["result"] is True
            removed_event = await _expect_obsws_event(
                ws,
                event_type="SceneItemRemoved",
                event_intent=OBSWS_EVENT_SUB_SCENES,
            )
            assert removed_event["d"]["eventData"]["sceneItemId"] == second_scene_item_id
            reindexed_event_2 = await _expect_obsws_event(
                ws,
                event_type="SceneItemListReindexed",
                event_intent=OBSWS_EVENT_SUB_SCENES,
            )
            reindexed_ids_2 = [
                item["sceneItemId"] for item in reindexed_event_2["d"]["eventData"]["sceneItems"]
            ]
            assert second_scene_item_id not in reindexed_ids_2

            duplicate_scene_item_response = await _send_obsws_request(
                ws,
                request_type="DuplicateSceneItem",
                request_id="req-duplicate-scene-item-event",
                request_data={
                    "fromSceneName": "Scene",
                    "toSceneName": "Scene",
                    "sceneItemId": first_scene_item_id,
                },
            )
            assert duplicate_scene_item_response["d"]["requestStatus"]["result"] is True
            duplicated_scene_item_id = duplicate_scene_item_response["d"]["responseData"][
                "sceneItemId"
            ]
            created_event_3 = await _expect_obsws_event(
                ws,
                event_type="SceneItemCreated",
                event_intent=OBSWS_EVENT_SUB_SCENES,
            )
            assert created_event_3["d"]["eventData"]["sceneItemId"] == duplicated_scene_item_id
            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False):
        asyncio.run(_run())


def test_obsws_scene_item_lock_and_transform_events_are_sent_when_scenes_subscription_enabled(
    binary_path: Path,
):
    """obsws が Scenes 購読時に Scene Item lock / transform イベントを送ることを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()

    async def _run():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(
                ws,
                None,
                event_subscriptions=OBSWS_EVENT_SUB_SCENES,
            )

            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-input-scene-item-lock-transform-events",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "scene-item-lock-transform-events-input",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True

            get_scene_item_id_response = await _send_obsws_request(
                ws,
                request_type="GetSceneItemId",
                request_id="req-get-scene-item-id-lock-transform-events",
                request_data={
                    "sceneName": "Scene",
                    "sourceName": "scene-item-lock-transform-events-input",
                    "searchOffset": 0,
                },
            )
            assert get_scene_item_id_response["d"]["requestStatus"]["result"] is True
            scene_item_id = get_scene_item_id_response["d"]["responseData"]["sceneItemId"]

            set_locked_response = await _send_obsws_request(
                ws,
                request_type="SetSceneItemLocked",
                request_id="req-set-scene-item-locked-event",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                    "sceneItemLocked": True,
                },
            )
            assert set_locked_response["d"]["requestStatus"]["result"] is True
            await _expect_scene_item_lock_state_changed_event(
                ws,
                scene_name="Scene",
                scene_item_id=scene_item_id,
                scene_item_locked=True,
            )

            set_locked_again_response = await _send_obsws_request(
                ws,
                request_type="SetSceneItemLocked",
                request_id="req-set-scene-item-locked-event-again",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                    "sceneItemLocked": True,
                },
            )
            assert set_locked_again_response["d"]["requestStatus"]["result"] is True
            await _assert_no_message_within(ws, timeout=0.5)

            set_transform_response = await _send_obsws_request(
                ws,
                request_type="SetSceneItemTransform",
                request_id="req-set-scene-item-transform-event",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                    "sceneItemTransform": {
                        "positionX": 99.0,
                    },
                },
            )
            assert set_transform_response["d"]["requestStatus"]["result"] is True
            transform_event = await _expect_scene_item_transform_changed_event(
                ws,
                scene_name="Scene",
                scene_item_id=scene_item_id,
            )
            event_transform = transform_event["d"]["eventData"]["sceneItemTransform"]
            assert event_transform["positionX"] == 99.0
            assert event_transform["positionY"] == 0.0
            assert event_transform["scaleX"] == 1.0
            assert "sourceWidth" in event_transform
            assert "sourceHeight" in event_transform

            set_transform_again_response = await _send_obsws_request(
                ws,
                request_type="SetSceneItemTransform",
                request_id="req-set-scene-item-transform-event-again",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": scene_item_id,
                    "sceneItemTransform": {
                        "positionX": 99.0,
                    },
                },
            )
            assert set_transform_again_response["d"]["requestStatus"]["result"] is True
            await _assert_no_message_within(ws, timeout=0.5)
            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False):
        asyncio.run(_run())


def test_obsws_remove_scene_item_tail_does_not_send_reindexed_event(
    binary_path: Path,
):
    """obsws が末尾 Scene Item 削除時に再インデックスイベントを送らないことを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()

    async def _run():
        timeout = aiohttp.ClientTimeout(total=20.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            ws = await session.ws_connect(
                f"ws://{host}:{ws_port}/",
                protocols=[OBSWS_SUBPROTOCOL],
            )
            await _identify_with_optional_password(
                ws,
                None,
                event_subscriptions=OBSWS_EVENT_SUB_SCENES,
            )

            create_input_response = await _send_obsws_request(
                ws,
                request_type="CreateInput",
                request_id="req-create-input-scene-item-tail-remove",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "scene-item-tail-remove-input",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                    "sceneItemEnabled": True,
                },
            )
            assert create_input_response["d"]["requestStatus"]["result"] is True
            source_uuid = create_input_response["d"]["responseData"]["inputUuid"]

            create_scene_item_second_response = await _send_obsws_request(
                ws,
                request_type="CreateSceneItem",
                request_id="req-create-scene-item-tail-remove-2",
                request_data={
                    "sceneName": "Scene",
                    "sourceUuid": source_uuid,
                    "sceneItemEnabled": True,
                },
            )
            assert create_scene_item_second_response["d"]["requestStatus"]["result"] is True
            second_scene_item_id = create_scene_item_second_response["d"]["responseData"][
                "sceneItemId"
            ]
            created_event = await _expect_obsws_event(
                ws,
                event_type="SceneItemCreated",
                event_intent=OBSWS_EVENT_SUB_SCENES,
            )
            assert created_event["d"]["eventData"]["sceneItemId"] == second_scene_item_id

            remove_scene_item_response = await _send_obsws_request(
                ws,
                request_type="RemoveSceneItem",
                request_id="req-remove-scene-item-tail-remove",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": second_scene_item_id,
                },
            )
            assert remove_scene_item_response["d"]["requestStatus"]["result"] is True
            removed_event = await _expect_obsws_event(
                ws,
                event_type="SceneItemRemoved",
                event_intent=OBSWS_EVENT_SUB_SCENES,
            )
            assert removed_event["d"]["eventData"]["sceneItemId"] == second_scene_item_id
            try:
                next_msg = await ws.receive(timeout=0.5)
            except asyncio.TimeoutError:
                await ws.close()
                return

            if next_msg.type == aiohttp.WSMsgType.TEXT:
                next_payload = json.loads(next_msg.data)
                is_reindexed_event = (
                    next_payload.get("op") == 5
                    and next_payload.get("d", {}).get("eventType")
                    == "SceneItemListReindexed"
                )
                assert not is_reindexed_event, (
                    "unexpected SceneItemListReindexed after tail remove: "
                    f"{next_payload}"
                )
                raise AssertionError(f"unexpected text message after tail remove: {next_payload}")
            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False):
        asyncio.run(_run())


def test_obsws_rejects_unsupported_rpc_version(binary_path: Path):
    """obsws が非対応 rpcVersion を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(
            _connect_and_expect_close_code(
                f"ws://{host}:{port}/",
                {"op": 1, "d": {"rpcVersion": 2}},
                4006,
            )
        )


def test_obsws_rejects_invalid_payload_message(binary_path: Path):
    """obsws が不正メッセージを invalid payload として拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        asyncio.run(
            _connect_and_expect_close_code(
                f"ws://{host}:{port}/",
                {"op": 999, "d": {}},
                1007,
            )
        )
