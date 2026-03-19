"""obsws のイベント通知に関する e2e テスト"""

import asyncio
import json
from pathlib import Path

import aiohttp

from helpers import (
    OBSWS_EVENT_SUB_INPUTS,
    OBSWS_EVENT_SUB_OUTPUTS,
    OBSWS_EVENT_SUB_SCENE_ITEMS,
    OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED,
    OBSWS_EVENT_SUB_SCENES,
    OBSWS_SUBPROTOCOL,
    ObswsServer,
    _assert_no_message_within,
    _expect_input_name_changed_event,
    _expect_input_settings_changed_event,
    _expect_obsws_event,
    _expect_record_state_changed_event,
    _expect_scene_item_enable_state_changed_event,
    _expect_scene_item_lock_state_changed_event,
    _expect_scene_item_transform_changed_event,
    _expect_stream_state_changed_event,
    _identify_with_optional_password,
    _send_obsws_request,
    _setup_stream_input_and_service,
    _write_test_png,
)
from hisui_server import reserve_ephemeral_port


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
            await _expect_record_state_changed_event(
                ws,
                output_active=True,
                output_state="OBS_WEBSOCKET_OUTPUT_STARTED",
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
                output_state="OBS_WEBSOCKET_OUTPUT_STOPPED",
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
                output_state="OBS_WEBSOCKET_OUTPUT_STARTED",
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
                output_state="OBS_WEBSOCKET_OUTPUT_STOPPED",
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

            set_preview_scene_response = await _send_obsws_request(
                ws,
                request_type="SetCurrentPreviewScene",
                request_id="req-set-current-preview-scene-events",
                request_data={"sceneName": "Scene B"},
            )
            # スタジオモードが無効のためエラーになる
            assert set_preview_scene_response["d"]["requestStatus"]["result"] is False
            assert set_preview_scene_response["d"]["requestStatus"]["code"] == 506

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
            assert "unversionedInputKind" in create_event["d"]["eventData"]
            assert "inputKindCaps" in create_event["d"]["eventData"]
            assert "inputSettings" in create_event["d"]["eventData"]
            assert "defaultInputSettings" in create_event["d"]["eventData"]

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
            assert "unversionedInputKind" in create_event_2["d"]["eventData"]
            assert "inputKindCaps" in create_event_2["d"]["eventData"]
            assert "inputSettings" in create_event_2["d"]["eventData"]
            assert "defaultInputSettings" in create_event_2["d"]["eventData"]

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
    """obsws が SceneItems 購読時に SetSceneItemEnabled のイベントを送ることを確認する"""
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
                event_subscriptions=OBSWS_EVENT_SUB_SCENE_ITEMS,
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
            # CreateInput 時に SceneItemCreated イベントが送信されるので消費する
            await _expect_obsws_event(
                ws,
                event_type="SceneItemCreated",
                event_intent=OBSWS_EVENT_SUB_SCENE_ITEMS,
            )

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
    """obsws が SceneItems 購読時に Scene Item 作成・削除・並び替えイベントを送ることを確認する"""
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
                event_subscriptions=OBSWS_EVENT_SUB_SCENE_ITEMS,
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
            # CreateInput 時に SceneItemCreated イベントが送信されるので消費する
            await _expect_obsws_event(
                ws,
                event_type="SceneItemCreated",
                event_intent=OBSWS_EVENT_SUB_SCENE_ITEMS,
            )

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
                event_intent=OBSWS_EVENT_SUB_SCENE_ITEMS,
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
                event_intent=OBSWS_EVENT_SUB_SCENE_ITEMS,
            )
            assert created_event_2["d"]["eventData"]["sceneItemId"] == second_scene_item_id

            # insert(0) で追加されるため second が index=0、first が index=1
            # first を index=0 に移動して再インデックスイベントを確認する
            set_scene_item_index_response = await _send_obsws_request(
                ws,
                request_type="SetSceneItemIndex",
                request_id="req-set-scene-item-index-event",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": first_scene_item_id,
                    "sceneItemIndex": 0,
                },
            )
            assert set_scene_item_index_response["d"]["requestStatus"]["result"] is True
            reindexed_event_1 = await _expect_obsws_event(
                ws,
                event_type="SceneItemListReindexed",
                event_intent=OBSWS_EVENT_SUB_SCENE_ITEMS,
            )
            reindexed_ids_1 = [
                item["sceneItemId"] for item in reindexed_event_1["d"]["eventData"]["sceneItems"]
            ]
            assert reindexed_ids_1[0] == first_scene_item_id

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
                event_intent=OBSWS_EVENT_SUB_SCENE_ITEMS,
            )
            assert removed_event["d"]["eventData"]["sceneItemId"] == second_scene_item_id
            reindexed_event_2 = await _expect_obsws_event(
                ws,
                event_type="SceneItemListReindexed",
                event_intent=OBSWS_EVENT_SUB_SCENE_ITEMS,
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
                    "sceneName": "Scene",
                    "destinationSceneName": "Scene",
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
                event_intent=OBSWS_EVENT_SUB_SCENE_ITEMS,
            )
            assert created_event_3["d"]["eventData"]["sceneItemId"] == duplicated_scene_item_id
            await ws.close()

    with ObswsServer(binary_path, host=host, port=ws_port, use_env=False):
        asyncio.run(_run())


def test_obsws_scene_item_lock_and_transform_events_are_sent_when_scenes_subscription_enabled(
    binary_path: Path,
):
    """obsws が SceneItems / SceneItemTransformChanged 購読時に Scene Item lock / transform イベントを送ることを確認する"""
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
                event_subscriptions=OBSWS_EVENT_SUB_SCENE_ITEMS | OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED,
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
            # CreateInput 時に SceneItemCreated イベントが送信されるので消費する
            await _expect_obsws_event(
                ws,
                event_type="SceneItemCreated",
                event_intent=OBSWS_EVENT_SUB_SCENE_ITEMS,
            )

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
                event_subscriptions=OBSWS_EVENT_SUB_SCENE_ITEMS,
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
            # CreateInput の sceneItemId は末尾確認に使用する
            first_scene_item_id = create_input_response["d"]["responseData"]["sceneItemId"]
            # CreateInput 時に SceneItemCreated イベントが送信されるので消費する
            await _expect_obsws_event(
                ws,
                event_type="SceneItemCreated",
                event_intent=OBSWS_EVENT_SUB_SCENE_ITEMS,
            )

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
                event_intent=OBSWS_EVENT_SUB_SCENE_ITEMS,
            )
            assert created_event["d"]["eventData"]["sceneItemId"] == second_scene_item_id

            # insert(0) で追加されるため、second が index=0、first が index=1（末尾）
            # 末尾のアイテムを削除して再インデックスイベントが送信されないことを確認する
            remove_scene_item_response = await _send_obsws_request(
                ws,
                request_type="RemoveSceneItem",
                request_id="req-remove-scene-item-tail-remove",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": first_scene_item_id,
                },
            )
            assert remove_scene_item_response["d"]["requestStatus"]["result"] is True
            removed_event = await _expect_obsws_event(
                ws,
                event_type="SceneItemRemoved",
                event_intent=OBSWS_EVENT_SUB_SCENE_ITEMS,
            )
            assert removed_event["d"]["eventData"]["sceneItemId"] == first_scene_item_id
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
