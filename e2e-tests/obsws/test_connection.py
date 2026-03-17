"""obsws の接続・認証・プロトコルエラーに関する e2e テスト"""

import asyncio
from pathlib import Path

import aiohttp
import pytest

from helpers import (
    ObswsServer,
    _connect_and_exchange_identify,
    _connect_and_exchange_identify_with_password,
    _connect_and_expect_close_code,
    _connect_and_send_duplicate_identify,
    _connect_and_send_invalid_password_auth,
    _connect_and_send_missing_password_auth,
    _connect_identify_and_expect_close_code,
    _connect_identify_and_request,
    _connect_identify_and_send_reidentify_then_request,
    _connect_websocket,
)
from hisui_server import reserve_ephemeral_port


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
