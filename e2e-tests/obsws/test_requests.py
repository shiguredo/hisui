"""obsws の一般リクエスト・レスポンスに関する e2e テスト"""

import asyncio
from pathlib import Path

import aiohttp

from helpers import (
    OBSWS_SUBPROTOCOL,
    ObswsServer,
    _connect_identify_and_request,
    _identify_with_optional_password,
    _send_obsws_request,
    _write_test_png,
)
from hisui_server import reserve_ephemeral_port


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
        assert "GetInputPropertiesListPropertyItems" in response_data["availableRequests"]
        assert "CreateInput" in response_data["availableRequests"]
        assert "RemoveInput" in response_data["availableRequests"]
        assert "RemoveScene" in response_data["availableRequests"]
        assert "GetSceneList" in response_data["availableRequests"]
        assert "GetCurrentPreviewScene" in response_data["availableRequests"]
        assert "SetCurrentPreviewScene" in response_data["availableRequests"]
        assert "GetTransitionKindList" in response_data["availableRequests"]
        assert "GetSceneTransitionList" in response_data["availableRequests"]
        assert "GetCurrentSceneTransition" in response_data["availableRequests"]
        assert "SetCurrentSceneTransition" in response_data["availableRequests"]
        assert "SetCurrentSceneTransitionDuration" in response_data["availableRequests"]
        assert "SetCurrentSceneTransitionSettings" in response_data["availableRequests"]
        assert "GetCurrentSceneTransitionCursor" in response_data["availableRequests"]
        assert "SetTBarPosition" in response_data["availableRequests"]
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
        assert len(response_data["canvases"]) > 0
        canvas = response_data["canvases"][0]
        assert "canvasName" in canvas
        assert "canvasUuid" in canvas
        assert "canvasFlags" in canvas
        assert "canvasVideoSettings" in canvas
        video_settings = canvas["canvasVideoSettings"]
        assert "baseWidth" in video_settings
        assert "baseHeight" in video_settings
        assert "outputWidth" in video_settings
        assert "outputHeight" in video_settings
        assert "fpsNumerator" in video_settings
        assert "fpsDenominator" in video_settings


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


def test_obsws_transition_requests(binary_path: Path):
    """obsws が Transition 関連 request に応答することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        kind_list_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetTransitionKindList",
                request_id="req-get-transition-kind-list",
            )
        )
        assert kind_list_response["d"]["requestStatus"]["result"] is True
        transition_kinds = kind_list_response["d"]["responseData"]["transitionKinds"]
        assert "cut_transition" in transition_kinds
        assert "fade_transition" in transition_kinds

        transition_list_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetSceneTransitionList",
                request_id="req-get-scene-transition-list",
            )
        )
        assert transition_list_response["d"]["requestStatus"]["result"] is True
        assert (
            transition_list_response["d"]["responseData"]["currentSceneTransitionName"]
            == "fade_transition"
        )
        assert (
            transition_list_response["d"]["responseData"]["currentSceneTransitionKind"]
            == "fade_transition"
        )
        transition_entries = transition_list_response["d"]["responseData"]["transitions"]
        cut_transition = next(
            t for t in transition_entries if t["transitionName"] == "cut_transition"
        )
        fade_transition = next(
            t for t in transition_entries if t["transitionName"] == "fade_transition"
        )
        assert cut_transition["transitionFixed"] is True
        assert fade_transition["transitionFixed"] is False

        set_transition_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetCurrentSceneTransition",
                request_id="req-set-current-scene-transition",
                request_data={"transitionName": "fade_transition"},
            )
        )
        assert set_transition_response["d"]["requestStatus"]["result"] is True

        set_transition_duration_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetCurrentSceneTransitionDuration",
                request_id="req-set-current-scene-transition-duration",
                request_data={"transitionDuration": 500},
            )
        )
        assert set_transition_duration_response["d"]["requestStatus"]["result"] is True

        get_current_transition_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetCurrentSceneTransition",
                request_id="req-get-current-scene-transition",
            )
        )
        assert get_current_transition_response["d"]["requestStatus"]["result"] is True
        assert (
            get_current_transition_response["d"]["responseData"]["transitionName"]
            == "fade_transition"
        )
        assert (
            get_current_transition_response["d"]["responseData"]["transitionDuration"]
            == 500
        )
        assert get_current_transition_response["d"]["responseData"]["transitionFixed"] is False
        # fade_transition は transitionSettings を初期状態では null で返す
        assert get_current_transition_response["d"]["responseData"][
            "transitionSettings"
        ] is None

        # ビルトイントランジションはカスタム設定をサポートしないので失敗する
        set_transition_settings_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetCurrentSceneTransitionSettings",
                request_id="req-set-current-scene-transition-settings",
                request_data={"transitionSettings": {"curve": "ease_in_out", "power": 2}},
            )
        )
        assert set_transition_settings_response["d"]["requestStatus"]["result"] is False
        assert set_transition_settings_response["d"]["requestStatus"]["code"] == 606

        get_transition_cursor_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetCurrentSceneTransitionCursor",
                request_id="req-get-current-scene-transition-cursor",
            )
        )
        assert get_transition_cursor_response["d"]["requestStatus"]["result"] is True
        assert (
            get_transition_cursor_response["d"]["responseData"]["transitionCursor"] == 0.0
        )

        set_tbar_position_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetTBarPosition",
                request_id="req-set-tbar-position",
                request_data={"position": 0.25},
            )
        )
        assert set_tbar_position_response["d"]["requestStatus"]["result"] is False
        assert set_tbar_position_response["d"]["requestStatus"]["code"] == 506

        get_transition_cursor_after_tbar_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetCurrentSceneTransitionCursor",
                request_id="req-get-current-scene-transition-cursor-after-tbar",
            )
        )
        assert get_transition_cursor_after_tbar_response["d"]["requestStatus"]["result"] is True
        assert (
            get_transition_cursor_after_tbar_response["d"]["responseData"][
                "transitionCursor"
            ]
            == 0.0
        )

        invalid_transition_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetCurrentSceneTransition",
                request_id="req-set-current-scene-transition-invalid-name",
                request_data={"transitionName": "Swipe"},
            )
        )
        assert invalid_transition_response["d"]["requestStatus"]["result"] is False
        assert invalid_transition_response["d"]["requestStatus"]["code"] == 601

        invalid_duration_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetCurrentSceneTransitionDuration",
                request_id="req-set-current-scene-transition-invalid-duration",
                request_data={"transitionDuration": 0},
            )
        )
        assert invalid_duration_response["d"]["requestStatus"]["result"] is False
        assert invalid_duration_response["d"]["requestStatus"]["code"] == 400

        # Studio Mode 無効のため、不正な position でも 506 を返す
        invalid_tbar_position_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetTBarPosition",
                request_id="req-set-tbar-position-invalid",
                request_data={"position": 1.5},
            )
        )
        assert invalid_tbar_position_response["d"]["requestStatus"]["result"] is False
        assert invalid_tbar_position_response["d"]["requestStatus"]["code"] == 506


def test_obsws_preview_scene_requests(binary_path: Path):
    """obsws が Preview Scene 関連 request に応答することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        get_preview_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetCurrentPreviewScene",
                request_id="req-get-current-preview-scene-initial",
            )
        )
        assert get_preview_response["d"]["requestStatus"]["result"] is False
        assert get_preview_response["d"]["requestStatus"]["code"] == 506

        set_preview_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetCurrentPreviewScene",
                request_id="req-set-current-preview-scene",
                request_data={"sceneName": "Scene"},
            )
        )
        assert set_preview_response["d"]["requestStatus"]["result"] is False
        assert set_preview_response["d"]["requestStatus"]["code"] == 506

        get_scene_list_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetSceneList",
                request_id="req-get-scene-list-after-preview-set",
            )
        )
        assert get_scene_list_response["d"]["requestStatus"]["result"] is True
        assert get_scene_list_response["d"]["responseData"]["currentProgramSceneName"] == "Scene"
        assert get_scene_list_response["d"]["responseData"]["currentPreviewSceneName"] is None
        assert get_scene_list_response["d"]["responseData"]["currentPreviewSceneUuid"] is None


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
        assert "inputSettings" in renamed_get_response["d"]["responseData"]
        # OBS 仕様では GetInputSettings の responseData に inputName は含まれない
        assert "inputName" not in renamed_get_response["d"]["responseData"]


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
        # OBS 仕様では GetInputDefaultSettings の responseData に inputKind は含まれない
        assert "inputKind" not in response_data
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


def test_obsws_get_input_properties_list_property_items_request(binary_path: Path):
    """obsws が GetInputPropertiesListPropertyItems request に応答することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        use_env=False,
    ):
        # テスト用 input を作成する
        create_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="CreateInput",
                request_id="req-create-input-for-props-list",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "obsws-props-list-input",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                    "sceneItemEnabled": True,
                },
            )
        )
        assert create_response["d"]["requestStatus"]["result"] is True

        # 正常系: 空の propertyItems を返す
        response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputPropertiesListPropertyItems",
                request_id="req-get-props-list",
                request_data={
                    "inputName": "obsws-props-list-input",
                    "propertyName": "device_id",
                },
            )
        )
        status = response["d"]["requestStatus"]
        assert status["result"] is True
        assert status["code"] == 100
        response_data = response["d"]["responseData"]
        assert response_data["propertyItems"] == []

        # 存在しない input でエラー
        not_found_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputPropertiesListPropertyItems",
                request_id="req-get-props-list-not-found",
                request_data={
                    "inputName": "nonexistent",
                    "propertyName": "device_id",
                },
            )
        )
        not_found_status = not_found_response["d"]["requestStatus"]
        assert not_found_status["result"] is False
        assert not_found_status["code"] == 601

        # propertyName 欠落でエラー
        missing_prop_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputPropertiesListPropertyItems",
                request_id="req-get-props-list-no-prop",
                request_data={"inputName": "obsws-props-list-input"},
            )
        )
        missing_prop_status = missing_prop_response["d"]["requestStatus"]
        assert missing_prop_status["result"] is False
        assert missing_prop_status["code"] == 300

        # requestData 空でエラー
        empty_data_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputPropertiesListPropertyItems",
                request_id="req-get-props-list-empty",
                request_data={},
            )
        )
        empty_data_status = empty_data_response["d"]["requestStatus"]
        assert empty_data_status["result"] is False


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
        assert status["code"] == 301


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
        scene_item_id = create_response["d"]["responseData"]["sceneItemId"]
        assert isinstance(scene_item_id, int)
        assert scene_item_id > 0

        list_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputList",
                request_id="req-get-input-list-after-create",
            )
        )
        list_status = list_response["d"]["requestStatus"]
        assert list_status["result"] is True
        inputs = list_response["d"]["responseData"]["inputs"]
        names = [v["inputName"] for v in inputs]
        assert "obsws-test-input" in names
        # 各入力エントリに inputKindCaps が含まれることを確認する
        for inp in inputs:
            assert "inputKindCaps" in inp

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
        assert (
            settings_response["d"]["responseData"]["inputSettings"]["device_id"]
            == "sample-device"
        )
        # OBS 仕様では GetInputSettings の responseData に inputName は含まれない
        assert "inputName" not in settings_response["d"]["responseData"]
        assert (
            settings_response["d"]["responseData"]["inputKind"]
            == "video_capture_device"
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

            # OBS 互換: 有効な入力がなくても StartRecord は成功する（黒画面+無音）
            start_record_disabled_response = await _send_obsws_request(
                ws,
                request_type="StartRecord",
                request_id="req-start-record-disabled-input",
            )
            assert start_record_disabled_response["d"]["requestStatus"]["result"] is True

            stop_record_disabled_response = await _send_obsws_request(
                ws,
                request_type="StopRecord",
                request_id="req-stop-record-disabled-input",
            )
            assert stop_record_disabled_response["d"]["requestStatus"]["result"] is True

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

            # insert(0) で追加されるため、second が index=0、first が index=1、create_input が index=2
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
                == 0
            )

            # second を末尾（index=2）に移動する
            set_scene_item_index_response = await _send_obsws_request(
                ws,
                request_type="SetSceneItemIndex",
                request_id="req-set-scene-item-index",
                request_data={
                    "sceneName": "Scene",
                    "sceneItemId": second_scene_item_id,
                    "sceneItemIndex": 2,
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
                == 2
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
                    "sceneName": "Scene",
                    "destinationSceneName": "Scene",
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


def test_obsws_input_mute_and_volume_requests(binary_path: Path):
    """obsws の入力ミュート・音量制御 API が正しく動作することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False):
        # まず入力を作成する
        create_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="CreateInput",
                request_id="req-create",
                request_data={
                    "sceneName": "Scene",
                    "inputName": "mute-vol-test",
                    "inputKind": "video_capture_device",
                    "inputSettings": {},
                    "sceneItemEnabled": True,
                },
            )
        )
        assert create_response["d"]["requestStatus"]["result"] is True

        # --- GetInputMute: 初期状態は false ---
        mute_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputMute",
                request_id="req-get-mute",
                request_data={"inputName": "mute-vol-test"},
            )
        )
        assert mute_response["d"]["requestStatus"]["result"] is True
        assert mute_response["d"]["responseData"]["inputMuted"] is False

        # --- SetInputMute: ミュート有効化 ---
        set_mute_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetInputMute",
                request_id="req-set-mute",
                request_data={"inputName": "mute-vol-test", "inputMuted": True},
            )
        )
        assert set_mute_response["d"]["requestStatus"]["result"] is True

        # --- GetInputMute: ミュート有効確認 ---
        mute_response2 = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputMute",
                request_id="req-get-mute-2",
                request_data={"inputName": "mute-vol-test"},
            )
        )
        assert mute_response2["d"]["responseData"]["inputMuted"] is True

        # --- ToggleInputMute: トグルで false に戻る ---
        toggle_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="ToggleInputMute",
                request_id="req-toggle-mute",
                request_data={"inputName": "mute-vol-test"},
            )
        )
        assert toggle_response["d"]["requestStatus"]["result"] is True
        assert toggle_response["d"]["responseData"]["inputMuted"] is False

        # --- GetInputVolume: 初期状態は 0dB / 1.0 ---
        vol_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputVolume",
                request_id="req-get-vol",
                request_data={"inputName": "mute-vol-test"},
            )
        )
        assert vol_response["d"]["requestStatus"]["result"] is True
        vol_data = vol_response["d"]["responseData"]
        assert vol_data["inputVolumeMul"] == 1.0
        assert abs(vol_data["inputVolumeDb"] - 0.0) < 0.01

        # --- SetInputVolume: mul で設定 ---
        set_vol_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="SetInputVolume",
                request_id="req-set-vol",
                request_data={"inputName": "mute-vol-test", "inputVolumeMul": 0.5},
            )
        )
        assert set_vol_response["d"]["requestStatus"]["result"] is True

        # --- GetInputVolume: 0.5 mul ≈ -6.02 dB ---
        vol_response2 = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputVolume",
                request_id="req-get-vol-2",
                request_data={"inputName": "mute-vol-test"},
            )
        )
        vol_data2 = vol_response2["d"]["responseData"]
        assert vol_data2["inputVolumeMul"] == 0.5
        assert abs(vol_data2["inputVolumeDb"] - (-6.0206)) < 0.01

        # GetInputList の各入力に inputKindCaps が含まれることを確認する
        # （OBS 互換: GetInputList には inputMuted / inputVolumeMul は含めない）
        list_response = asyncio.run(
            _connect_identify_and_request(
                f"ws://{host}:{port}/",
                request_type="GetInputList",
                request_id="req-get-list",
            )
        )
        inputs = list_response["d"]["responseData"]["inputs"]
        test_input = next(i for i in inputs if i["inputName"] == "mute-vol-test")
        assert "inputKindCaps" in test_input
        assert "inputMuted" not in test_input
        assert "inputVolumeMul" not in test_input
