"""obsws state file の永続化 e2e テスト"""

import asyncio
import json
from pathlib import Path

import aiohttp

from hisui_server import reserve_ephemeral_port

from helpers import (
    OBSWS_SUBPROTOCOL,
    ObswsServer,
    _create_output,
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


def test_record_without_directory_causes_startup_failure(
    binary_path: Path, tmp_path: Path
):
    """record セクションに recordDirectory がない state file は起動に失敗する"""
    host = "127.0.0.1"
    state_file = tmp_path / "no-record-dir.jsonc"
    state_file.write_text(json.dumps({"version": 1, "record": {}}))

    port, sock = reserve_ephemeral_port()
    sock.close()

    try:
        with ObswsServer(binary_path, host=host, port=port, state_file=state_file):
            assert False, "server must not start with record section missing recordDirectory"
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
                "outputs": [
                    {
                        "outputName": "stream",
                        "outputKind": "rtmp_output",
                        "outputSettings": {
                            "streamServiceType": "rtmp_custom",
                            "streamServiceSettings": {
                                "server": "rtmp://preexisting-server/live",
                                "key": "preexisting-key",
                            },
                        },
                    },
                    {
                        "outputName": "record",
                        "outputKind": "mp4_output",
                        "outputSettings": {
                            "recordDirectory": str(
                                tmp_path / "preexisting-recordings"
                            ),
                        },
                    },
                ],
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


def test_rtmp_outbound_persists_across_restart(binary_path: Path, tmp_path: Path):
    """SetOutputSettings で rtmp_outbound を設定した値が再起動後も復元される"""
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
                await _create_output(ws, "rtmp_outbound", "rtmp_outbound_output")
                response = await _send_obsws_request(
                    ws,
                    "SetOutputSettings",
                    "set-rtmp-outbound",
                    {
                        "outputName": "rtmp_outbound",
                        "outputSettings": {
                            "outputUrl": "rtmp://relay:1935/live",
                            "streamName": "backup",
                        },
                    },
                )
                assert response["d"]["requestStatus"]["result"] is True
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
                response = await _send_obsws_request(
                    ws,
                    "GetOutputSettings",
                    "get-rtmp-outbound",
                    {"outputName": "rtmp_outbound"},
                )
                settings = response["d"]["responseData"]["outputSettings"]
                assert settings["outputUrl"] == "rtmp://relay:1935/live"
                assert settings["streamName"] == "backup"
                await ws.close()

        asyncio.run(_get())


def test_sora_persists_across_restart(binary_path: Path, tmp_path: Path):
    """SetOutputSettings で sora を設定した値が再起動後も復元される"""
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
                await _create_output(ws, "sora", "sora_webrtc_output")
                response = await _send_obsws_request(
                    ws,
                    "SetOutputSettings",
                    "set-sora",
                    {
                        "outputName": "sora",
                        "outputSettings": {
                            "soraSdkSettings": {
                                "signalingUrls": ["wss://example.com/signaling"],
                                "channelId": "test-ch",
                                "metadata": {"key": "value"},
                            }
                        },
                    },
                )
                assert response["d"]["requestStatus"]["result"] is True
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
                response = await _send_obsws_request(
                    ws,
                    "GetOutputSettings",
                    "get-sora",
                    {"outputName": "sora"},
                )
                settings = response["d"]["responseData"]["outputSettings"]
                sora = settings["soraSdkSettings"]
                assert sora["signalingUrls"] == ["wss://example.com/signaling"]
                assert sora["channelId"] == "test-ch"
                await ws.close()

        asyncio.run(_get())


def test_hls_filesystem_persists_across_restart(binary_path: Path, tmp_path: Path):
    """SetOutputSettings で hls filesystem を設定した値が再起動後も復元される"""
    host = "127.0.0.1"
    state_file = tmp_path / "state.jsonc"
    hls_dir = str(tmp_path / "hls-output")

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
                await _create_output(ws, "hls", "hls_output")
                response = await _send_obsws_request(
                    ws,
                    "SetOutputSettings",
                    "set-hls",
                    {
                        "outputName": "hls",
                        "outputSettings": {
                            "destination": {
                                "type": "filesystem",
                                "directory": hls_dir,
                            },
                            "segmentDuration": 3.0,
                            "maxRetainedSegments": 10,
                            "variants": [
                                {"videoBitrate": 1500000, "audioBitrate": 96000}
                            ],
                        },
                    },
                )
                assert response["d"]["requestStatus"]["result"] is True
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
                response = await _send_obsws_request(
                    ws,
                    "GetOutputSettings",
                    "get-hls",
                    {"outputName": "hls"},
                )
                settings = response["d"]["responseData"]["outputSettings"]
                assert settings["destination"]["type"] == "filesystem"
                assert settings["destination"]["directory"] == hls_dir
                assert settings["segmentDuration"] == 3.0
                assert settings["maxRetainedSegments"] == 10
                assert settings["variants"][0]["videoBitrate"] == 1500000
                await ws.close()

        asyncio.run(_get())


def test_mpeg_dash_filesystem_persists_across_restart(binary_path: Path, tmp_path: Path):
    """SetOutputSettings で mpeg_dash filesystem を設定した値が再起動後も復元される"""
    host = "127.0.0.1"
    state_file = tmp_path / "state.jsonc"
    dash_dir = str(tmp_path / "dash-output")

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
                await _create_output(ws, "mpeg_dash", "mpeg_dash_output")
                response = await _send_obsws_request(
                    ws,
                    "SetOutputSettings",
                    "set-dash",
                    {
                        "outputName": "mpeg_dash",
                        "outputSettings": {
                            "destination": {
                                "type": "filesystem",
                                "directory": dash_dir,
                            },
                            "segmentDuration": 4.0,
                            "variants": [
                                {"videoBitrate": 3000000, "audioBitrate": 192000}
                            ],
                            "videoCodec": "H265",
                            "audioCodec": "OPUS",
                        },
                    },
                )
                assert response["d"]["requestStatus"]["result"] is True
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
                response = await _send_obsws_request(
                    ws,
                    "GetOutputSettings",
                    "get-dash",
                    {"outputName": "mpeg_dash"},
                )
                settings = response["d"]["responseData"]["outputSettings"]
                assert settings["destination"]["type"] == "filesystem"
                assert settings["destination"]["directory"] == dash_dir
                assert settings["segmentDuration"] == 4.0
                assert settings["variants"][0]["videoBitrate"] == 3000000
                assert settings["videoCodec"] == "H265"
                assert settings["audioCodec"] == "OPUS"
                await ws.close()

        asyncio.run(_get())


def test_scene_persists_across_restart(binary_path: Path, tmp_path: Path):
    """CreateScene で作成した scene が再起動後も復元され、sceneUuid が保持される"""
    host = "127.0.0.1"
    state_file = tmp_path / "state.jsonc"

    port, sock = reserve_ephemeral_port()
    sock.close()

    # 1 回目の起動: scene を作成し、sceneUuid を記録する
    created = {}
    with ObswsServer(binary_path, host=host, port=port, state_file=state_file):

        async def _set():
            timeout = aiohttp.ClientTimeout(total=10.0)
            async with aiohttp.ClientSession(timeout=timeout) as session:
                ws = await session.ws_connect(
                    f"ws://{host}:{port}/", protocols=[OBSWS_SUBPROTOCOL]
                )
                await _identify_with_optional_password(ws, password=None)
                # scene を作成
                response = await _send_obsws_request(
                    ws,
                    "CreateScene",
                    "create-scene",
                    {"sceneName": "MyScene"},
                )
                assert response["d"]["requestStatus"]["result"] is True
                created["sceneUuid"] = response["d"]["responseData"]["sceneUuid"]
                # program scene を変更
                response = await _send_obsws_request(
                    ws,
                    "SetCurrentProgramScene",
                    "set-program",
                    {"sceneName": "MyScene"},
                )
                assert response["d"]["requestStatus"]["result"] is True
                await ws.close()

        asyncio.run(_set())

    assert created["sceneUuid"], "sceneUuid must be captured"

    # 2 回目の起動: sceneUuid が同一であることを確認する
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
                response = await _send_obsws_request(ws, "GetSceneList", "get-scenes")
                data = response["d"]["responseData"]
                scene_names = [s["sceneName"] for s in data["scenes"]]
                assert "Scene" in scene_names
                assert "MyScene" in scene_names
                assert data["currentProgramSceneName"] == "MyScene"
                # sceneUuid の保持を確認
                my_scene = [s for s in data["scenes"] if s["sceneName"] == "MyScene"]
                assert len(my_scene) == 1
                assert my_scene[0]["sceneUuid"] == created["sceneUuid"]
                await ws.close()

        asyncio.run(_get())


def test_input_persists_across_restart(binary_path: Path, tmp_path: Path):
    """CreateInput で作成した input が再起動後も復元され、UUID と sceneItemId が保持される"""
    host = "127.0.0.1"
    state_file = tmp_path / "state.jsonc"

    port, sock = reserve_ephemeral_port()
    sock.close()

    # 1 回目の起動: input を作成し、UUID と sceneItemId を記録する
    created = {}
    with ObswsServer(binary_path, host=host, port=port, state_file=state_file):

        async def _set():
            timeout = aiohttp.ClientTimeout(total=10.0)
            async with aiohttp.ClientSession(timeout=timeout) as session:
                ws = await session.ws_connect(
                    f"ws://{host}:{port}/", protocols=[OBSWS_SUBPROTOCOL]
                )
                await _identify_with_optional_password(ws, password=None)
                response = await _send_obsws_request(
                    ws,
                    "CreateInput",
                    "create-input",
                    {
                        "sceneName": "Scene",
                        "inputName": "test-image",
                        "inputKind": "image_source",
                        "inputSettings": {"file": "/tmp/test.png"},
                    },
                )
                assert response["d"]["requestStatus"]["result"] is True
                data = response["d"]["responseData"]
                created["inputUuid"] = data["inputUuid"]
                created["sceneItemId"] = data["sceneItemId"]
                await ws.close()

        asyncio.run(_set())

    assert created["inputUuid"], "inputUuid must be captured"

    # 2 回目の起動: UUID と sceneItemId が同一であることを確認する
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
                # inputUuid の保持を確認
                response = await _send_obsws_request(ws, "GetInputList", "get-inputs")
                inputs = response["d"]["responseData"]["inputs"]
                target = [i for i in inputs if i["inputName"] == "test-image"]
                assert len(target) == 1
                assert target[0]["inputUuid"] == created["inputUuid"]
                # sceneItemId の保持を確認
                response = await _send_obsws_request(
                    ws,
                    "GetSceneItemList",
                    "get-items",
                    {"sceneName": "Scene"},
                )
                items = response["d"]["responseData"]["sceneItems"]
                target_items = [i for i in items if i["sourceName"] == "test-image"]
                assert len(target_items) == 1
                assert target_items[0]["sceneItemId"] == created["sceneItemId"]
                await ws.close()

        asyncio.run(_get())


def test_scene_item_enabled_persists_across_restart(binary_path: Path, tmp_path: Path):
    """SetSceneItemEnabled の変更が再起動後も復元される"""
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
                # input を作成
                response = await _send_obsws_request(
                    ws,
                    "CreateInput",
                    "create-input",
                    {
                        "sceneName": "Scene",
                        "inputName": "disable-test",
                        "inputKind": "image_source",
                        "inputSettings": {},
                    },
                )
                assert response["d"]["requestStatus"]["result"] is True
                scene_item_id = response["d"]["responseData"]["sceneItemId"]
                # scene item を無効にする
                response = await _send_obsws_request(
                    ws,
                    "SetSceneItemEnabled",
                    "disable-item",
                    {
                        "sceneName": "Scene",
                        "sceneItemId": scene_item_id,
                        "sceneItemEnabled": False,
                    },
                )
                assert response["d"]["requestStatus"]["result"] is True
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
                response = await _send_obsws_request(
                    ws,
                    "GetSceneItemList",
                    "get-items",
                    {"sceneName": "Scene"},
                )
                items = response["d"]["responseData"]["sceneItems"]
                target = [i for i in items if i["sourceName"] == "disable-test"]
                assert len(target) == 1
                assert target[0]["sceneItemEnabled"] is False
                await ws.close()

        asyncio.run(_get())
