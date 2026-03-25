use super::*;
use crate::obsws::input_registry::{
    ObswsInputRegistry, ObswsInputSettings, ObswsSceneItemTransform,
};
use crate::obsws::protocol::{
    OBSWS_EVENT_SUB_GENERAL, OBSWS_EVENT_SUB_OUTPUTS, OBSWS_EVENT_SUB_SCENE_ITEMS, OBSWS_OP_EVENT,
    REQUEST_STATUS_RESOURCE_ALREADY_EXISTS, REQUEST_STATUS_SUCCESS,
};

#[test]
fn build_stream_state_changed_event_contains_expected_fields() {
    let event = build_stream_state_changed_event(true, "OBS_WEBSOCKET_OUTPUT_STARTED");
    let json = nojson::RawJson::parse(event.text()).expect("event must be valid json");
    let op: i64 = json
        .value()
        .to_member("op")
        .and_then(|v| v.required()?.try_into())
        .expect("op must be i64");
    let event_type: String = json
        .value()
        .to_path_member(&["d", "eventType"])
        .and_then(|v| v.required()?.try_into())
        .expect("eventType must be string");
    let event_intent: u32 = json
        .value()
        .to_path_member(&["d", "eventIntent"])
        .and_then(|v| v.required()?.try_into())
        .expect("eventIntent must be u32");
    let output_active: bool = json
        .value()
        .to_path_member(&["d", "eventData", "outputActive"])
        .and_then(|v| v.required()?.try_into())
        .expect("outputActive must be bool");
    let output_state: String = json
        .value()
        .to_path_member(&["d", "eventData", "outputState"])
        .and_then(|v| v.required()?.try_into())
        .expect("outputState must be string");
    assert_eq!(op, OBSWS_OP_EVENT);
    assert_eq!(event_type, "StreamStateChanged");
    assert_eq!(event_intent, OBSWS_EVENT_SUB_OUTPUTS);
    assert!(output_active);
    assert_eq!(output_state, "OBS_WEBSOCKET_OUTPUT_STARTED");
}

#[test]
fn build_stop_record_response_includes_output_path() {
    let response = build_stop_record_response("req-stop-record", "/tmp/output.mp4");
    let json = nojson::RawJson::parse(response.text()).expect("response must be valid json");
    let output_path: String = json
        .value()
        .to_path_member(&["d", "responseData", "outputPath"])
        .and_then(|v| v.required()?.try_into())
        .expect("outputPath must be string");
    assert_eq!(output_path, "/tmp/output.mp4");
}

#[test]
fn build_record_state_changed_event_includes_output_path_when_present() {
    let event = build_record_state_changed_event(
        false,
        "OBS_WEBSOCKET_OUTPUT_STOPPED",
        Some("/tmp/record.mp4"),
    );
    let json = nojson::RawJson::parse(event.text()).expect("event must be valid json");
    let event_type: String = json
        .value()
        .to_path_member(&["d", "eventType"])
        .and_then(|v| v.required()?.try_into())
        .expect("eventType must be string");
    let output_state: String = json
        .value()
        .to_path_member(&["d", "eventData", "outputState"])
        .and_then(|v| v.required()?.try_into())
        .expect("outputState must be string");
    let output_path: String = json
        .value()
        .to_path_member(&["d", "eventData", "outputPath"])
        .and_then(|v| v.required()?.try_into())
        .expect("outputPath must be string");
    assert_eq!(event_type, "RecordStateChanged");
    assert_eq!(output_state, "OBS_WEBSOCKET_OUTPUT_STOPPED");
    assert_eq!(output_path, "/tmp/record.mp4");
}

#[test]
fn build_scene_events_contain_expected_fields() {
    let created_event = build_scene_created_event("Scene A", "scene-uuid-a");
    let removed_event = build_scene_removed_event("Scene B", "scene-uuid-b");

    for (event, expected_type, expected_name) in [
        (created_event, "SceneCreated", "Scene A"),
        (removed_event, "SceneRemoved", "Scene B"),
    ] {
        let json = nojson::RawJson::parse(event.text()).expect("event must be valid json");
        let event_type: String = json
            .value()
            .to_path_member(&["d", "eventType"])
            .and_then(|v| v.required()?.try_into())
            .expect("eventType must be string");
        let scene_name: String = json
            .value()
            .to_path_member(&["d", "eventData", "sceneName"])
            .and_then(|v| v.required()?.try_into())
            .expect("sceneName must be string");
        assert_eq!(event_type, expected_type);
        assert_eq!(scene_name, expected_name);
    }
}

#[test]
fn build_current_preview_scene_changed_event_contains_expected_fields() {
    let event = build_current_preview_scene_changed_event("Scene P", "scene-uuid-p");
    let json = nojson::RawJson::parse(event.text()).expect("event must be valid json");
    let event_type: String = json
        .value()
        .to_path_member(&["d", "eventType"])
        .and_then(|v| v.required()?.try_into())
        .expect("eventType must be string");
    let scene_name: String = json
        .value()
        .to_path_member(&["d", "eventData", "sceneName"])
        .and_then(|v| v.required()?.try_into())
        .expect("sceneName must be string");
    assert_eq!(event_type, "CurrentPreviewSceneChanged");
    assert_eq!(scene_name, "Scene P");
}

#[test]
fn build_custom_event_contains_expected_fields() {
    let event = build_custom_event(
        &nojson::RawJsonOwned::parse(r#"{"message":"hello"}"#).expect("eventData must be valid"),
    );
    let json = nojson::RawJson::parse(event.text()).expect("event must be valid json");
    let event_type: String = json
        .value()
        .to_path_member(&["d", "eventType"])
        .and_then(|v| v.required()?.try_into())
        .expect("eventType must be string");
    let event_intent: u32 = json
        .value()
        .to_path_member(&["d", "eventIntent"])
        .and_then(|v| v.required()?.try_into())
        .expect("eventIntent must be u32");
    let message: String = json
        .value()
        .to_path_member(&["d", "eventData", "message"])
        .and_then(|v| v.required()?.try_into())
        .expect("message must be string");
    assert_eq!(event_type, "CustomEvent");
    assert_eq!(event_intent, OBSWS_EVENT_SUB_GENERAL);
    assert_eq!(message, "hello");
}

#[test]
fn build_get_and_set_current_preview_scene_response_succeeds() {
    let set_response = build_set_current_preview_scene_response("req-set-preview-scene");
    let set_json =
        nojson::RawJson::parse(set_response.text()).expect("response must be valid json");
    let set_result: bool = set_json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    assert!(!set_result);

    let get_response = build_get_current_preview_scene_response("req-get-preview-scene");
    let get_json =
        nojson::RawJson::parse(get_response.text()).expect("response must be valid json");
    let get_result: bool = get_json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    assert!(!get_result);
}

#[test]
fn build_set_current_scene_transition_settings_rejects_fixed_transition() {
    let mut registry = ObswsInputRegistry::new_for_test();
    // cut_transition に切り替えてからカスタム設定を試みる
    registry
        .set_current_scene_transition("cut_transition")
        .expect("set transition must succeed");
    let set_transition_settings_request_data =
        nojson::RawJsonOwned::parse(r#"{"transitionSettings":{"curve":"ease","power":2}}"#)
            .expect("requestData must be valid json");
    let set_transition_settings_response = build_set_current_scene_transition_settings_response(
        "req-set-transition-settings",
        Some(&set_transition_settings_request_data),
        &mut registry,
    );
    let set_transition_settings_json =
        nojson::RawJson::parse(set_transition_settings_response.text())
            .expect("response must be valid json");
    let set_transition_settings_result: bool = set_transition_settings_json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    // cut_transition は固定トランジションなので 606 を返す
    assert!(!set_transition_settings_result);
    let set_transition_settings_code: i64 = set_transition_settings_json
        .value()
        .to_path_member(&["d", "requestStatus", "code"])
        .and_then(|v| v.required()?.try_into())
        .expect("code must be i64");
    assert_eq!(set_transition_settings_code, 606);
}

#[test]
fn build_set_current_scene_transition_settings_rejects_fade_transition() {
    let mut registry = ObswsInputRegistry::new_for_test();
    // fade_transition はビルトイントランジションなのでカスタム設定をサポートしない
    let set_transition_settings_request_data =
        nojson::RawJsonOwned::parse(r#"{"transitionSettings":{"curve":"ease","power":2}}"#)
            .expect("requestData must be valid json");
    let set_transition_settings_response = build_set_current_scene_transition_settings_response(
        "req-set-transition-settings",
        Some(&set_transition_settings_request_data),
        &mut registry,
    );
    let set_transition_settings_json =
        nojson::RawJson::parse(set_transition_settings_response.text())
            .expect("response must be valid json");
    let set_transition_settings_result: bool = set_transition_settings_json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    assert!(!set_transition_settings_result);
}

#[test]
fn build_set_tbar_position_returns_506() {
    let registry = ObswsInputRegistry::new_for_test();
    // SetTBarPosition は Studio Mode 無効のため 506 を返す
    let set_tbar_position_response = build_set_tbar_position_response("req-set-tbar-position");
    let set_tbar_position_json = nojson::RawJson::parse(set_tbar_position_response.text())
        .expect("response must be valid json");
    let set_tbar_position_result: bool = set_tbar_position_json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    assert!(!set_tbar_position_result);
    let set_tbar_position_code: i64 = set_tbar_position_json
        .value()
        .to_path_member(&["d", "requestStatus", "code"])
        .and_then(|v| v.required()?.try_into())
        .expect("code must be i64");
    assert_eq!(set_tbar_position_code, 506);

    let get_transition_cursor_response =
        build_get_current_scene_transition_cursor_response("req-get-transition-cursor", &registry);
    let get_transition_cursor_json = nojson::RawJson::parse(get_transition_cursor_response.text())
        .expect("response must be valid json");
    let transition_cursor: f64 = get_transition_cursor_json
        .value()
        .to_path_member(&["d", "responseData", "transitionCursor"])
        .and_then(|v| v.required()?.try_into())
        .expect("transitionCursor must be f64");
    // tbar_position は変更されていないのでデフォルト値 0.0
    assert_eq!(transition_cursor, 0.0);
}

#[test]
fn build_input_events_contain_expected_fields() {
    let input_settings = ObswsInputSettings::default_for_kind("image_source")
        .expect("default settings must be available");
    let default_input_settings = ObswsInputSettings::default_for_kind("image_source")
        .expect("default settings must be available");
    let created_event = build_input_created_event(
        "camera-1",
        "input-uuid-1",
        "image_source",
        &input_settings,
        &default_input_settings,
    );
    let removed_event = build_input_removed_event("camera-2", "input-uuid-2");

    for (event, expected_type, expected_name, expected_uuid) in [
        (created_event, "InputCreated", "camera-1", "input-uuid-1"),
        (removed_event, "InputRemoved", "camera-2", "input-uuid-2"),
    ] {
        let json = nojson::RawJson::parse(event.text()).expect("event must be valid json");
        let event_type: String = json
            .value()
            .to_path_member(&["d", "eventType"])
            .and_then(|v| v.required()?.try_into())
            .expect("eventType must be string");
        let event_data = json
            .value()
            .to_path_member(&["d", "eventData"])
            .expect("eventData access must succeed")
            .required()
            .expect("eventData must exist");
        let input_name: String = event_data
            .to_member("inputName")
            .and_then(|v| v.required()?.try_into())
            .expect("inputName must be string");
        let input_uuid: String = event_data
            .to_member("inputUuid")
            .and_then(|v| v.required()?.try_into())
            .expect("inputUuid must be string");
        assert_eq!(event_type, expected_type);
        assert_eq!(input_name, expected_name);
        assert_eq!(input_uuid, expected_uuid);
    }
}

#[test]
fn build_input_settings_changed_event_contains_expected_fields() {
    let input_settings = ObswsInputSettings::VideoCaptureDevice(
        crate::obsws::input_registry::ObswsVideoCaptureDeviceSettings {
            device_id: Some("camera-1".to_owned()),
        },
    );
    let event =
        build_input_settings_changed_event("camera-source", "input-uuid-3", &input_settings);
    let json = nojson::RawJson::parse(event.text()).expect("event must be valid json");
    let event_type: String = json
        .value()
        .to_path_member(&["d", "eventType"])
        .and_then(|v| v.required()?.try_into())
        .expect("eventType must be string");
    let input_name: String = json
        .value()
        .to_path_member(&["d", "eventData", "inputName"])
        .and_then(|v| v.required()?.try_into())
        .expect("inputName must be string");
    let device_id: String = json
        .value()
        .to_path_member(&["d", "eventData", "inputSettings", "device_id"])
        .and_then(|v| v.required()?.try_into())
        .expect("device_id must be string");
    assert_eq!(event_type, "InputSettingsChanged");
    assert_eq!(input_name, "camera-source");
    assert_eq!(device_id, "camera-1");
}

#[test]
fn build_input_name_changed_event_contains_expected_fields() {
    let event = build_input_name_changed_event("camera-renamed", "camera-before", "input-uuid-4");
    let json = nojson::RawJson::parse(event.text()).expect("event must be valid json");
    let event_type: String = json
        .value()
        .to_path_member(&["d", "eventType"])
        .and_then(|v| v.required()?.try_into())
        .expect("eventType must be string");
    let input_name: String = json
        .value()
        .to_path_member(&["d", "eventData", "inputName"])
        .and_then(|v| v.required()?.try_into())
        .expect("inputName must be string");
    let old_input_name: String = json
        .value()
        .to_path_member(&["d", "eventData", "oldInputName"])
        .and_then(|v| v.required()?.try_into())
        .expect("oldInputName must be string");
    let input_uuid: String = json
        .value()
        .to_path_member(&["d", "eventData", "inputUuid"])
        .and_then(|v| v.required()?.try_into())
        .expect("inputUuid must be string");
    assert_eq!(event_type, "InputNameChanged");
    assert_eq!(input_name, "camera-renamed");
    assert_eq!(old_input_name, "camera-before");
    assert_eq!(input_uuid, "input-uuid-4");
}

#[test]
fn build_scene_item_enable_state_changed_event_contains_expected_fields() {
    let event = build_scene_item_enable_state_changed_event(
        "Scene",
        "10000000-0000-0000-0000-000000000000",
        10,
        false,
    );
    let json = nojson::RawJson::parse(event.text()).expect("event must be valid json");
    let event_type: String = json
        .value()
        .to_path_member(&["d", "eventType"])
        .and_then(|v| v.required()?.try_into())
        .expect("eventType must be string");
    let event_intent: u32 = json
        .value()
        .to_path_member(&["d", "eventIntent"])
        .and_then(|v| v.required()?.try_into())
        .expect("eventIntent must be u32");
    let event_data = json
        .value()
        .to_path_member(&["d", "eventData"])
        .expect("eventData access must succeed")
        .required()
        .expect("eventData must exist");
    let scene_name: String = event_data
        .to_member("sceneName")
        .and_then(|v| v.required()?.try_into())
        .expect("sceneName must be string");
    let scene_uuid: String = event_data
        .to_member("sceneUuid")
        .and_then(|v| v.required()?.try_into())
        .expect("sceneUuid must be string");
    let scene_item_id: i64 = event_data
        .to_member("sceneItemId")
        .and_then(|v| v.required()?.try_into())
        .expect("sceneItemId must be i64");
    let scene_item_enabled: bool = event_data
        .to_member("sceneItemEnabled")
        .and_then(|v| v.required()?.try_into())
        .expect("sceneItemEnabled must be bool");
    assert_eq!(event_type, "SceneItemEnableStateChanged");
    assert_eq!(event_intent, OBSWS_EVENT_SUB_SCENE_ITEMS);
    assert_eq!(scene_name, "Scene");
    assert_eq!(scene_uuid, "10000000-0000-0000-0000-000000000000");
    assert_eq!(scene_item_id, 10);
    assert!(!scene_item_enabled);
}

#[test]
fn build_scene_item_lock_state_changed_event_contains_expected_fields() {
    let event = build_scene_item_lock_state_changed_event(
        "Scene",
        "10000000-0000-0000-0000-000000000000",
        10,
        true,
    );
    let json = nojson::RawJson::parse(event.text()).expect("event must be valid json");
    let event_type: String = json
        .value()
        .to_path_member(&["d", "eventType"])
        .and_then(|v| v.required()?.try_into())
        .expect("eventType must be string");
    let scene_item_locked: bool = json
        .value()
        .to_path_member(&["d", "eventData", "sceneItemLocked"])
        .and_then(|v| v.required()?.try_into())
        .expect("sceneItemLocked must be bool");
    assert_eq!(event_type, "SceneItemLockStateChanged");
    assert!(scene_item_locked);
}

#[test]
fn build_scene_item_transform_changed_event_contains_expected_fields() {
    let event = build_scene_item_transform_changed_event(
        "Scene",
        "10000000-0000-0000-0000-000000000000",
        10,
        &ObswsSceneItemTransform {
            position_x: 12.0,
            position_y: 34.0,
            ..Default::default()
        },
    );
    let json = nojson::RawJson::parse(event.text()).expect("event must be valid json");
    let event_type: String = json
        .value()
        .to_path_member(&["d", "eventType"])
        .and_then(|v| v.required()?.try_into())
        .expect("eventType must be string");
    let position_x: f64 = json
        .value()
        .to_path_member(&["d", "eventData", "sceneItemTransform", "positionX"])
        .and_then(|v| v.required()?.try_into())
        .expect("positionX must be f64");
    assert_eq!(event_type, "SceneItemTransformChanged");
    assert_eq!(position_x, 12.0);
}

#[test]
fn build_get_scene_item_id_response_succeeds_when_scene_item_exists() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input = ObswsInput::from_kind_and_settings(
        "image_source",
        nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
            .expect("settings must be valid json")
            .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "input-1", input, true)
        .expect("input creation must succeed");
    let request_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","sourceName":"input-1","searchOffset":0}"#,
    )
    .expect("request data must be valid json");

    let response =
        build_get_scene_item_id_response("req-get-scene-item-id", Some(&request_data), &registry);
    let json = nojson::RawJson::parse(response.text()).expect("response must be valid json");
    let result: bool = json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    let scene_item_id: i64 = json
        .value()
        .to_path_member(&["d", "responseData", "sceneItemId"])
        .and_then(|v| v.required()?.try_into())
        .expect("sceneItemId must be i64");
    assert!(result);
    assert_eq!(scene_item_id, 1);
}

#[test]
fn build_get_scene_item_id_response_succeeds_with_scene_uuid_and_source_uuid() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input = ObswsInput::from_kind_and_settings(
        "image_source",
        nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
            .expect("settings must be valid json")
            .value(),
    )
    .expect("input settings must be valid");
    let (created_input, _scene_item_id) = registry
        .create_input("Scene", "input-1", input, true)
        .expect("input creation must succeed");
    let scene_uuid = registry
        .get_scene_uuid("Scene")
        .expect("scene uuid must exist");
    // sceneUuid と sourceUuid のみで指定する
    let request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneUuid":"{}","sourceUuid":"{}","searchOffset":0}}"#,
        scene_uuid, created_input.input_uuid
    ))
    .expect("request data must be valid json");

    let response = build_get_scene_item_id_response(
        "req-get-scene-item-id-uuid",
        Some(&request_data),
        &registry,
    );
    let json = nojson::RawJson::parse(response.text()).expect("response must be valid json");
    let result: bool = json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    let scene_item_id: i64 = json
        .value()
        .to_path_member(&["d", "responseData", "sceneItemId"])
        .and_then(|v| v.required()?.try_into())
        .expect("sceneItemId must be i64");
    assert!(result);
    assert_eq!(scene_item_id, 1);
}

#[test]
fn build_set_current_program_scene_response_succeeds_with_scene_uuid() {
    let mut registry = ObswsInputRegistry::new_for_test();
    registry
        .create_scene("Scene B")
        .expect("scene creation must succeed");
    let scene_uuid = registry
        .get_scene_uuid("Scene B")
        .expect("scene uuid must exist");
    let request_data = nojson::RawJsonOwned::parse(format!(r#"{{"sceneUuid":"{}"}}"#, scene_uuid))
        .expect("request data must be valid json");

    let response = build_set_current_program_scene_response(
        "req-set-current-program-scene-uuid",
        Some(&request_data),
        &mut registry,
    );
    let json = nojson::RawJson::parse(response.text()).expect("response must be valid json");
    let result: bool = json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    assert!(result);
    let current = registry
        .current_program_scene()
        .expect("current program scene must exist");
    assert_eq!(current.scene_name, "Scene B");
}

#[test]
fn build_remove_scene_response_succeeds_with_scene_uuid() {
    let mut registry = ObswsInputRegistry::new_for_test();
    registry
        .create_scene("Scene B")
        .expect("scene creation must succeed");
    let scene_uuid = registry
        .get_scene_uuid("Scene B")
        .expect("scene uuid must exist");
    let request_data = nojson::RawJsonOwned::parse(format!(r#"{{"sceneUuid":"{}"}}"#, scene_uuid))
        .expect("request data must be valid json");

    let response =
        build_remove_scene_response("req-remove-scene-uuid", Some(&request_data), &mut registry);
    let json = nojson::RawJson::parse(response.text()).expect("response must be valid json");
    let result: bool = json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    assert!(result);
}

#[test]
fn build_set_scene_name_response_succeeds_with_scene_uuid() {
    let mut registry = ObswsInputRegistry::new_for_test();
    registry
        .create_scene("Scene B")
        .expect("scene creation must succeed");
    let scene_uuid = registry
        .get_scene_uuid("Scene B")
        .expect("scene uuid must exist");
    let request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneUuid":"{}","newSceneName":"Scene C"}}"#,
        scene_uuid
    ))
    .expect("request data must be valid json");

    let response = build_set_scene_name_response(
        "req-set-scene-name-uuid",
        Some(&request_data),
        &mut registry,
    );
    let json = nojson::RawJson::parse(response.text()).expect("response must be valid json");
    let result: bool = json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    assert!(result);
    // OBS は SetSceneName で responseData を返さないため、responseData が存在しないことを確認
    let response_data = json
        .value()
        .to_path_member(&["d", "responseData"])
        .expect("responseData path must be parseable")
        .optional();
    assert!(response_data.is_none());
}

#[test]
fn build_set_scene_item_enabled_response_succeeds_with_scene_uuid() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input = ObswsInput::from_kind_and_settings(
        "image_source",
        nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
            .expect("settings must be valid json")
            .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "input-1", input, true)
        .expect("input creation must succeed");
    let scene_item_id = registry
        .get_scene_item_id("Scene", Some("input-1"), None, 0)
        .expect("scene item id must exist");
    let scene_uuid = registry
        .get_scene_uuid("Scene")
        .expect("scene uuid must exist");
    let request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneUuid":"{}","sceneItemId":{},"sceneItemEnabled":false}}"#,
        scene_uuid, scene_item_id
    ))
    .expect("request data must be valid json");

    let response = build_set_scene_item_enabled_response(
        "req-set-scene-item-enabled-uuid",
        Some(&request_data),
        &mut registry,
    );
    let json = nojson::RawJson::parse(response.text()).expect("response must be valid json");
    let result: bool = json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    assert!(result);
}

#[test]
fn build_get_scene_item_enabled_response_succeeds_with_scene_uuid() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input = ObswsInput::from_kind_and_settings(
        "image_source",
        nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
            .expect("settings must be valid json")
            .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "input-1", input, true)
        .expect("input creation must succeed");
    let scene_item_id = registry
        .get_scene_item_id("Scene", Some("input-1"), None, 0)
        .expect("scene item id must exist");
    let scene_uuid = registry
        .get_scene_uuid("Scene")
        .expect("scene uuid must exist");
    let request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneUuid":"{}","sceneItemId":{}}}"#,
        scene_uuid, scene_item_id
    ))
    .expect("request data must be valid json");

    let response = build_get_scene_item_enabled_response(
        "req-get-scene-item-enabled-uuid",
        Some(&request_data),
        &registry,
    );
    let json = nojson::RawJson::parse(response.text()).expect("response must be valid json");
    let result: bool = json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    let scene_item_enabled: bool = json
        .value()
        .to_path_member(&["d", "responseData", "sceneItemEnabled"])
        .and_then(|v| v.required()?.try_into())
        .expect("sceneItemEnabled must be bool");
    assert!(result);
    assert!(scene_item_enabled);
}

#[test]
fn build_set_scene_item_enabled_response_succeeds_when_scene_item_exists() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input = ObswsInput::from_kind_and_settings(
        "image_source",
        nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
            .expect("settings must be valid json")
            .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "input-1", input, true)
        .expect("input creation must succeed");
    let scene_item_id = registry
        .get_scene_item_id("Scene", Some("input-1"), None, 0)
        .expect("scene item id must exist");
    let request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemEnabled":false}}"#,
        scene_item_id
    ))
    .expect("request data must be valid json");

    let response = build_set_scene_item_enabled_response(
        "req-set-scene-item-enabled",
        Some(&request_data),
        &mut registry,
    );
    let json = nojson::RawJson::parse(response.text()).expect("response must be valid json");
    let result: bool = json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    assert!(result);
    assert!(registry.list_current_program_scene_inputs().is_empty());
}

#[test]
fn build_get_scene_item_enabled_response_succeeds_when_scene_item_exists() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input = ObswsInput::from_kind_and_settings(
        "image_source",
        nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
            .expect("settings must be valid json")
            .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "input-1", input, true)
        .expect("input creation must succeed");
    let scene_item_id = registry
        .get_scene_item_id("Scene", Some("input-1"), None, 0)
        .expect("scene item id must exist");
    registry
        .set_scene_item_enabled("Scene", scene_item_id, false)
        .expect("set scene item enabled must succeed");
    let request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{}}}"#,
        scene_item_id
    ))
    .expect("request data must be valid json");

    let response = build_get_scene_item_enabled_response(
        "req-get-scene-item-enabled",
        Some(&request_data),
        &registry,
    );
    let json = nojson::RawJson::parse(response.text()).expect("response must be valid json");
    let result: bool = json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    let scene_item_enabled: bool = json
        .value()
        .to_path_member(&["d", "responseData", "sceneItemEnabled"])
        .and_then(|v| v.required()?.try_into())
        .expect("sceneItemEnabled must be bool");
    assert!(result);
    assert!(!scene_item_enabled);
}

#[test]
fn build_get_and_set_scene_item_locked_response_succeeds_when_scene_item_exists() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input = ObswsInput::from_kind_and_settings(
        "image_source",
        nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
            .expect("settings must be valid json")
            .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "input-1", input, true)
        .expect("input creation must succeed");
    let scene_item_id = registry
        .get_scene_item_id("Scene", Some("input-1"), None, 0)
        .expect("scene item id must exist");
    let set_request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemLocked":true}}"#,
        scene_item_id
    ))
    .expect("request data must be valid json");

    let set_response = execute_set_scene_item_locked(
        "req-set-scene-item-locked",
        Some(&set_request_data),
        &mut registry,
    )
    .response_text;
    let set_json =
        nojson::RawJson::parse(set_response.text()).expect("response must be valid json");
    let set_result: bool = set_json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    assert!(set_result);

    let get_request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{}}}"#,
        scene_item_id
    ))
    .expect("request data must be valid json");
    let get_response = build_get_scene_item_locked_response(
        "req-get-scene-item-locked",
        Some(&get_request_data),
        &registry,
    );
    let get_json =
        nojson::RawJson::parse(get_response.text()).expect("response must be valid json");
    let locked: bool = get_json
        .value()
        .to_path_member(&["d", "responseData", "sceneItemLocked"])
        .and_then(|v| v.required()?.try_into())
        .expect("sceneItemLocked must be bool");
    assert!(locked);
}

#[test]
fn build_get_and_set_scene_item_blend_mode_response_succeeds_when_scene_item_exists() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input = ObswsInput::from_kind_and_settings(
        "image_source",
        nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
            .expect("settings must be valid json")
            .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "input-1", input, true)
        .expect("input creation must succeed");
    let scene_item_id = registry
        .get_scene_item_id("Scene", Some("input-1"), None, 0)
        .expect("scene item id must exist");
    let set_request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemBlendMode":"OBS_BLEND_ADDITIVE"}}"#,
        scene_item_id
    ))
    .expect("request data must be valid json");

    let set_response = build_set_scene_item_blend_mode_response(
        "req-set-scene-item-blend-mode",
        Some(&set_request_data),
        &mut registry,
    );
    let set_json =
        nojson::RawJson::parse(set_response.text()).expect("response must be valid json");
    let set_result: bool = set_json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    assert!(set_result);

    let get_request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{}}}"#,
        scene_item_id
    ))
    .expect("request data must be valid json");
    let get_response = build_get_scene_item_blend_mode_response(
        "req-get-scene-item-blend-mode",
        Some(&get_request_data),
        &registry,
    );
    let get_json =
        nojson::RawJson::parse(get_response.text()).expect("response must be valid json");
    let blend_mode: String = get_json
        .value()
        .to_path_member(&["d", "responseData", "sceneItemBlendMode"])
        .and_then(|v| v.required()?.try_into())
        .expect("sceneItemBlendMode must be string");
    assert_eq!(blend_mode, "OBS_BLEND_ADDITIVE");
}

#[test]
fn build_get_and_set_scene_item_transform_response_succeeds_when_scene_item_exists() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input = ObswsInput::from_kind_and_settings(
        "image_source",
        nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
            .expect("settings must be valid json")
            .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "input-1", input, true)
        .expect("input creation must succeed");
    let scene_item_id = registry
        .get_scene_item_id("Scene", Some("input-1"), None, 0)
        .expect("scene item id must exist");
    let set_request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemTransform":{{"positionX":321.0}}}}"#,
        scene_item_id
    ))
    .expect("request data must be valid json");

    let set_response = execute_set_scene_item_transform(
        "req-set-scene-item-transform",
        Some(&set_request_data),
        &mut registry,
    )
    .response_text;
    let set_json =
        nojson::RawJson::parse(set_response.text()).expect("response must be valid json");
    let set_result: bool = set_json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    assert!(set_result);

    let get_request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{}}}"#,
        scene_item_id
    ))
    .expect("request data must be valid json");
    let get_response = build_get_scene_item_transform_response(
        "req-get-scene-item-transform",
        Some(&get_request_data),
        &registry,
    );
    let get_json =
        nojson::RawJson::parse(get_response.text()).expect("response must be valid json");
    let position_x: f64 = get_json
        .value()
        .to_path_member(&["d", "responseData", "sceneItemTransform", "positionX"])
        .and_then(|v| v.required()?.try_into())
        .expect("positionX must be f64");
    assert_eq!(position_x, 321.0);
}

#[test]
fn execute_set_scene_item_transform_rejects_invalid_alignment_value() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input = ObswsInput::from_kind_and_settings(
        "image_source",
        nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
            .expect("settings must be valid json")
            .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "input-1", input, true)
        .expect("input creation must succeed");
    let scene_item_id = registry
        .get_scene_item_id("Scene", Some("input-1"), None, 0)
        .expect("scene item id must exist");
    let request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemTransform":{{"alignment":3}}}}"#,
        scene_item_id
    ))
    .expect("request data must be valid json");

    let response = execute_set_scene_item_transform(
        "req-set-scene-item-transform-invalid-alignment",
        Some(&request_data),
        &mut registry,
    )
    .response_text;
    let json = nojson::RawJson::parse(response.text()).expect("response must be valid json");
    let result: bool = json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    let code: i64 = json
        .value()
        .to_path_member(&["d", "requestStatus", "code"])
        .and_then(|v| v.required()?.try_into())
        .expect("code must be i64");
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
}

#[test]
fn build_get_scene_item_list_response_succeeds_when_scene_exists() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input = ObswsInput::from_kind_and_settings(
        "image_source",
        nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
            .expect("settings must be valid json")
            .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "input-1", input, true)
        .expect("input creation must succeed");
    let request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene"}"#)
        .expect("request data must be valid json");

    let response = build_get_scene_item_list_response(
        "req-get-scene-item-list",
        Some(&request_data),
        &registry,
    );
    let json = nojson::RawJson::parse(response.text()).expect("response must be valid json");
    let result: bool = json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    let scene_items = json
        .value()
        .to_path_member(&["d", "responseData", "sceneItems"])
        .expect("sceneItems access must succeed")
        .required()
        .expect("sceneItems must exist")
        .to_array()
        .expect("sceneItems must be array");
    let scene_name = json
        .value()
        .to_path_member(&["d", "responseData", "sceneName"])
        .expect("sceneName access must succeed")
        .optional();
    assert!(result);
    assert!(scene_items.count() >= 1);
    assert!(scene_name.is_none());
}

#[test]
fn build_create_scene_item_response_succeeds_when_source_exists() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input = ObswsInput::from_kind_and_settings(
        "image_source",
        nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
            .expect("settings must be valid json")
            .value(),
    )
    .expect("input settings must be valid");
    let created = registry
        .create_input("Scene", "input-1", input, false)
        .expect("input creation must succeed");
    let request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sourceUuid":"{}","sceneItemEnabled":true}}"#,
        created.0.input_uuid
    ))
    .expect("request data must be valid json");

    let response =
        execute_create_scene_item("req-create-scene-item", Some(&request_data), &mut registry)
            .response_text;
    let json = nojson::RawJson::parse(response.text()).expect("response must be valid json");
    let result: bool = json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    let scene_item_id: i64 = json
        .value()
        .to_path_member(&["d", "responseData", "sceneItemId"])
        .and_then(|v| v.required()?.try_into())
        .expect("sceneItemId must be i64");
    assert!(result);
    assert!(scene_item_id > 0);
}

#[test]
fn build_set_scene_item_index_response_rejects_invalid_index() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input = ObswsInput::from_kind_and_settings(
        "image_source",
        nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
            .expect("settings must be valid json")
            .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "input-1", input, true)
        .expect("input creation must succeed");
    let scene_item_id = registry
        .get_scene_item_id("Scene", Some("input-1"), None, 0)
        .expect("scene item id must exist");
    let request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemIndex":100}}"#,
        scene_item_id
    ))
    .expect("request data must be valid json");

    let response = execute_set_scene_item_index(
        "req-set-scene-item-index",
        Some(&request_data),
        &mut registry,
    )
    .response_text;
    let json = nojson::RawJson::parse(response.text()).expect("response must be valid json");
    let result: bool = json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    let code: i64 = json
        .value()
        .to_path_member(&["d", "requestStatus", "code"])
        .and_then(|v| v.required()?.try_into())
        .expect("code must be i64");
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
}

#[test]
fn build_scene_item_created_event_contains_expected_fields() {
    let event =
        build_scene_item_created_event("Scene", "scene-uuid-1", 10, "camera-1", "input-uuid-1", 0);
    let json = nojson::RawJson::parse(event.text()).expect("event must be valid json");
    let event_type: String = json
        .value()
        .to_path_member(&["d", "eventType"])
        .and_then(|v| v.required()?.try_into())
        .expect("eventType must be string");
    let scene_item_id: i64 = json
        .value()
        .to_path_member(&["d", "eventData", "sceneItemId"])
        .and_then(|v| v.required()?.try_into())
        .expect("sceneItemId must be i64");
    assert_eq!(event_type, "SceneItemCreated");
    assert_eq!(scene_item_id, 10);
}

#[test]
fn build_remove_scene_response_succeeds_when_scene_exists() {
    let mut registry = ObswsInputRegistry::new_for_test();
    registry
        .create_scene("Scene B")
        .expect("scene creation must succeed");
    let request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
        .expect("requestData must be valid json");

    let response =
        build_remove_scene_response("req-remove-scene", Some(&request_data), &mut registry);
    let json = nojson::RawJson::parse(response.text()).expect("response must be valid json");
    let result: bool = json
        .value()
        .to_path_member(&["d", "requestStatus", "result"])
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    assert!(result);
}

#[test]
fn build_and_parse_request_batch_response_preserves_fields() {
    let response = build_request_batch_response(
        "batch-1",
        &[
            RequestBatchResult {
                request_id: "req-1".to_owned(),
                request_type: "GetVersion".to_owned(),
                request_status_result: true,
                request_status_code: REQUEST_STATUS_SUCCESS,
                request_status_comment: None,
                response_data: Some(
                    nojson::RawJsonOwned::parse(r#"{"rpcVersion":1}"#)
                        .expect("responseData must be valid json"),
                ),
            },
            RequestBatchResult {
                request_id: "req-2".to_owned(),
                request_type: "CreateScene".to_owned(),
                request_status_result: false,
                request_status_code: REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
                request_status_comment: Some("Scene already exists".to_owned()),
                response_data: None,
            },
        ],
    );
    let json = nojson::RawJson::parse(response.text()).expect("response must be valid json");
    let op: i64 = json
        .value()
        .to_member("op")
        .and_then(|v| v.required()?.try_into())
        .expect("op must be i64");
    assert_eq!(op, OBSWS_OP_REQUEST_BATCH_RESPONSE);

    let results = json
        .value()
        .to_path_member(&["d", "results"])
        .expect("results access must succeed")
        .required()
        .expect("results must exist");
    let mut results = results.to_array().expect("results must be array");
    let first = results.next().expect("first result must exist");
    let first_request_type: String = first
        .to_member("requestType")
        .and_then(|v| v.required()?.try_into())
        .expect("requestType must be string");
    assert_eq!(first_request_type, "GetVersion");

    let source_response = build_get_version_response("req-1");
    let parsed = parse_request_response_for_batch_result(&source_response)
        .expect("request response must be parsed");
    assert_eq!(parsed.request_type, "GetVersion");
    assert!(parsed.request_status_result);
    assert_eq!(parsed.request_status_code, REQUEST_STATUS_SUCCESS);
    assert!(parsed.response_data.is_some());
}
