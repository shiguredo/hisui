"""obsws state file の永続化 e2e テスト"""

import asyncio
import json
from pathlib import Path

import aiohttp

from hisui_server import reserve_ephemeral_port

from helpers import (
    OBSWS_SUBPROTOCOL,
    ObswsServer,
    _identify_with_optional_password,
    _send_obsws_request,
)


async def _get_stream_service_settings(ws: aiohttp.ClientWebSocketResponse):
    """GetStreamServiceSettings を送信し、設定値を返す"""
    response = await _send_obsws_request(ws, "GetStreamServiceSettings", "get-stream")
    data = response["d"]["responseData"]
    return data


async def _set_stream_service_settings(
    ws: aiohttp.ClientWebSocketResponse,
    server: str,
    key: str | None = None,
):
    """SetStreamServiceSettings を送信する"""
    settings: dict[str, object] = {"server": server}
    if key is not None:
        settings["key"] = key
    response = await _send_obsws_request(
        ws,
        "SetStreamServiceSettings",
        "set-stream",
        {
            "streamServiceType": "rtmp_custom",
            "streamServiceSettings": settings,
        },
    )
    assert response["d"]["requestStatus"]["result"] is True
    return response


async def _get_record_directory(ws: aiohttp.ClientWebSocketResponse):
    """GetRecordDirectory を送信し、値を返す"""
    response = await _send_obsws_request(ws, "GetRecordDirectory", "get-record-dir")
    return response["d"]["responseData"]["recordDirectory"]


async def _set_record_directory(ws: aiohttp.ClientWebSocketResponse, directory: str):
    """SetRecordDirectory を送信する"""
    response = await _send_obsws_request(
        ws,
        "SetRecordDirectory",
        "set-record-dir",
        {"recordDirectory": directory},
    )
    assert response["d"]["requestStatus"]["result"] is True
    return response


async def _set_output_settings_stream(
    ws: aiohttp.ClientWebSocketResponse,
    server: str,
    key: str | None = None,
):
    """SetOutputSettings で stream を更新する"""
    settings: dict[str, object] = {"server": server}
    if key is not None:
        settings["key"] = key
    response = await _send_obsws_request(
        ws,
        "SetOutputSettings",
        "set-output-stream",
        {
            "outputName": "stream",
            "outputSettings": {
                "streamServiceType": "rtmp_custom",
                "streamServiceSettings": settings,
            },
        },
    )
    assert response["d"]["requestStatus"]["result"] is True
    return response


def test_stream_settings_persist_across_restart(binary_path: Path, tmp_path: Path):
    """SetStreamServiceSettings で設定した値が再起動後も復元される"""
    host = "127.0.0.1"
    state_file = tmp_path / "state.jsonc"

    port, sock = reserve_ephemeral_port()
    sock.close()

    # 1 回目の起動: 設定を変更する
    with ObswsServer(binary_path, host=host, port=port, state_file=state_file):

        async def _set():
            timeout = aiohttp.ClientTimeout(total=10.0)
            async with aiohttp.ClientSession(timeout=timeout) as session:
                ws = await session.ws_connect(
                    f"ws://{host}:{port}/", protocols=[OBSWS_SUBPROTOCOL]
                )
                await _identify_with_optional_password(ws, password=None)
                await _set_stream_service_settings(
                    ws, server="rtmp://test-server/live", key="test-key"
                )
                await ws.close()

        asyncio.run(_set())

    # state file が作成されたことを確認する
    assert state_file.exists(), "state file must be created after SetStreamServiceSettings"

    # 2 回目の起動: 値が復元されることを確認する
    port2, sock2 = reserve_ephemeral_port()
    sock2.close()

    with ObswsServer(binary_path, host=host, port=port2, state_file=state_file):

        async def _get():
            timeout = aiohttp.ClientTimeout(total=10.0)
            async with aiohttp.ClientSession(timeout=timeout) as session:
                ws = await session.ws_connect(
                    f"ws://{host}:{port2}/", protocols=[OBSWS_SUBPROTOCOL]
                )
                await _identify_with_optional_password(ws, password=None)
                data = await _get_stream_service_settings(ws)
                assert data["streamServiceSettings"]["server"] == "rtmp://test-server/live"
                assert data["streamServiceSettings"]["key"] == "test-key"
                await ws.close()

        asyncio.run(_get())


def test_record_directory_persists_across_restart(binary_path: Path, tmp_path: Path):
    """SetRecordDirectory で設定した値が再起動後も復元される"""
    host = "127.0.0.1"
    state_file = tmp_path / "state.jsonc"
    record_dir = str(tmp_path / "my-recordings")

    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(binary_path, host=host, port=port, state_file=state_file):

        async def _set():
            timeout = aiohttp.ClientTimeout(total=10.0)
            async with aiohttp.ClientSession(timeout=timeout) as session:
                ws = await session.ws_connect(
                    f"ws://{host}:{port}/", protocols=[OBSWS_SUBPROTOCOL]
                )
                await _identify_with_optional_password(ws, password=None)
                await _set_record_directory(ws, record_dir)
                await ws.close()

        asyncio.run(_set())

    port2, sock2 = reserve_ephemeral_port()
    sock2.close()

    with ObswsServer(binary_path, host=host, port=port2, state_file=state_file):

        async def _get():
            timeout = aiohttp.ClientTimeout(total=10.0)
            async with aiohttp.ClientSession(timeout=timeout) as session:
                ws = await session.ws_connect(
                    f"ws://{host}:{port2}/", protocols=[OBSWS_SUBPROTOCOL]
                )
                await _identify_with_optional_password(ws, password=None)
                result = await _get_record_directory(ws)
                assert result == record_dir
                await ws.close()

        asyncio.run(_get())


def test_set_output_settings_stream_persists(binary_path: Path, tmp_path: Path):
    """SetOutputSettings 経由で stream を変更した値が再起動後も復元される"""
    host = "127.0.0.1"
    state_file = tmp_path / "state.jsonc"

    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(binary_path, host=host, port=port, state_file=state_file):

        async def _set():
            timeout = aiohttp.ClientTimeout(total=10.0)
            async with aiohttp.ClientSession(timeout=timeout) as session:
                ws = await session.ws_connect(
                    f"ws://{host}:{port}/", protocols=[OBSWS_SUBPROTOCOL]
                )
                await _identify_with_optional_password(ws, password=None)
                await _set_output_settings_stream(
                    ws, server="rtmp://output-test/live", key="output-key"
                )
                await ws.close()

        asyncio.run(_set())

    port2, sock2 = reserve_ephemeral_port()
    sock2.close()

    with ObswsServer(binary_path, host=host, port=port2, state_file=state_file):

        async def _get():
            timeout = aiohttp.ClientTimeout(total=10.0)
            async with aiohttp.ClientSession(timeout=timeout) as session:
                ws = await session.ws_connect(
                    f"ws://{host}:{port2}/", protocols=[OBSWS_SUBPROTOCOL]
                )
                await _identify_with_optional_password(ws, password=None)
                data = await _get_stream_service_settings(ws)
                assert data["streamServiceSettings"]["server"] == "rtmp://output-test/live"
                assert data["streamServiceSettings"]["key"] == "output-key"
                await ws.close()

        asyncio.run(_get())


def test_no_state_file_means_no_persistence(binary_path: Path, tmp_path: Path):
    """state file 未指定では再起動後に値が復元されない"""
    host = "127.0.0.1"

    port, sock = reserve_ephemeral_port()
    sock.close()

    # state_file を指定しない
    with ObswsServer(binary_path, host=host, port=port):

        async def _set():
            timeout = aiohttp.ClientTimeout(total=10.0)
            async with aiohttp.ClientSession(timeout=timeout) as session:
                ws = await session.ws_connect(
                    f"ws://{host}:{port}/", protocols=[OBSWS_SUBPROTOCOL]
                )
                await _identify_with_optional_password(ws, password=None)
                await _set_stream_service_settings(
                    ws, server="rtmp://ephemeral/live", key="eph-key"
                )
                await ws.close()

        asyncio.run(_set())

    port2, sock2 = reserve_ephemeral_port()
    sock2.close()

    with ObswsServer(binary_path, host=host, port=port2):

        async def _get():
            timeout = aiohttp.ClientTimeout(total=10.0)
            async with aiohttp.ClientSession(timeout=timeout) as session:
                ws = await session.ws_connect(
                    f"ws://{host}:{port2}/", protocols=[OBSWS_SUBPROTOCOL]
                )
                await _identify_with_optional_password(ws, password=None)
                data = await _get_stream_service_settings(ws)
                # state file 未指定なので server はデフォルト値（未設定）になるはず
                settings = data["streamServiceSettings"]
                assert settings.get("server") is None or "ephemeral" not in settings.get(
                    "server", ""
                )
                await ws.close()

        asyncio.run(_get())


def test_corrupted_state_file_causes_startup_failure(binary_path: Path, tmp_path: Path):
    """壊れた state file を指定すると起動に失敗する"""
    host = "127.0.0.1"
    state_file = tmp_path / "corrupted.jsonc"
    state_file.write_text("{ invalid json content !!!")

    port, sock = reserve_ephemeral_port()
    sock.close()

    try:
        with ObswsServer(binary_path, host=host, port=port, state_file=state_file):
            # ここに到達した場合はテスト失敗
            assert False, "server must not start with corrupted state file"
    except AssertionError as e:
        # ObswsServer._wait_until_listening で起動前に終了を検知する
        assert "exited before listening" in str(e)


def test_invalid_version_state_file_causes_startup_failure(
    binary_path: Path, tmp_path: Path
):
    """version が 1 以外の state file を指定すると起動に失敗する"""
    host = "127.0.0.1"
    state_file = tmp_path / "bad-version.jsonc"
    state_file.write_text(json.dumps({"version": 99}))

    port, sock = reserve_ephemeral_port()
    sock.close()

    try:
        with ObswsServer(binary_path, host=host, port=port, state_file=state_file):
            assert False, "server must not start with invalid version state file"
    except AssertionError as e:
        assert "exited before listening" in str(e)


def test_preexisting_state_file_is_loaded_on_startup(
    binary_path: Path, tmp_path: Path
):
    """事前に作成された state file の値で起動時に初期化される"""
    host = "127.0.0.1"
    state_file = tmp_path / "preexisting.jsonc"
    state_file.write_text(
        json.dumps(
            {
                "version": 1,
                "stream": {
                    "streamServiceType": "rtmp_custom",
                    "streamServiceSettings": {
                        "server": "rtmp://preexisting-server/live",
                        "key": "preexisting-key",
                    },
                },
                "record": {
                    "recordDirectory": str(tmp_path / "preexisting-recordings"),
                },
            }
        )
    )

    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(binary_path, host=host, port=port, state_file=state_file):

        async def _get():
            timeout = aiohttp.ClientTimeout(total=10.0)
            async with aiohttp.ClientSession(timeout=timeout) as session:
                ws = await session.ws_connect(
                    f"ws://{host}:{port}/", protocols=[OBSWS_SUBPROTOCOL]
                )
                await _identify_with_optional_password(ws, password=None)

                # stream 設定の確認
                data = await _get_stream_service_settings(ws)
                assert (
                    data["streamServiceSettings"]["server"]
                    == "rtmp://preexisting-server/live"
                )
                assert data["streamServiceSettings"]["key"] == "preexisting-key"

                # record ディレクトリの確認
                record_dir = await _get_record_directory(ws)
                assert record_dir == str(tmp_path / "preexisting-recordings")

                await ws.close()

        asyncio.run(_get())
