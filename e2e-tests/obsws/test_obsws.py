"""obsws サブコマンドの e2e テスト"""

import asyncio
import base64
import hashlib
import json
import os
import signal
import socket
import subprocess
import time
from pathlib import Path

import aiohttp
import pytest

from hisui_server import reserve_ephemeral_port

OBSWS_SUBPROTOCOL = "obswebsocket.json"


class ObswsServer:
    """obsws サブコマンドプロセスを管理するテスト補助クラス"""

    def __init__(
        self,
        binary_path: Path,
        *,
        host: str,
        port: int,
        password: str | None = None,
        use_env: bool = False,
    ):
        self.binary_path = binary_path
        self.host = host
        self.port = port
        self.password = password
        self.use_env = use_env
        self._process: subprocess.Popen[None] | None = None

    def __enter__(self):
        return self.start()

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.stop()

    def start(self):
        if self._process is not None:
            raise RuntimeError("obsws server is already started")

        cmd = [str(self.binary_path), "--experimental", "obsws"]
        env = os.environ.copy()
        if self.use_env:
            env["HISUI_OBSWS_HOST"] = self.host
            env["HISUI_OBSWS_PORT"] = str(self.port)
            if self.password is not None:
                env["HISUI_OBSWS_PASSWORD"] = self.password
        else:
            cmd.extend(["--host", self.host, "--port", str(self.port)])
            if self.password is not None:
                cmd.extend(["--password", self.password])

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
            try:
                with socket.create_connection((self.host, self.port), timeout=0.5):
                    return
            except OSError:
                time.sleep(0.1)
        raise AssertionError(
            f"obsws server did not start listening in time: host={self.host}, port={self.port}"
        )


async def _connect_websocket(url: str):
    timeout = aiohttp.ClientTimeout(total=10.0)
    async with aiohttp.ClientSession(timeout=timeout) as session:
        ws = await session.ws_connect(url, protocols=[OBSWS_SUBPROTOCOL])
        await ws.close()


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
):
    hello_msg = await ws.receive(timeout=5.0)
    assert hello_msg.type == aiohttp.WSMsgType.TEXT
    hello = json.loads(hello_msg.data)
    assert hello["op"] == 0
    hello_data = hello["d"]
    assert hello_data["rpcVersion"] == 1

    identify_data: dict[str, object] = {"rpcVersion": 1}
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
                    "d": {"requestType": "GetVersion", "requestId": "req-before-identify"},
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
