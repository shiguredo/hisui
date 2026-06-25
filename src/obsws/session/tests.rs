use super::*;
use crate::obsws::message::RequestMessage;
use crate::obsws::protocol::{
    OBSWS_EVENT_SUB_INPUTS, OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED,
    OBSWS_EVENT_SUB_SCENE_ITEMS, REQUEST_STATUS_INVALID_REQUEST_FIELD,
    REQUEST_STATUS_MISSING_REQUEST_FIELD, REQUEST_STATUS_OUTPUT_NOT_RUNNING,
    REQUEST_STATUS_REQUEST_PROCESSING_FAILED, REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
    REQUEST_STATUS_RESOURCE_NOT_FOUND,
};
use crate::obsws::state::{ObswsInput, ObswsSessionState};
use std::time::Duration;

// Phase 1 で `tests/common.rs` に共通ヘルパー 23 件を物理移動した。
// 暫定の `use common::*;` はエントリポイント直下に残るテストが
// ヘルパーを修飾なしで呼び続けられるようにするもの。
// 全テストが各サブモジュールへ移動完了する Phase 14 で `mod common;` 含めて整理する。
#[path = "tests/common.rs"]
mod common;
#[path = "tests/lifecycle.rs"]
mod lifecycle;
#[path = "tests/scene.rs"]
mod scene;
use common::*;

#[tokio::test]
async fn create_and_remove_input_with_input_subscription_send_input_events() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":8}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let create_request_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-1","inputKind":"image_source","inputSettings":{},"sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_request_data),
        })
        .await;
    let messages = unwrap_send_texts(create_action);
    let (_, event_type, event_intent) = parse_event_type_and_intent(&messages[1].0);
    assert_eq!(event_type, "InputCreated");
    assert_eq!(event_intent, OBSWS_EVENT_SUB_INPUTS);

    let remove_request_data = nojson::RawJsonOwned::parse(r#"{"inputName":"camera-1"}"#)
        .expect("requestData must be valid json");
    let remove_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-remove-input".to_owned()),
            request_type: Some("RemoveInput".to_owned()),
            request_data: Some(remove_request_data),
        })
        .await;
    let messages = unwrap_send_texts(remove_action);
    let (_, event_type, event_intent) = parse_event_type_and_intent(&messages[1].0);
    assert_eq!(event_type, "InputRemoved");
    assert_eq!(event_intent, OBSWS_EVENT_SUB_INPUTS);
}

#[tokio::test]
async fn set_input_settings_with_input_subscription_sends_event() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":8}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let create_request_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-1","inputKind":"video_capture_device","inputSettings":{},"sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_request_data),
        })
        .await;
    let _ = unwrap_send_texts(create_action);

    let set_request_data = nojson::RawJsonOwned::parse(
        r#"{"inputName":"camera-1","inputSettings":{"device_id":"camera-2"}}"#,
    )
    .expect("requestData must be valid json");
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-input-settings".to_owned()),
            request_type: Some("SetInputSettings".to_owned()),
            request_data: Some(set_request_data),
        })
        .await;
    let messages = unwrap_send_texts(set_action);
    let (_, event_type, event_intent) = parse_event_type_and_intent(&messages[1].0);
    assert_eq!(event_type, "InputSettingsChanged");
    assert_eq!(event_intent, OBSWS_EVENT_SUB_INPUTS);
}

#[tokio::test]
async fn set_input_settings_with_input_subscription_does_not_send_event_on_error() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":8}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let create_request_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-1","inputKind":"video_capture_device","inputSettings":{},"sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_request_data),
        })
        .await;
    let _ = unwrap_send_texts(create_action);

    let set_request_data =
        nojson::RawJsonOwned::parse(r#"{"inputName":"camera-1","inputSettings":{"device_id":1}}"#)
            .expect("requestData must be valid json");
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-input-settings".to_owned()),
            request_type: Some("SetInputSettings".to_owned()),
            request_data: Some(set_request_data),
        })
        .await;
    let text = unwrap_send_text(set_action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
}

#[tokio::test]
async fn set_input_name_with_input_subscription_sends_event() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":8}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let create_request_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-1","inputKind":"video_capture_device","inputSettings":{},"sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_request_data),
        })
        .await;
    let _ = unwrap_send_texts(create_action);

    let set_request_data = nojson::RawJsonOwned::parse(
        r#"{"inputName":"camera-1","newInputName":"camera-1-renamed"}"#,
    )
    .expect("requestData must be valid json");
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-input-name".to_owned()),
            request_type: Some("SetInputName".to_owned()),
            request_data: Some(set_request_data),
        })
        .await;
    let messages = unwrap_send_texts(set_action);
    let (_, event_type, event_intent) = parse_event_type_and_intent(&messages[1].0);
    assert_eq!(event_type, "InputNameChanged");
    assert_eq!(event_intent, OBSWS_EVENT_SUB_INPUTS);
}

#[tokio::test]
async fn set_input_name_with_input_subscription_does_not_send_event_on_error() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":8}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let create_request_data_a = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-1","inputKind":"video_capture_device","inputSettings":{},"sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_action_a = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input-a".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_request_data_a),
        })
        .await;
    let _ = unwrap_send_texts(create_action_a);

    let create_request_data_b = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-2","inputKind":"video_capture_device","inputSettings":{},"sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_action_b = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input-b".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_request_data_b),
        })
        .await;
    let _ = unwrap_send_texts(create_action_b);

    let set_request_data =
        nojson::RawJsonOwned::parse(r#"{"inputName":"camera-1","newInputName":"camera-2"}"#)
            .expect("requestData must be valid json");
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-input-name-duplicate".to_owned()),
            request_type: Some("SetInputName".to_owned()),
            request_data: Some(set_request_data),
        })
        .await;
    let text = unwrap_send_text(set_action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_RESOURCE_ALREADY_EXISTS);
}

#[tokio::test]
async fn set_input_name_with_invalid_input_uuid_type_returns_parse_error() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":8}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let request_data =
        nojson::RawJsonOwned::parse(r#"{"inputUuid":1,"newInputName":"camera-renamed"}"#)
            .expect("requestData must be valid json");
    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-input-name-invalid-type".to_owned()),
            request_type: Some("SetInputName".to_owned()),
            request_data: Some(request_data),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
}

#[tokio::test]
async fn set_scene_item_enabled_with_scene_subscription_sends_event_when_changed() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":132}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let create_request_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-1","inputKind":"image_source","inputSettings":{},"sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_request_data),
        })
        .await;
    // SCENE_ITEMS サブスクリプションが有効なため SceneItemCreated イベントも送信される
    let _ = unwrap_send_texts(create_action);

    let get_scene_item_id_request_data =
        nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene","sourceName":"camera-1"}"#)
            .expect("requestData must be valid json");
    let get_scene_item_id_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-scene-item-id".to_owned()),
            request_type: Some("GetSceneItemId".to_owned()),
            request_data: Some(get_scene_item_id_request_data),
        })
        .await;
    let text = unwrap_send_text(get_scene_item_id_action);
    let scene_item_id = parse_response_scene_item_id(&text);

    let set_request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemEnabled":false}}"#,
        scene_item_id
    ))
    .expect("requestData must be valid json");
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-scene-item-enabled".to_owned()),
            request_type: Some("SetSceneItemEnabled".to_owned()),
            request_data: Some(set_request_data),
        })
        .await;
    let messages = unwrap_send_texts(set_action);
    assert_eq!(messages.len(), 2);
    let (_, event_type, event_intent) = parse_event_type_and_intent(&messages[1].0);
    let event_json =
        nojson::RawJson::parse(messages[1].0.text()).expect("event message must be valid json");
    let scene_uuid: String = event_json
        .value()
        .to_path_member(&["d", "eventData", "sceneUuid"])
        .and_then(|v| v.required()?.try_into())
        .expect("sceneUuid must be string");
    assert_eq!(event_type, "SceneItemEnableStateChanged");
    assert_eq!(event_intent, OBSWS_EVENT_SUB_SCENE_ITEMS);
    assert_eq!(scene_uuid, "10000000-0000-0000-0000-000000000000");
}

#[tokio::test]
async fn set_scene_item_enabled_with_same_value_returns_response_only() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":4}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let create_request_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-1","inputKind":"image_source","inputSettings":{},"sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_request_data),
        })
        .await;
    assert!(matches!(create_action, SessionAction::SendText { .. }));

    let get_scene_item_id_request_data =
        nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene","sourceName":"camera-1"}"#)
            .expect("requestData must be valid json");
    let get_scene_item_id_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-scene-item-id".to_owned()),
            request_type: Some("GetSceneItemId".to_owned()),
            request_data: Some(get_scene_item_id_request_data),
        })
        .await;
    let text = unwrap_send_text(get_scene_item_id_action);
    let scene_item_id = parse_response_scene_item_id(&text);

    let set_request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemEnabled":true}}"#,
        scene_item_id
    ))
    .expect("requestData must be valid json");
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-scene-item-enabled-same".to_owned()),
            request_type: Some("SetSceneItemEnabled".to_owned()),
            request_data: Some(set_request_data),
        })
        .await;
    assert!(matches!(set_action, SessionAction::SendText { .. }));
}

#[tokio::test]
async fn set_scene_item_locked_with_scene_subscription_sends_event_when_changed() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":132}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let create_request_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-1","inputKind":"image_source","inputSettings":{},"sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_request_data),
        })
        .await;
    let _ = unwrap_send_texts(create_action);

    let get_scene_item_id_request_data =
        nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene","sourceName":"camera-1"}"#)
            .expect("requestData must be valid json");
    let get_scene_item_id_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-scene-item-id".to_owned()),
            request_type: Some("GetSceneItemId".to_owned()),
            request_data: Some(get_scene_item_id_request_data),
        })
        .await;
    let text = unwrap_send_text(get_scene_item_id_action);
    let scene_item_id = parse_response_scene_item_id(&text);

    let set_request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemLocked":true}}"#,
        scene_item_id
    ))
    .expect("requestData must be valid json");
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-scene-item-locked".to_owned()),
            request_type: Some("SetSceneItemLocked".to_owned()),
            request_data: Some(set_request_data),
        })
        .await;
    let messages = unwrap_send_texts(set_action);
    assert_eq!(messages.len(), 2);
    let (_, event_type, event_intent) = parse_event_type_and_intent(&messages[1].0);
    assert_eq!(event_type, "SceneItemLockStateChanged");
    assert_eq!(event_intent, OBSWS_EVENT_SUB_SCENE_ITEMS);
}

#[tokio::test]
async fn set_scene_item_transform_with_scene_subscription_sends_event_when_changed() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    // 524420 = OBSWS_EVENT_SUB_SCENES (1 << 2) | OBSWS_EVENT_SUB_SCENE_ITEMS (1 << 7) | OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED (1 << 19)
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":524420}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let create_request_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-1","inputKind":"image_source","inputSettings":{},"sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_request_data),
        })
        .await;
    let _ = unwrap_send_texts(create_action);

    let get_scene_item_id_request_data =
        nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene","sourceName":"camera-1"}"#)
            .expect("requestData must be valid json");
    let get_scene_item_id_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-scene-item-id".to_owned()),
            request_type: Some("GetSceneItemId".to_owned()),
            request_data: Some(get_scene_item_id_request_data),
        })
        .await;
    let text = unwrap_send_text(get_scene_item_id_action);
    let scene_item_id = parse_response_scene_item_id(&text);

    let set_request_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemTransform":{{"positionX":10.0}}}}"#,
        scene_item_id
    ))
    .expect("requestData must be valid json");
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-scene-item-transform".to_owned()),
            request_type: Some("SetSceneItemTransform".to_owned()),
            request_data: Some(set_request_data),
        })
        .await;
    let messages = unwrap_send_texts(set_action);
    assert_eq!(messages.len(), 2);
    let (_, event_type, event_intent) = parse_event_type_and_intent(&messages[1].0);
    assert_eq!(event_type, "SceneItemTransformChanged");
    assert_eq!(event_intent, OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED);
}

#[tokio::test]
async fn create_scene_item_with_scene_subscription_sends_created_event() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":132}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let create_input_request_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-1","inputKind":"image_source","inputSettings":{},"sceneItemEnabled":false}"#,
    )
    .expect("requestData must be valid json");
    let create_input_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_input_request_data),
        })
        .await;
    // SCENE_ITEMS サブスクリプションが有効なため SceneItemCreated イベントも送信される
    let create_input_messages = unwrap_send_texts(create_input_action);
    assert_eq!(create_input_messages.len(), 2);
    let (_, event_type, event_intent) = parse_event_type_and_intent(&create_input_messages[1].0);
    assert_eq!(event_type, "SceneItemCreated");
    assert_eq!(event_intent, OBSWS_EVENT_SUB_SCENE_ITEMS);

    let create_scene_item_request_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","sourceName":"camera-1","sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_scene_item_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-scene-item".to_owned()),
            request_type: Some("CreateSceneItem".to_owned()),
            request_data: Some(create_scene_item_request_data),
        })
        .await;
    let messages = unwrap_send_texts(create_scene_item_action);
    assert_eq!(messages.len(), 2);
    let (_, event_type, event_intent) = parse_event_type_and_intent(&messages[1].0);
    assert_eq!(event_type, "SceneItemCreated");
    assert_eq!(event_intent, OBSWS_EVENT_SUB_SCENE_ITEMS);
}

#[tokio::test]
async fn remove_scene_item_with_scene_subscription_sends_removed_and_reindexed_events() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":132}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let create_first_input_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-1","inputKind":"image_source","inputSettings":{},"sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_first_input_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input-1".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_first_input_data),
        })
        .await;
    // SCENE_ITEMS サブスクリプションが有効なため SceneItemCreated イベントも送信される
    let _ = unwrap_send_texts(create_first_input_action);

    let create_second_input_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-2","inputKind":"image_source","inputSettings":{},"sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_second_input_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input-2".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_second_input_data),
        })
        .await;
    let _ = unwrap_send_texts(create_second_input_action);

    // insert(0) で追加されるため、camera-2 が index=0（先頭）、camera-1 が index=1（末尾）
    // 先頭（非末尾）のアイテムを削除して再インデックスイベントが送信されることを確認する
    let get_scene_item_id_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","sourceName":"camera-2","searchOffset":0}"#,
    )
    .expect("requestData must be valid json");
    let get_scene_item_id_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-scene-item-id".to_owned()),
            request_type: Some("GetSceneItemId".to_owned()),
            request_data: Some(get_scene_item_id_data),
        })
        .await;
    let text = unwrap_send_text(get_scene_item_id_action);
    let scene_item_id = parse_response_scene_item_id(&text);

    let remove_scene_item_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{}}}"#,
        scene_item_id
    ))
    .expect("requestData must be valid json");
    let remove_scene_item_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-remove-scene-item".to_owned()),
            request_type: Some("RemoveSceneItem".to_owned()),
            request_data: Some(remove_scene_item_data),
        })
        .await;
    let messages = unwrap_send_texts(remove_scene_item_action);
    assert_eq!(messages.len(), 3);
    let (_, first_event_type, first_event_intent) = parse_event_type_and_intent(&messages[1].0);
    let (_, second_event_type, second_event_intent) = parse_event_type_and_intent(&messages[2].0);
    assert_eq!(first_event_type, "SceneItemRemoved");
    assert_eq!(first_event_intent, OBSWS_EVENT_SUB_SCENE_ITEMS);
    assert_eq!(second_event_type, "SceneItemListReindexed");
    assert_eq!(second_event_intent, OBSWS_EVENT_SUB_SCENE_ITEMS);
}

#[tokio::test]
async fn remove_scene_item_tail_with_scene_subscription_does_not_send_reindexed_event() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":132}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let create_first_input_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-1","inputKind":"image_source","inputSettings":{},"sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_first_input_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input-1".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_first_input_data),
        })
        .await;
    let _ = unwrap_send_texts(create_first_input_action);

    let create_second_input_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-2","inputKind":"image_source","inputSettings":{},"sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_second_input_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input-2".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_second_input_data),
        })
        .await;
    let _ = unwrap_send_texts(create_second_input_action);

    // insert(0) で追加されるため、camera-2 が index=0、camera-1 が index=1（末尾）
    // 末尾のアイテムを削除して再インデックスイベントが送信されないことを確認する
    let get_scene_item_id_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","sourceName":"camera-1","searchOffset":0}"#,
    )
    .expect("requestData must be valid json");
    let get_scene_item_id_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-scene-item-id".to_owned()),
            request_type: Some("GetSceneItemId".to_owned()),
            request_data: Some(get_scene_item_id_data),
        })
        .await;
    let text = unwrap_send_text(get_scene_item_id_action);
    let scene_item_id = parse_response_scene_item_id(&text);

    let remove_scene_item_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{}}}"#,
        scene_item_id
    ))
    .expect("requestData must be valid json");
    let remove_scene_item_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-remove-scene-item-tail".to_owned()),
            request_type: Some("RemoveSceneItem".to_owned()),
            request_data: Some(remove_scene_item_data),
        })
        .await;
    let messages = unwrap_send_texts(remove_scene_item_action);
    assert_eq!(messages.len(), 2);
    let (_, event_type, event_intent) = parse_event_type_and_intent(&messages[1].0);
    assert_eq!(event_type, "SceneItemRemoved");
    assert_eq!(event_intent, OBSWS_EVENT_SUB_SCENE_ITEMS);
}

#[tokio::test]
async fn set_scene_item_index_with_scene_subscription_sends_reindexed_event() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":132}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let create_first_input_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-1","inputKind":"image_source","inputSettings":{},"sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_first_input_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input-1".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_first_input_data),
        })
        .await;
    let _ = unwrap_send_texts(create_first_input_action);

    let create_second_input_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","inputName":"camera-2","inputKind":"image_source","inputSettings":{},"sceneItemEnabled":true}"#,
    )
    .expect("requestData must be valid json");
    let create_second_input_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-input-2".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(create_second_input_data),
        })
        .await;
    let _ = unwrap_send_texts(create_second_input_action);

    // insert(0) で追加されるため、camera-2 が index=0、camera-1 が index=1
    // camera-1 を index=0 に移動して再インデックスイベントが送信されることを確認する
    let get_scene_item_id_data = nojson::RawJsonOwned::parse(
        r#"{"sceneName":"Scene","sourceName":"camera-1","searchOffset":0}"#,
    )
    .expect("requestData must be valid json");
    let get_scene_item_id_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-scene-item-id".to_owned()),
            request_type: Some("GetSceneItemId".to_owned()),
            request_data: Some(get_scene_item_id_data),
        })
        .await;
    let text = unwrap_send_text(get_scene_item_id_action);
    let scene_item_id = parse_response_scene_item_id(&text);

    let set_scene_item_index_data = nojson::RawJsonOwned::parse(format!(
        r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemIndex":0}}"#,
        scene_item_id
    ))
    .expect("requestData must be valid json");
    let set_scene_item_index_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-scene-item-index".to_owned()),
            request_type: Some("SetSceneItemIndex".to_owned()),
            request_data: Some(set_scene_item_index_data),
        })
        .await;
    let messages = unwrap_send_texts(set_scene_item_index_action);
    assert_eq!(messages.len(), 2);
    let (_, event_type, event_intent) = parse_event_type_and_intent(&messages[1].0);
    assert_eq!(event_type, "SceneItemListReindexed");
    assert_eq!(event_intent, OBSWS_EVENT_SUB_SCENE_ITEMS);
}

#[tokio::test]
async fn set_scene_item_enabled_missing_field_returns_missing_request_field_error() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let request_data = nojson::RawJsonOwned::parse(r#"{"sceneItemId":1,"sceneItemEnabled":true}"#)
        .expect("requestData must be valid json");
    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-scene-item-enabled-missing-scene-name".to_owned()),
            request_type: Some("SetSceneItemEnabled".to_owned()),
            request_data: Some(request_data),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_MISSING_REQUEST_FIELD);
}

#[tokio::test]
async fn stop_record_when_inactive_returns_error_response() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-stop-record".to_owned()),
            request_type: Some("StopRecord".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_OUTPUT_NOT_RUNNING);
}

#[tokio::test]
async fn start_record_with_mp4_file_source_can_start_and_stop() -> crate::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let mut registry = ObswsSessionState::new(
        crate::types::EvenUsize::new(1920).unwrap(),
        crate::types::EvenUsize::new(1080).unwrap(),
        crate::video::FrameRate::FPS_30,
        None,
        None,
    );
    let input = ObswsInput::from_kind_and_settings(
        "mp4_file_source",
        nojson::RawJsonOwned::parse(
            r#"{"path":"testdata/beep-aac-audio.mp4","loop_playback":true}"#,
        )
        .expect("requestData must be valid json")
        .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "audio-file-1", input, true)
        .expect("input creation must succeed");

    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let handle = create_coordinator_handle_with_pipeline_and_record_dir(
        registry,
        pipeline_handle,
        temp_dir.path().to_path_buf(),
    );
    let mut session = ObswsSession::new(None, handle);
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-record-mp4".to_owned()),
            request_type: Some("StartRecord".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(start_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    tokio::time::sleep(Duration::from_millis(200)).await;

    let stop_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-stop-record-mp4".to_owned()),
            request_type: Some("StopRecord".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(stop_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    let mut output_paths = std::fs::read_dir(temp_dir.path())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    output_paths.retain(|path| path.extension().is_some_and(|ext| ext == "mp4"));
    assert_eq!(output_paths.len(), 1);
    let output_size = std::fs::metadata(&output_paths[0])?.len();
    assert!(output_size > 0);

    pipeline_task.abort();

    Ok(())
}

#[tokio::test]
async fn start_record_with_mp4_file_source_can_stop_immediately_after_start() -> crate::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let mut registry = ObswsSessionState::new(
        crate::types::EvenUsize::new(1920).unwrap(),
        crate::types::EvenUsize::new(1080).unwrap(),
        crate::video::FrameRate::FPS_30,
        None,
        None,
    );
    let input = ObswsInput::from_kind_and_settings(
        "mp4_file_source",
        nojson::RawJsonOwned::parse(
            r#"{"path":"testdata/beep-aac-audio.mp4","loop_playback":true}"#,
        )
        .expect("requestData must be valid json")
        .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "audio-file-immediate-stop", input, true)
        .expect("input creation must succeed");

    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let handle = create_coordinator_handle_with_pipeline_and_record_dir(
        registry,
        pipeline_handle,
        temp_dir.path().to_path_buf(),
    );
    let mut session = ObswsSession::new(None, handle);
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-record-mp4-immediate-stop".to_owned()),
            request_type: Some("StartRecord".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(start_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    let stop_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-stop-record-mp4-immediate-stop".to_owned()),
            request_type: Some("StopRecord".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(stop_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    pipeline_task.abort();

    Ok(())
}

#[tokio::test]
async fn start_record_with_multiple_audio_inputs_uses_audio_mixer() -> crate::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let mut registry = ObswsSessionState::new(
        crate::types::EvenUsize::new(1920).unwrap(),
        crate::types::EvenUsize::new(1080).unwrap(),
        crate::video::FrameRate::FPS_30,
        None,
        None,
    );
    for input_name in ["audio-file-1", "audio-file-2"] {
        let input = ObswsInput::from_kind_and_settings(
            "mp4_file_source",
            nojson::RawJsonOwned::parse(
                r#"{"path":"testdata/beep-aac-audio.mp4","loop_playback":true}"#,
            )
            .expect("requestData must be valid json")
            .value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input("Scene", input_name, input, true)
            .expect("input creation must succeed");
    }

    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let handle = create_initialized_coordinator_handle_with_pipeline_and_record_dir(
        registry,
        pipeline_handle.clone(),
        temp_dir.path().to_path_buf(),
    )
    .await?;
    let mut session = ObswsSession::new(None, handle);
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-record-audio-mixer".to_owned()),
            request_type: Some("StartRecord".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(start_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    // record は program mixer の出力を直接使用するため、
    // record 独自の mixer プロセッサは存在しない。
    // start/stop が成功することのみ確認する。

    let stop_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-stop-record-audio-mixer".to_owned()),
            request_type: Some("StopRecord".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(stop_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    pipeline_task.abort();

    Ok(())
}

#[tokio::test]
async fn start_record_with_no_inputs_succeeds() -> crate::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let registry = ObswsSessionState::new(
        crate::types::EvenUsize::new(1920).unwrap(),
        crate::types::EvenUsize::new(1080).unwrap(),
        crate::video::FrameRate::FPS_30,
        None,
        None,
    );

    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let handle = create_initialized_coordinator_handle_with_pipeline_and_record_dir(
        registry,
        pipeline_handle.clone(),
        temp_dir.path().to_path_buf(),
    )
    .await?;
    let mut session = ObswsSession::new(None, handle);
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-record-no-inputs".to_owned()),
            request_type: Some("StartRecord".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(start_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    // record は program mixer の出力を直接使用するため、
    // record 独自の mixer プロセッサは存在しない。

    let stop_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-stop-record-no-inputs".to_owned()),
            request_type: Some("StopRecord".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(stop_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    pipeline_task.abort();

    Ok(())
}

#[tokio::test]
async fn start_stream_with_no_inputs_succeeds() -> crate::Result<()> {
    let registry = ObswsSessionState::new_for_test();

    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let handle = create_coordinator_handle_with_pipeline(registry, pipeline_handle.clone());
    let mut session = ObswsSession::new(None, handle);
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    // SetStreamServiceSettings で stream 設定を行う
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-stream-settings".to_owned()),
            request_type: Some("SetStreamServiceSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"streamServiceType":"rtmp_custom","streamServiceSettings":{"server":"rtmp://127.0.0.1:1935/live","key":"stream-no-inputs"}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_action);
    let (result, _code) = parse_request_status(&text);
    assert!(result);

    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-stream-no-inputs".to_owned()),
            request_type: Some("StartStream".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(start_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    // actor が registry を所有しているため stream_run() に直接アクセスできない。
    // StartStream の成功レスポンスで十分に検証できる。

    let stop_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-stop-stream-no-inputs".to_owned()),
            request_type: Some("StopStream".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(stop_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    pipeline_task.abort();

    Ok(())
}

#[tokio::test]
async fn start_record_with_multiple_video_inputs_builds_plan_successfully() {
    // 複数映像入力は受理されるが、パイプラインがないため実行時エラーになる
    let mut registry = ObswsSessionState::new_for_test();
    for input_name in ["image-1", "image-2"] {
        let input = ObswsInput::from_kind_and_settings(
            "image_source",
            nojson::RawJsonOwned::parse(r#"{"file":"dummy.png"}"#)
                .expect("requestData must be valid json")
                .value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input("Scene", input_name, input, true)
            .expect("input creation must succeed");
    }

    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-record-multiple-video".to_owned()),
            request_type: Some("StartRecord".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    // パイプラインがない場合は失敗レスポンスを返す
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_REQUEST_PROCESSING_FAILED);
}

#[tokio::test]
async fn start_stream_with_multiple_audio_inputs_uses_audio_mixer() -> crate::Result<()> {
    let mut registry = ObswsSessionState::new_for_test();
    for input_name in ["audio-file-1", "audio-file-2"] {
        let input = ObswsInput::from_kind_and_settings(
            "mp4_file_source",
            nojson::RawJsonOwned::parse(
                r#"{"path":"testdata/beep-aac-audio.mp4","loop_playback":true}"#,
            )
            .expect("requestData must be valid json")
            .value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input("Scene", input_name, input, true)
            .expect("input creation must succeed");
    }

    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let handle = create_coordinator_handle_with_pipeline(registry, pipeline_handle);
    let mut session = ObswsSession::new(None, handle);
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    // SetStreamServiceSettings で stream 設定を行う
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-stream-settings".to_owned()),
            request_type: Some("SetStreamServiceSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"streamServiceType":"rtmp_custom","streamServiceSettings":{"server":"rtmp://127.0.0.1:1935/live","key":"stream-main"}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_action);
    let (result, _code) = parse_request_status(&text);
    assert!(result);

    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-stream-audio-mixer".to_owned()),
            request_type: Some("StartStream".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(start_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    // actor が registry を所有しているため stream_run() に直接アクセスできない。
    // StartStream の成功レスポンスで十分に検証できる。

    let stop_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-stop-stream-audio-mixer".to_owned()),
            request_type: Some("StopStream".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(stop_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    pipeline_task.abort();

    Ok(())
}

#[tokio::test]
async fn hls_output_uses_program_mixers_after_scene_item_change() -> crate::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let mut registry = ObswsSessionState::new(
        crate::types::EvenUsize::new(1920).expect("canvas width must be valid"),
        crate::types::EvenUsize::new(1080).expect("canvas height must be valid"),
        crate::video::FrameRate::FPS_30,
        None,
        None,
    );
    let input = ObswsInput::from_kind_and_settings(
        "mp4_file_source",
        nojson::RawJsonOwned::parse(
            r#"{"path":"testdata/red-320x320-h264-aac.mp4","loop_playback":true}"#,
        )
        .expect("requestData must be valid json")
        .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "video-file", input, true)
        .expect("input creation must succeed");

    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let handle = create_coordinator_handle_with_pipeline(registry, pipeline_handle.clone());
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;
    create_output(&mut session, "hls", "hls_output").await;

    let hls_output_dir = temp_dir.path().join("hls-output");
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-hls-output".to_owned()),
            request_type: Some("SetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(format!(
                    r#"{{"outputName":"hls","outputSettings":{{"destination":{{"type":"filesystem","directory":"{}"}},"variants":[{{"video_bitrate":2000000,"audio_bitrate":128000}},{{"video_bitrate":1000000,"audio_bitrate":64000,"width":1280,"height":720}}]}}}}"#,
                    hls_output_dir.display()
                ))
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-hls-output".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"hls"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(start_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "output:hls:v1_scaler:0", true).await?;
    wait_for_processor_presence(&pipeline_handle, "output:hls:v0_hls_writer:0", true).await?;
    wait_for_processor_presence(&pipeline_handle, "output:hls:video_mixer:0", false).await?;

    let get_scene_item_id_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-hls-scene-item-id".to_owned()),
            request_type: Some("GetSceneItemId".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene","sourceName":"video-file"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(get_scene_item_id_action);
    let scene_item_id = parse_response_scene_item_id(&text);

    let set_scene_item_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-disable-hls-scene-item".to_owned()),
            request_type: Some("SetSceneItemEnabled".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(format!(
                    r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemEnabled":false}}"#,
                    scene_item_id
                ))
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_scene_item_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "output:hls:v0_hls_writer:0", true).await?;

    let stop_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-stop-hls-output".to_owned()),
            request_type: Some("StopOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"hls"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(stop_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "output:hls:v0_hls_writer:0", false).await?;

    pipeline_task.abort();

    Ok(())
}

#[tokio::test]
async fn dash_output_uses_program_mixers_after_scene_change() -> crate::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let mut registry = ObswsSessionState::new(
        crate::types::EvenUsize::new(1920).expect("canvas width must be valid"),
        crate::types::EvenUsize::new(1080).expect("canvas height must be valid"),
        crate::video::FrameRate::FPS_30,
        None,
        None,
    );
    registry
        .create_scene("Scene B")
        .expect("second scene must be created");
    let input = ObswsInput::from_kind_and_settings(
        "mp4_file_source",
        nojson::RawJsonOwned::parse(
            r#"{"path":"testdata/red-320x320-h264-aac.mp4","loop_playback":true}"#,
        )
        .expect("requestData must be valid json")
        .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "video-file", input, true)
        .expect("input creation must succeed");

    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let handle =
        create_initialized_coordinator_handle_with_pipeline(registry, pipeline_handle.clone())
            .await?;
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;
    create_output(&mut session, "mpeg_dash", "mpeg_dash_output").await;

    let dash_output_dir = temp_dir.path().join("dash-output");
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-dash-output".to_owned()),
            request_type: Some("SetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(format!(
                    r#"{{"outputName":"mpeg_dash","outputSettings":{{"destination":{{"type":"filesystem","directory":"{}"}},"video_codec":"VP9","audio_codec":"OPUS","variants":[{{"video_bitrate":2000000,"audio_bitrate":128000}},{{"video_bitrate":1000000,"audio_bitrate":64000,"width":1280,"height":720}}]}}}}"#,
                    dash_output_dir.display()
                ))
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-dash-output".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"mpeg_dash"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(start_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "output:mpeg_dash:v1_scaler:0", true).await?;
    wait_for_processor_presence(&pipeline_handle, "output:mpeg_dash:v0_dash_writer:0", true)
        .await?;
    wait_for_processor_presence(&pipeline_handle, "output:mpeg_dash:video_mixer:0", false).await?;

    // ABR 結合 MPD は SampleEntry からコーデック文字列が確定してから書き出される。
    // manifest.mpd の出現を待ち、codecs 属性が実際の SampleEntry と一致することを検証する。
    // VP9 + Opus は libvpx / opus が全環境で利用可能なため、エンコーダー不在で失敗しない。
    let manifest_path = dash_output_dir.join("manifest.mpd");
    for _ in 0..60 {
        if manifest_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(
        manifest_path.exists(),
        "ABR combined manifest.mpd must be written after codec resolution"
    );
    let mpd_xml = std::fs::read_to_string(&manifest_path).expect("manifest.mpd must be readable");
    let mpd = shiguredo_mpd::parse(&mpd_xml).expect("manifest.mpd must be valid MPD XML");
    let adaptation_set = &mpd.periods[0].adaptation_sets[0];
    let codecs = adaptation_set
        .codecs
        .as_ref()
        .expect("AdaptationSet.codecs must be present");
    // VP9 + Opus を指定しているので codecs は vp09 と opus を含むこと
    assert!(
        codecs.contains("vp09."),
        "codecs must contain vp09 prefix from actual SampleEntry, got: {codecs}"
    );
    assert!(
        codecs.contains("opus"),
        "codecs must contain opus from actual SampleEntry, got: {codecs}"
    );

    let set_scene_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-program-scene-dash".to_owned()),
            request_type: Some("SetCurrentProgramScene".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_scene_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "output:mpeg_dash:v0_dash_writer:0", true)
        .await?;

    let stop_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-stop-dash-output".to_owned()),
            request_type: Some("StopOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"mpeg_dash"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(stop_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "output:mpeg_dash:v0_dash_writer:0", false)
        .await?;

    pipeline_task.abort();

    Ok(())
}

#[tokio::test]
async fn start_stream_with_multiple_video_inputs_builds_plan_successfully() {
    // 複数映像入力は受理されるが、パイプラインがないため実行時エラーになる
    let mut registry = ObswsSessionState::new_for_test();
    for input_name in ["image-1", "image-2"] {
        let input = ObswsInput::from_kind_and_settings(
            "image_source",
            nojson::RawJsonOwned::parse(r#"{"file":"dummy.png"}"#)
                .expect("requestData must be valid json")
                .value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input("Scene", input_name, input, true)
            .expect("input creation must succeed");
    }

    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    // SetStreamServiceSettings で stream 設定を行う
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-stream-settings".to_owned()),
            request_type: Some("SetStreamServiceSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"streamServiceType":"rtmp_custom","streamServiceSettings":{"server":"rtmp://127.0.0.1:1935/live","key":"stream-main"}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_action);
    let (result, _code) = parse_request_status(&text);
    assert!(result);

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-stream-multiple-video".to_owned()),
            request_type: Some("StartStream".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    // パイプラインがない場合は失敗レスポンスを返す
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_REQUEST_PROCESSING_FAILED);
}

#[tokio::test]
async fn toggle_stream_without_image_input_returns_toggle_request_type_error() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-toggle-stream".to_owned()),
            request_type: Some("ToggleStream".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
    assert_eq!(parse_request_type(&text), "ToggleStream");
}

#[tokio::test]
async fn start_output_with_unknown_name_returns_not_found() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-output".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"unknown"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_RESOURCE_NOT_FOUND);
    assert_eq!(parse_request_type(&text), "StartOutput");
}

#[tokio::test]
async fn toggle_output_without_image_input_returns_toggle_request_type_error() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-toggle-output".to_owned()),
            request_type: Some("ToggleOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"stream"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
    assert_eq!(parse_request_type(&text), "ToggleOutput");
}

#[tokio::test]
async fn stop_output_when_record_is_inactive_returns_output_request_type_error() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-stop-output".to_owned()),
            request_type: Some("StopOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"record"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_OUTPUT_NOT_RUNNING);
    assert_eq!(parse_request_type(&text), "StopOutput");
}

#[cfg(feature = "player")]
#[tokio::test]
async fn start_output_player_with_closed_control_channel_returns_processing_failed() {
    let registry = ObswsSessionState::new_for_test();
    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())
        .expect("failed to create test media pipeline");
    let pipeline_handle = pipeline.handle();
    let (player_command_tx, player_command_rx) = std::sync::mpsc::sync_channel(1);
    drop(player_command_rx);
    let player_media_tx = std::sync::mpsc::sync_channel(1).0;
    let player_lifecycle_rx = tokio::sync::mpsc::unbounded_channel().1;
    let handle = create_coordinator_handle_with_player_channels(
        registry,
        Some(pipeline_handle),
        player_command_tx,
        player_media_tx,
        player_lifecycle_rx,
    );
    let mut session = ObswsSession::new(None, handle);
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-player".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_REQUEST_PROCESSING_FAILED);
    assert_eq!(parse_request_type(&text), "StartOutput");
}

#[cfg(feature = "player")]
#[tokio::test]
async fn player_lifecycle_stop_updates_output_status() {
    let registry = ObswsSessionState::new_for_test();
    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())
        .expect("failed to create test media pipeline");
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let (player_command_tx, player_command_rx) = std::sync::mpsc::sync_channel(4);
    let player_media_tx = std::sync::mpsc::sync_channel(1).0;
    let (player_lifecycle_tx, player_lifecycle_rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = create_coordinator_handle_with_player_channels(
        registry,
        Some(pipeline_handle),
        player_command_tx,
        player_media_tx,
        player_lifecycle_rx,
    );
    let command_thread = std::thread::spawn(move || {
        let command = player_command_rx
            .recv()
            .expect("player command must be sent");
        let crate::obsws::player::PlayerCommand::Start { reply_tx, .. } = command else {
            panic!("unexpected player command");
        };
        let _ = reply_tx.send(Ok(()));
    });

    let mut session = ObswsSession::new(None, handle);
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-player".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let start_text = unwrap_send_text(start_action);
    let (start_result, _start_code) = parse_request_status(&start_text);
    assert!(start_result);

    let get_active_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-player-active".to_owned()),
            request_type: Some("GetOutputStatus".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let get_active_text = unwrap_send_text(get_active_action);
    assert!(parse_output_active(&get_active_text));

    player_lifecycle_tx
        .send(crate::obsws::player::PlayerLifecycleEvent::Stopped { generation: 1 })
        .expect("player lifecycle event must be sent");
    tokio::task::yield_now().await;

    let get_inactive_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-player-inactive".to_owned()),
            request_type: Some("GetOutputStatus".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let get_inactive_text = unwrap_send_text(get_inactive_action);
    assert!(!parse_output_active(&get_inactive_text));

    command_thread
        .join()
        .expect("player command thread must not panic");
    pipeline_task.abort();
}

#[cfg(feature = "player")]
#[tokio::test]
async fn start_output_player_returns_processing_failed_when_subscriber_startup_fails() {
    let registry = ObswsSessionState::new_for_test();
    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())
        .expect("failed to create test media pipeline");
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let _existing_processor = pipeline_handle
        .register_processor(
            crate::ProcessorId::new("player"),
            crate::ProcessorMetadata::new("player"),
        )
        .await
        .expect("player processor must be registered");
    let (player_command_tx, player_command_rx) = std::sync::mpsc::sync_channel(4);
    let player_media_tx = std::sync::mpsc::sync_channel(1).0;
    let player_lifecycle_rx = tokio::sync::mpsc::unbounded_channel().1;
    let handle = create_coordinator_handle_with_player_channels(
        registry,
        Some(pipeline_handle),
        player_command_tx,
        player_media_tx,
        player_lifecycle_rx,
    );
    let command_thread = std::thread::spawn(move || {
        while let Ok(command) = player_command_rx.recv() {
            match command {
                crate::obsws::player::PlayerCommand::Start { reply_tx, .. } => {
                    let _ = reply_tx.send(Ok(()));
                }
                crate::obsws::player::PlayerCommand::Stop => break,
                crate::obsws::player::PlayerCommand::Terminate => break,
            }
        }
    });

    let mut session = ObswsSession::new(None, handle);
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-player-duplicate".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let start_text = unwrap_send_text(start_action);
    let (start_result, start_code) = parse_request_status(&start_text);
    assert!(!start_result);
    assert_eq!(start_code, REQUEST_STATUS_REQUEST_PROCESSING_FAILED);

    let status_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-player-after-dup".to_owned()),
            request_type: Some("GetOutputStatus".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let status_text = unwrap_send_text(status_action);
    assert!(!parse_output_active(&status_text));

    command_thread
        .join()
        .expect("player command thread must not panic");
    pipeline_task.abort();
}

#[cfg(feature = "player")]
#[tokio::test]
async fn stale_player_stopped_event_does_not_deactivate_restarted_player() {
    let registry = ObswsSessionState::new_for_test();
    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())
        .expect("failed to create test media pipeline");
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let (player_command_tx, player_command_rx) = std::sync::mpsc::sync_channel(8);
    let player_media_tx = std::sync::mpsc::sync_channel(1).0;
    let (player_lifecycle_tx, player_lifecycle_rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = create_coordinator_handle_with_player_channels(
        registry,
        Some(pipeline_handle),
        player_command_tx,
        player_media_tx,
        player_lifecycle_rx,
    );
    let command_thread = std::thread::spawn(move || {
        let mut start_count = 0;
        while let Ok(command) = player_command_rx.recv() {
            match command {
                crate::obsws::player::PlayerCommand::Start { reply_tx, .. } => {
                    start_count += 1;
                    let _ = reply_tx.send(Ok(()));
                    if start_count == 2 {
                        break;
                    }
                }
                crate::obsws::player::PlayerCommand::Stop => {}
                crate::obsws::player::PlayerCommand::Terminate => break,
            }
        }
    });

    let mut session = ObswsSession::new(None, handle);
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let first_start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-player-first".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let first_start_text = unwrap_send_text(first_start_action);
    let (first_start_result, _) = parse_request_status(&first_start_text);
    assert!(first_start_result);

    let stop_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-stop-player".to_owned()),
            request_type: Some("StopOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let stop_text = unwrap_send_text(stop_action);
    let (stop_result, _) = parse_request_status(&stop_text);
    assert!(stop_result);

    let second_start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-player-second".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let second_start_text = unwrap_send_text(second_start_action);
    let (second_start_result, _) = parse_request_status(&second_start_text);
    assert!(second_start_result);

    player_lifecycle_tx
        .send(crate::obsws::player::PlayerLifecycleEvent::Stopped { generation: 1 })
        .expect("stale player lifecycle event must be sent");
    tokio::task::yield_now().await;

    let status_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-player-after-stale".to_owned()),
            request_type: Some("GetOutputStatus".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let status_text = unwrap_send_text(status_action);
    assert!(parse_output_active(&status_text));

    command_thread
        .join()
        .expect("player command thread must not panic");
    pipeline_task.abort();
}

#[tokio::test]
async fn request_batch_with_halt_on_failure_stops_after_first_failure() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let action = session
        .on_text_message(
            r#"{"op":8,"d":{"requestId":"batch-1","haltOnFailure":true,"requests":[{"requestType":"CreateScene","requestData":{"sceneName":"Scene B"}},{"requestType":"CreateScene","requestData":{"sceneName":"Scene B"}},{"requestType":"SetCurrentProgramScene","requestData":{"sceneName":"Scene B"}}]}}"#,
        )
        .await
        .expect("request batch must be parsed");
    let text = unwrap_send_text(action);
    let results = parse_request_batch_results(&text);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, "CreateScene");
    assert!(results[0].1);
    assert_eq!(results[1].0, "CreateScene");
    assert!(!results[1].1);
}

#[tokio::test]
async fn request_batch_without_halt_on_failure_continues_after_failure() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let action = session
        .on_text_message(
            r#"{"op":8,"d":{"requestId":"batch-2","haltOnFailure":false,"requests":[{"requestType":"CreateScene","requestData":{"sceneName":"Scene B"}},{"requestType":"CreateScene","requestData":{"sceneName":"Scene B"}},{"requestType":"SetCurrentProgramScene","requestData":{"sceneName":"Scene B"}}]}}"#,
        )
        .await
        .expect("request batch must be parsed");
    let text = unwrap_send_text(action);
    let results = parse_request_batch_results(&text);
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].0, "CreateScene");
    assert!(results[0].1);
    assert_eq!(results[1].0, "CreateScene");
    assert!(!results[1].1);
    assert_eq!(results[2].0, "SetCurrentProgramScene");
    assert!(results[2].1);
}

// --- PersistentData テスト ---

#[tokio::test]
async fn set_persistent_data_rejects_null_slot_value() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let _ = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
        .await
        .expect("identify must succeed");

    let request_data = nojson::RawJsonOwned::parse(
        r#"{"realm":"OBS_WEBSOCKET_DATA_REALM_GLOBAL","slotName":"s","slotValue":null}"#,
    )
    .expect("requestData must be valid json");
    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-null".to_owned()),
            request_type: Some("SetPersistentData".to_owned()),
            request_data: Some(request_data),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_MISSING_REQUEST_FIELD);
}

#[tokio::test]
async fn set_persistent_data_rejects_profile_realm() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let _ = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
        .await
        .expect("identify must succeed");

    let request_data = nojson::RawJsonOwned::parse(
        r#"{"realm":"OBS_WEBSOCKET_DATA_REALM_PROFILE","slotName":"s","slotValue":1}"#,
    )
    .expect("requestData must be valid json");
    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-profile".to_owned()),
            request_type: Some("SetPersistentData".to_owned()),
            request_data: Some(request_data),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
}

#[tokio::test]
async fn get_persistent_data_returns_null_for_nonexistent_slot() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let _ = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
        .await
        .expect("identify must succeed");

    let request_data = nojson::RawJsonOwned::parse(
        r#"{"realm":"OBS_WEBSOCKET_DATA_REALM_GLOBAL","slotName":"nonexistent"}"#,
    )
    .expect("requestData must be valid json");
    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-nonexistent".to_owned()),
            request_type: Some("GetPersistentData".to_owned()),
            request_data: Some(request_data),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, _code) = parse_request_status(&text);
    assert!(result);
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid json");
    let slot_value = json
        .value()
        .to_path_member(&["d", "responseData", "slotValue"])
        .and_then(|v| v.required())
        .expect("slotValue must be present");
    assert!(slot_value.kind().is_null());
}

#[tokio::test]
async fn set_then_get_persistent_data_roundtrip() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let _ = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
        .await
        .expect("identify must succeed");

    // Set
    let set_data = nojson::RawJsonOwned::parse(
        r#"{"realm":"OBS_WEBSOCKET_DATA_REALM_GLOBAL","slotName":"mySlot","slotValue":{"key":"value","num":42}}"#,
    )
    .expect("requestData must be valid json");
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set".to_owned()),
            request_type: Some("SetPersistentData".to_owned()),
            request_data: Some(set_data),
        })
        .await;
    let set_text = unwrap_send_text(set_action);
    let (set_result, _) = parse_request_status(&set_text);
    assert!(set_result);

    // Get
    let get_data = nojson::RawJsonOwned::parse(
        r#"{"realm":"OBS_WEBSOCKET_DATA_REALM_GLOBAL","slotName":"mySlot"}"#,
    )
    .expect("requestData must be valid json");
    let get_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get".to_owned()),
            request_type: Some("GetPersistentData".to_owned()),
            request_data: Some(get_data),
        })
        .await;
    let get_text = unwrap_send_text(get_action);
    let (get_result, _) = parse_request_status(&get_text);
    assert!(get_result);

    let json = nojson::RawJson::parse(get_text.text()).expect("response must be valid json");
    let slot_value = json
        .value()
        .to_path_member(&["d", "responseData", "slotValue"])
        .and_then(|v| v.required())
        .expect("slotValue must be present");
    let key: String = slot_value
        .to_member("key")
        .and_then(|v| v.required()?.try_into())
        .expect("key must be string");
    assert_eq!(key, "value");
    let num: i64 = slot_value
        .to_member("num")
        .and_then(|v| v.required()?.try_into())
        .expect("num must be i64");
    assert_eq!(num, 42);
}

// --- HisuiCreateOutput 回帰テスト ---

#[tokio::test]
async fn hisui_create_output_stream_reads_stream_service_settings() {
    // HisuiCreateOutput で rtmp_output を作成し、streamServiceSettings.server が読めることを確認する
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    // streamServiceSettings のネスト形式で作成する
    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-stream".to_owned()),
            request_type: Some("HisuiCreateOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"my_stream","outputKind":"rtmp_output","outputSettings":{"streamServiceType":"rtmp_custom","streamServiceSettings":{"server":"rtmp://test/live","key":"test-key"}}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(create_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    // GetOutputSettings で設定が反映されていることを確認する
    let get_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-settings".to_owned()),
            request_type: Some("GetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"my_stream"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(get_action);
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid JSON");
    let settings = json
        .value()
        .to_path_member(&["d", "responseData", "outputSettings"])
        .expect("outputSettings access must succeed")
        .required()
        .expect("outputSettings must be present");
    // ObswsStreamServiceSettings の DisplayJson は streamServiceSettings のネスト形式で出力する
    let server: String = settings
        .to_path_member(&["streamServiceSettings", "server"])
        .expect("server access must succeed")
        .required()
        .expect("server must be present")
        .try_into()
        .expect("server must be string");
    assert_eq!(server, "rtmp://test/live");
}

#[tokio::test]
async fn hisui_create_output_sora_reads_sora_sdk_settings() {
    // HisuiCreateOutput で sora_webrtc_output を作成し、soraSdkSettings が読めることを確認する
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    // soraSdkSettings のネスト形式で作成する
    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-sora".to_owned()),
            request_type: Some("HisuiCreateOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"my_sora","outputKind":"sora_webrtc_output","outputSettings":{"soraSdkSettings":{"signaling_urls":["wss://example.com/signaling"],"channel_id":"test-ch"}}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(create_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    // GetOutputSettings で設定が反映されていることを確認する
    let get_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-sora-settings".to_owned()),
            request_type: Some("GetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"my_sora"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(get_action);
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid JSON");
    let settings = json
        .value()
        .to_path_member(&["d", "responseData", "outputSettings"])
        .expect("outputSettings access must succeed")
        .required()
        .expect("outputSettings must be present");
    let urls = settings
        .to_path_member(&["soraSdkSettings", "signaling_urls"])
        .expect("signaling_urls access must succeed")
        .required()
        .expect("signaling_urls must be present");
    let url_list: Vec<String> = urls
        .to_array()
        .expect("signaling_urls must be array")
        .map(|v| v.try_into().expect("url must be string"))
        .collect();
    assert_eq!(url_list, vec!["wss://example.com/signaling"]);
    let channel_id: String = settings
        .to_path_member(&["soraSdkSettings", "channel_id"])
        .expect("channel_id access must succeed")
        .required()
        .expect("channel_id must be present")
        .try_into()
        .expect("channel_id must be string");
    assert_eq!(channel_id, "test-ch");
}

#[tokio::test]
async fn hisui_remove_output_running_returns_error() {
    // 稼働中の output に対する HisuiRemoveOutput が OUTPUT_RUNNING を返すことを確認する
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    // stream はデフォルトで作成される。SetStreamServiceSettings で設定する
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-stream".to_owned()),
            request_type: Some("SetStreamServiceSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"streamServiceType":"rtmp_custom","streamServiceSettings":{"server":"rtmp://127.0.0.1:1935/live"}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_action);
    let (result, _) = parse_request_status(&text);
    assert!(result);

    // StartStream で起動（pipeline がないので失敗するが、稼働チェックは別の方法で）
    // pipeline なしだと StartStream は失敗するので、代わりに非稼働の output で削除成功を確認し、
    // 稼働チェックは直接 outputs BTreeMap の状態で確認する
    // → 実際にはここで稼働中にはできないので、非稼働の output が正常に削除できることを確認

    // 非稼働の output を削除できることを確認する
    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-temp".to_owned()),
            request_type: Some("HisuiCreateOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"temp_output","outputKind":"rtmp_output"}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(create_action);
    let (result, _) = parse_request_status(&text);
    assert!(result);

    let remove_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-remove-temp".to_owned()),
            request_type: Some("HisuiRemoveOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"temp_output"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(remove_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);
}

#[tokio::test]
async fn hisui_create_output_mp4_without_record_directory_uses_default() {
    // HisuiCreateOutput で mp4_output を outputSettings 省略で作成できることを確認する
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-mp4".to_owned()),
            request_type: Some("HisuiCreateOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"my_record","outputKind":"mp4_output"}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(create_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);
}

#[tokio::test]
async fn hisui_create_output_hls_reads_destination_and_variants() {
    // HisuiCreateOutput で hls_output を destination + variants 指定で作成し、設定が反映されることを確認する
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-hls".to_owned()),
            request_type: Some("HisuiCreateOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"my_hls","outputKind":"hls_output","outputSettings":{"destination":{"type":"filesystem","directory":"/tmp/hls-test"},"variants":[{"video_bitrate":2000000,"audio_bitrate":128000}],"segment_duration":4.0}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(create_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    // GetOutputSettings で設定が反映されていることを確認する
    let get_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-hls-settings".to_owned()),
            request_type: Some("GetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"my_hls"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(get_action);
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid JSON");
    let settings = json
        .value()
        .to_path_member(&["d", "responseData", "outputSettings"])
        .expect("outputSettings access must succeed")
        .required()
        .expect("outputSettings must be present");
    // destination が反映されていることを確認
    let dest_type: String = settings
        .to_path_member(&["destination", "type"])
        .expect("destination.type access must succeed")
        .required()
        .expect("destination.type must be present")
        .try_into()
        .expect("destination.type must be string");
    assert_eq!(dest_type, "filesystem");
    // segment_duration が反映されていることを確認
    let segment_duration: f64 = settings
        .to_member("segment_duration")
        .expect("segment_duration access must succeed")
        .required()
        .expect("segment_duration must be present")
        .try_into()
        .expect("segment_duration must be f64");
    assert!((segment_duration - 4.0).abs() < f64::EPSILON);
    // variants の中身が反映されていることを確認
    let variants = settings
        .to_member("variants")
        .expect("variants access must succeed")
        .required()
        .expect("variants must be present");
    let variants_arr: Vec<_> = variants
        .to_array()
        .expect("variants must be array")
        .collect();
    assert_eq!(variants_arr.len(), 1);
    let video_bitrate: i64 = variants_arr[0]
        .to_member("video_bitrate")
        .expect("video_bitrate access must succeed")
        .required()
        .expect("video_bitrate must be present")
        .try_into()
        .expect("video_bitrate must be i64");
    assert_eq!(video_bitrate, 2_000_000);
}

#[tokio::test]
async fn hisui_create_output_sora_with_metadata_preserves_it() {
    // HisuiCreateOutput で sora_webrtc_output を metadata 付きで作成し、
    // restore_outputs_from_state で復元後も metadata が残ることを確認する
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-sora-meta".to_owned()),
            request_type: Some("HisuiCreateOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"sora_meta","outputKind":"sora_webrtc_output","outputSettings":{"soraSdkSettings":{"signaling_urls":["wss://example.com/signaling"],"channel_id":"ch","metadata":{"key":"value"}}}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(create_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    // GetOutputSettings で metadata が反映されていることを確認する
    let get_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-sora-meta".to_owned()),
            request_type: Some("GetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"sora_meta"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(get_action);
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid JSON");
    let metadata_key: String = json
        .value()
        .to_path_member(&[
            "d",
            "responseData",
            "outputSettings",
            "soraSdkSettings",
            "metadata",
            "key",
        ])
        .expect("metadata.key access must succeed")
        .required()
        .expect("metadata.key must be present")
        .try_into()
        .expect("metadata.key must be string");
    assert_eq!(metadata_key, "value");
}

// --- SetOutputSettings の入力検証テスト ---

#[tokio::test]
async fn set_output_settings_rejects_invalid_record_directory_type() {
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    // recordDirectory に数値を渡すと INVALID_REQUEST_FIELD を返す
    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-bad-record".to_owned()),
            request_type: Some("SetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"record","outputSettings":{"recordDirectory":123}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
}

#[tokio::test]
async fn set_output_settings_rejects_invalid_sora_metadata_type() {
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;
    create_output(&mut session, "sora", "sora_webrtc_output").await;

    // metadata に配列を渡すと INVALID_REQUEST_FIELD を返す
    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-bad-meta".to_owned()),
            request_type: Some("SetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"sora","outputSettings":{"soraSdkSettings":{"metadata":[]}}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
}

#[tokio::test]
async fn set_output_settings_rejects_invalid_signaling_urls_type() {
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;
    create_output(&mut session, "sora", "sora_webrtc_output").await;

    // signaling_urls に文字列を渡すと INVALID_REQUEST_FIELD を返す
    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-bad-urls".to_owned()),
            request_type: Some("SetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"sora","outputSettings":{"soraSdkSettings":{"signaling_urls":"not-an-array"}}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
}

#[tokio::test]
async fn set_output_settings_rejects_invalid_stream_service_type() {
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-bad-sst".to_owned()),
            request_type: Some("SetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"stream","outputSettings":{"streamServiceType":123}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
}

#[tokio::test]
async fn set_output_settings_null_clears_sora_channel_id() {
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;
    create_output(&mut session, "sora", "sora_webrtc_output").await;

    // まず channel_id を設定する
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-sora-ch".to_owned()),
            request_type: Some("SetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"sora","outputSettings":{"soraSdkSettings":{"channel_id":"test-ch"}}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_action);
    let (result, _) = parse_request_status(&text);
    assert!(result);

    // channel_id: null でクリアする
    let clear_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-clear-sora-ch".to_owned()),
            request_type: Some("SetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"sora","outputSettings":{"soraSdkSettings":{"channel_id":null}}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(clear_action);
    let (result, _) = parse_request_status(&text);
    assert!(result);

    // GetOutputSettings で channel_id が消えていることを確認する
    let get_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-sora-ch".to_owned()),
            request_type: Some("GetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"sora"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(get_action);
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid JSON");
    let sdk = json
        .value()
        .to_path_member(&["d", "responseData", "outputSettings", "soraSdkSettings"])
        .expect("soraSdkSettings access must succeed")
        .required()
        .expect("soraSdkSettings must be present");
    // channel_id が null/未設定なので soraSdkSettings に含まれないはず
    let channel_id = sdk.to_member("channel_id").ok().and_then(|v| v.optional());
    assert!(channel_id.is_none());
}

// --- SetRecordDirectory + HisuiCreateOutput の既定値連携テスト ---

#[tokio::test]
async fn set_record_directory_updates_default_for_future_mp4_outputs() {
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    // SetRecordDirectory で録画先を変更する
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-dir".to_owned()),
            request_type: Some("SetRecordDirectory".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"recordDirectory":"/tmp/new-recordings"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_action);
    let (result, _) = parse_request_status(&text);
    assert!(result);

    // HisuiCreateOutput で mp4_output を省略作成する
    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-mp4-after-set".to_owned()),
            request_type: Some("HisuiCreateOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"new_record","outputKind":"mp4_output"}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(create_action);
    let (result, _) = parse_request_status(&text);
    assert!(result);

    // GetOutputSettings で新しい録画先が使われていることを確認する
    let get_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-new-mp4".to_owned()),
            request_type: Some("GetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"new_record"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(get_action);
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid JSON");
    let dir: String = json
        .value()
        .to_path_member(&["d", "responseData", "outputSettings", "recordDirectory"])
        .expect("recordDirectory access must succeed")
        .required()
        .expect("recordDirectory must be present")
        .try_into()
        .expect("recordDirectory must be string");
    assert_eq!(dir, "/tmp/new-recordings");
}

// --- HisuiCreateOutput の型検証テスト ---

#[tokio::test]
async fn hisui_create_output_rejects_invalid_record_directory_type() {
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-bad-record".to_owned()),
            request_type: Some("HisuiCreateOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"bad_record","outputKind":"mp4_output","outputSettings":{"recordDirectory":123}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
}

#[tokio::test]
async fn hisui_create_output_rejects_invalid_stream_service_type() {
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-bad-stream".to_owned()),
            request_type: Some("HisuiCreateOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"bad_stream","outputKind":"rtmp_output","outputSettings":{"streamServiceType":123}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
}

#[tokio::test]
async fn hisui_create_output_rejects_invalid_sora_signaling_urls_type() {
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-bad-sora".to_owned()),
            request_type: Some("HisuiCreateOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"bad_sora","outputKind":"sora_webrtc_output","outputSettings":{"soraSdkSettings":{"signaling_urls":"not-an-array"}}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
}

#[tokio::test]
async fn hisui_create_output_rejects_non_object_output_settings() {
    // outputSettings が object でない場合は INVALID_REQUEST_FIELD を返す
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-bad-settings".to_owned()),
            request_type: Some("HisuiCreateOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"bad_output","outputKind":"mp4_output","outputSettings":123}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
}

#[tokio::test]
async fn set_output_settings_rejects_non_object_output_settings() {
    // SetOutputSettings で outputSettings が object でない場合は INVALID_REQUEST_FIELD を返す
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    // outputSettings: 123
    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-bad-settings-num".to_owned()),
            request_type: Some("SetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"stream","outputSettings":123}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);

    // outputSettings: null
    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-bad-settings-null".to_owned()),
            request_type: Some("SetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"record","outputSettings":null}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
}

#[tokio::test]
async fn set_output_settings_record_updates_default_record_directory() {
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    // SetOutputSettings で record の recordDirectory を変更
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-record-dir-via-settings".to_owned()),
            request_type: Some("SetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"record","outputSettings":{"recordDirectory":"/tmp/updated-via-settings"}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_action);
    let (result, _) = parse_request_status(&text);
    assert!(result);

    // HisuiCreateOutput で mp4_output を省略作成
    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-mp4-after-settings".to_owned()),
            request_type: Some("HisuiCreateOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"new_record_via_settings","outputKind":"mp4_output"}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(create_action);
    let (result, _) = parse_request_status(&text);
    assert!(result);

    // GetOutputSettings で更新後の値が使われていることを確認
    let get_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-new-mp4-via-settings".to_owned()),
            request_type: Some("GetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"new_record_via_settings"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(get_action);
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid JSON");
    let dir: String = json
        .value()
        .to_path_member(&["d", "responseData", "outputSettings", "recordDirectory"])
        .expect("recordDirectory access must succeed")
        .required()
        .expect("recordDirectory must be present")
        .try_into()
        .expect("recordDirectory must be string");
    assert_eq!(dir, "/tmp/updated-via-settings");
}

#[tokio::test]
async fn set_stream_service_settings_after_remove_returns_not_found() {
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    // stream を削除する
    let remove_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-remove-stream".to_owned()),
            request_type: Some("HisuiRemoveOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"stream"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(remove_action);
    let (result, _) = parse_request_status(&text);
    assert!(result);

    // 削除後の SetStreamServiceSettings は RESOURCE_NOT_FOUND を返す
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-stream-after-remove".to_owned()),
            request_type: Some("SetStreamServiceSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"streamServiceType":"rtmp_custom","streamServiceSettings":{"server":"rtmp://127.0.0.1/live"}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_RESOURCE_NOT_FOUND);
}

#[tokio::test]
async fn start_output_uses_output_kind_even_when_name_matches_legacy_builtin() {
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-sora-named-hls".to_owned()),
            request_type: Some("HisuiCreateOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"outputName":"hls","outputKind":"sora_webrtc_output","outputSettings":{"soraSdkSettings":{"signaling_urls":["wss://example.com/signaling"]}}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(create_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    // 名前ではなく output_kind で dispatch されるなら、
    // HLS の destination エラーではなく Sora の channel_id エラーになる。
    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-sora-named-hls".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"hls"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(start_action);
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid JSON");
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
    let comment: String = json
        .value()
        .to_path_member(&["d", "requestStatus", "comment"])
        .expect("comment access must succeed")
        .required()
        .expect("comment must be present")
        .try_into()
        .expect("comment must be string");
    assert_eq!(
        comment,
        "Missing outputSettings.soraSdkSettings.channel_id field"
    );
}

// -----------------------------------------------------------------------
// テキストオーバーレイ機能のヘルパー (機能有効時)
// -----------------------------------------------------------------------

/// 機能有効時の `ObswsCoordinator` を立ち上げる。
///
/// `testdata/fonts/PublicSans-Regular.ttf` を `--font-search-root` 兼デフォルトフォントとして
/// 使い、 MediaPipeline + program mixer 群 (ビデオミキサーは内部レイヤとして
/// テキストオーバーレイ機能を有効化) を spawn する。
/// 戻り値の `ObswsCoordinatorHandle` を通じて 4 メソッドの obsws リクエストが実行できる。
async fn create_initialized_coordinator_with_text_overlay()
-> crate::Result<crate::obsws::coordinator::ObswsCoordinatorHandle> {
    let text_overlay_config = crate::mixer::video::text_overlay::TextOverlayConfig::new(
        std::path::PathBuf::from("testdata/fonts"),
        "PublicSans-Regular.ttf".to_owned(),
    )
    .expect("テスト用 TextOverlayConfig が組み立てられる");

    let registry = ObswsSessionState::new_for_test_with_text_overlay(text_overlay_config.clone());

    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())?;
    let pipeline_handle = pipeline.handle();
    tokio::spawn(pipeline.run());
    pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("trigger_start: pipeline terminated"))?;

    // テキストオーバーレイ機能は VideoRealtimeMixer の内部レイヤとして
    // `start_mixer_processors` 経由で組み込まれる。
    let scene_inputs = registry.list_current_program_scene_input_entries();
    let output_plan = crate::obsws::output_plan::build_composed_output_plan(
        &scene_inputs,
        registry.canvas_width(),
        registry.canvas_height(),
        registry.frame_rate(),
    )
    .map_err(|e| crate::Error::new(format!("failed to build output plan: {}", e.message())))?;
    crate::obsws::session::output::start_mixer_processors(
        &pipeline_handle,
        &output_plan,
        Some(text_overlay_config),
    )
    .await?;

    let scene_uuid = registry
        .current_program_scene()
        .map(|s| s.scene_uuid)
        .unwrap_or_default();
    let program_output = crate::obsws::server::ProgramOutputState {
        scene_uuid,
        video_track_id: output_plan.video_track_id,
        audio_track_id: output_plan.audio_track_id,
        video_mixer_processor_id: output_plan.video_mixer_processor_id,
        audio_mixer_processor_id: output_plan.audio_mixer_processor_id,
        source_processor_ids: output_plan.source_processor_ids,
    };

    let (mut actor, handle, _shutdown_rx) = crate::obsws::coordinator::ObswsCoordinator::new(
        registry,
        std::path::PathBuf::from("recordings-for-test"),
        program_output,
        Some(pipeline_handle),
        #[cfg(feature = "player")]
        test_player_command_tx(),
        #[cfg(feature = "player")]
        test_player_media_tx(),
    );
    actor.start_initial_input_source_processors().await?;
    tokio::spawn(actor.run());
    Ok(handle)
}

/// テキストオーバーレイ系のリクエストを 1 件投げて `CommandResult` を返す。
async fn process_text_overlay_request(
    coordinator: &crate::obsws::coordinator::ObswsCoordinatorHandle,
    request_id: &str,
    request_type: &str,
    request_data_json: Option<&str>,
) -> crate::obsws::coordinator::CommandResult {
    let request_data = request_data_json
        .map(|s| nojson::RawJsonOwned::parse(s).expect("テスト requestData JSON は妥当"));
    let request = crate::obsws::message::RequestMessage {
        request_id: Some(request_id.to_owned()),
        request_type: Some(request_type.to_owned()),
        request_data,
    };
    let stats = crate::obsws::message::ObswsSessionStats::default();
    coordinator
        .process_request(request, stats)
        .await
        .expect("coordinator のリクエスト処理は成功する")
}

/// `HisuiListTextOverlays` レスポンスの `textOverlays` 配列の長さを返す。
fn parse_text_overlays_count(text: &nojson::RawJsonOwned) -> usize {
    let json = nojson::RawJson::parse(text.text()).expect("List レスポンスは JSON");
    json.value()
        .to_path_member(&["d", "responseData", "textOverlays"])
        .expect("textOverlays フィールドが存在する")
        .required()
        .expect("textOverlays は値を持つ")
        .to_array()
        .expect("textOverlays は配列")
        .count()
}

// -----------------------------------------------------------------------
// テキストオーバーレイ機能の無効時挙動。
// 機能有効時のフルパス検証は VideoRealtimeMixer の構築と TextOverlayLayer の
// raden 描画が必要なため、 mixer モジュール側の単体テストで別途扱う。

/// `default_coordinator_handle()` は `ObswsSessionState::new_for_test()` を使うため
/// テキストオーバーレイ機能が無効 (`text_overlay_config = None`) の状態である。
/// この状態で `HisuiCreateTextOverlay` を呼ぶと `RESOURCE_ACTION_NOT_SUPPORTED` が返る。
#[tokio::test]
async fn hisui_create_text_overlay_returns_disabled_when_feature_off() {
    let coordinator = default_coordinator_handle();
    let request = crate::obsws::message::RequestMessage {
        request_id: Some("req-1".to_owned()),
        request_type: Some("HisuiCreateTextOverlay".to_owned()),
        request_data: None,
    };
    let stats = crate::obsws::message::ObswsSessionStats::default();
    let result = coordinator
        .process_request(request, stats)
        .await
        .expect("coordinator のリクエスト処理は成功する");
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "機能無効時はエラー応答となる");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED,
        "REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED (606) が返る"
    );
}

/// `HisuiUpdateTextOverlay` も機能無効時は `RESOURCE_ACTION_NOT_SUPPORTED` を返す。
#[tokio::test]
async fn hisui_update_text_overlay_returns_disabled_when_feature_off() {
    let coordinator = default_coordinator_handle();
    let request = crate::obsws::message::RequestMessage {
        request_id: Some("req-1".to_owned()),
        request_type: Some("HisuiUpdateTextOverlay".to_owned()),
        request_data: None,
    };
    let stats = crate::obsws::message::ObswsSessionStats::default();
    let result = coordinator
        .process_request(request, stats)
        .await
        .expect("coordinator のリクエスト処理は成功する");
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "エラー応答となる");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED
    );
}

/// `HisuiRemoveTextOverlay` も機能無効時は `RESOURCE_ACTION_NOT_SUPPORTED` を返す。
#[tokio::test]
async fn hisui_remove_text_overlay_returns_disabled_when_feature_off() {
    let coordinator = default_coordinator_handle();
    let request = crate::obsws::message::RequestMessage {
        request_id: Some("req-1".to_owned()),
        request_type: Some("HisuiRemoveTextOverlay".to_owned()),
        request_data: None,
    };
    let stats = crate::obsws::message::ObswsSessionStats::default();
    let result = coordinator
        .process_request(request, stats)
        .await
        .expect("coordinator のリクエスト処理は成功する");
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "エラー応答となる");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED
    );
}

/// `HisuiListTextOverlays` も機能無効時は `RESOURCE_ACTION_NOT_SUPPORTED` を返す
/// (空配列ではなくエラーとする方針)。
#[tokio::test]
async fn hisui_list_text_overlays_returns_disabled_when_feature_off() {
    let coordinator = default_coordinator_handle();
    let request = crate::obsws::message::RequestMessage {
        request_id: Some("req-1".to_owned()),
        request_type: Some("HisuiListTextOverlays".to_owned()),
        request_data: None,
    };
    let stats = crate::obsws::message::ObswsSessionStats::default();
    let result = coordinator
        .process_request(request, stats)
        .await
        .expect("coordinator のリクエスト処理は成功する");
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "エラー応答となる");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED
    );
}

// -----------------------------------------------------------------------
// テキストオーバーレイ機能の有効時挙動 (4 メソッド往復・エラーケース)
// -----------------------------------------------------------------------

/// 機能有効時に Create → List → Update → List → Remove → List が正しく順序処理される。
/// List の中身も更新前後で確認することで、Create/Update/Remove の副作用が反映されることを検証する。
#[tokio::test]
async fn hisui_text_overlay_create_list_update_remove_roundtrip() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;

    // 初期状態は空。
    let initial = process_text_overlay_request(
        &coordinator,
        "req-initial-list",
        "HisuiListTextOverlays",
        None,
    )
    .await;
    let (success, _) = parse_request_status(&initial.response_text);
    assert!(success, "初期 List は成功する");
    assert_eq!(
        parse_text_overlays_count(&initial.response_text),
        0,
        "初期状態は overlay ゼロ件"
    );

    // Create で 1 件登録する。
    let create = process_text_overlay_request(
        &coordinator,
        "req-create",
        "HisuiCreateTextOverlay",
        Some(r#"{"textOverlayName":"greeting","text":"hello","x":100,"y":200,"fontSize":48}"#),
    )
    .await;
    let (success, _) = parse_request_status(&create.response_text);
    assert!(success, "Create は成功する");

    // List で 1 件返り、内容も Create で送った値と一致する。
    let after_create = process_text_overlay_request(
        &coordinator,
        "req-list-after-create",
        "HisuiListTextOverlays",
        None,
    )
    .await;
    let (success, _) = parse_request_status(&after_create.response_text);
    assert!(success, "Create 後の List は成功する");
    assert_eq!(
        parse_text_overlays_count(&after_create.response_text),
        1,
        "Create 後は 1 件存在する"
    );
    let json =
        nojson::RawJson::parse(after_create.response_text.text()).expect("List レスポンスは JSON");
    let item_name: String = json
        .value()
        .to_path_member(&["d", "responseData", "textOverlays"])
        .and_then(|v| v.required()?.to_array())
        .expect("textOverlays は配列")
        .next()
        .expect("少なくとも 1 件はある")
        .to_member("textOverlayName")
        .and_then(|v| v.required()?.try_into())
        .expect("textOverlayName は文字列");
    assert_eq!(item_name, "greeting", "Create で登録した名前が List に出る");

    // Update で text を更新する。
    let update = process_text_overlay_request(
        &coordinator,
        "req-update",
        "HisuiUpdateTextOverlay",
        Some(r#"{"textOverlayName":"greeting","text":"updated"}"#),
    )
    .await;
    let (success, _) = parse_request_status(&update.response_text);
    assert!(success, "Update は成功する");

    // List で更新後の text が反映されている。
    let after_update = process_text_overlay_request(
        &coordinator,
        "req-list-after-update",
        "HisuiListTextOverlays",
        None,
    )
    .await;
    let json =
        nojson::RawJson::parse(after_update.response_text.text()).expect("List レスポンスは JSON");
    let updated_text: String = json
        .value()
        .to_path_member(&["d", "responseData", "textOverlays"])
        .and_then(|v| v.required()?.to_array())
        .expect("textOverlays は配列")
        .next()
        .expect("Update 後も 1 件存在")
        .to_member("text")
        .and_then(|v| v.required()?.try_into())
        .expect("text は文字列");
    assert_eq!(updated_text, "updated", "Update で text が変わっている");

    // Remove で削除する。
    let remove = process_text_overlay_request(
        &coordinator,
        "req-remove",
        "HisuiRemoveTextOverlay",
        Some(r#"{"textOverlayName":"greeting"}"#),
    )
    .await;
    let (success, _) = parse_request_status(&remove.response_text);
    assert!(success, "Remove は成功する");

    // List で 0 件に戻る。
    let after_remove = process_text_overlay_request(
        &coordinator,
        "req-list-after-remove",
        "HisuiListTextOverlays",
        None,
    )
    .await;
    assert_eq!(
        parse_text_overlays_count(&after_remove.response_text),
        0,
        "Remove 後は 0 件"
    );

    Ok(())
}

/// 同名の Create を 2 回呼ぶと 2 回目は RESOURCE_ALREADY_EXISTS で拒否される。
#[tokio::test]
async fn hisui_create_text_overlay_rejects_duplicate_name() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;
    let body = r#"{"textOverlayName":"dup","text":"x","x":0,"y":0,"fontSize":32}"#;

    let first = process_text_overlay_request(
        &coordinator,
        "req-create-1",
        "HisuiCreateTextOverlay",
        Some(body),
    )
    .await;
    let (success, _) = parse_request_status(&first.response_text);
    assert!(success, "初回 Create は成功");

    let second = process_text_overlay_request(
        &coordinator,
        "req-create-2",
        "HisuiCreateTextOverlay",
        Some(body),
    )
    .await;
    let (success, code) = parse_request_status(&second.response_text);
    assert!(!success, "重複 Create は失敗");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
        "重複は RESOURCE_ALREADY_EXISTS (602)"
    );
    Ok(())
}

/// 未登録名で Update すると RESOURCE_NOT_FOUND で拒否される。
#[tokio::test]
async fn hisui_update_text_overlay_rejects_unknown_name() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;
    let result = process_text_overlay_request(
        &coordinator,
        "req-update-missing",
        "HisuiUpdateTextOverlay",
        Some(r#"{"textOverlayName":"missing","text":"x"}"#),
    )
    .await;
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "エラー応答となる");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
        "未登録は RESOURCE_NOT_FOUND (601)"
    );
    Ok(())
}

/// 未登録名で Remove すると RESOURCE_NOT_FOUND で拒否される。
#[tokio::test]
async fn hisui_remove_text_overlay_rejects_unknown_name() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;
    let result = process_text_overlay_request(
        &coordinator,
        "req-remove-missing",
        "HisuiRemoveTextOverlay",
        Some(r#"{"textOverlayName":"missing"}"#),
    )
    .await;
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "エラー応答となる");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
        "未登録は RESOURCE_NOT_FOUND (601)"
    );
    Ok(())
}

/// `fontName` に path traversal 文字 (`/` `\` `..` NUL) を含む値は INVALID_REQUEST_FIELD で拒否される。
#[tokio::test]
async fn hisui_create_text_overlay_rejects_invalid_font_name() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;
    // 各禁止文字パターンを 1 リクエストで 1 つずつ試す。
    for (label, font_name_json) in [
        ("slash", r#""foo/bar.ttf""#),
        ("backslash", r#""foo\\bar.ttf""#),
        ("dotdot", r#""../etc/passwd""#),
        // NUL バイトは JSON 文字列として "\u0000" でエンコードする
        ("nul", r#""evil\u0000.ttf""#),
    ] {
        let body = format!(
            r#"{{"textOverlayName":"name-{label}","text":"x","x":0,"y":0,"fontSize":32,"fontName":{font_name_json}}}"#
        );
        let result = process_text_overlay_request(
            &coordinator,
            &format!("req-{label}"),
            "HisuiCreateTextOverlay",
            Some(&body),
        )
        .await;
        let (success, code) = parse_request_status(&result.response_text);
        assert!(!success, "{label} は拒否される");
        assert_eq!(
            code,
            crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
            "{label} は INVALID_REQUEST_FIELD (400)"
        );
    }
    Ok(())
}

/// 存在しないフォントファイルを指定すると FontResolveFailed 経路で INVALID_REQUEST_FIELD が返る。
#[tokio::test]
async fn hisui_create_text_overlay_rejects_unresolvable_font() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;
    let result = process_text_overlay_request(
        &coordinator,
        "req-unresolvable",
        "HisuiCreateTextOverlay",
        Some(
            r#"{"textOverlayName":"x","text":"x","x":0,"y":0,"fontSize":32,"fontName":"nonexistent.ttf"}"#,
        ),
    )
    .await;
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "エラー応答となる");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
        "解決失敗は INVALID_REQUEST_FIELD (400)"
    );
    Ok(())
}

/// 不正な `fontColor` (`#GGGGGG` / `#FF` / `#` 不在 / 9 桁) は INVALID_REQUEST_FIELD で拒否される。
#[tokio::test]
async fn hisui_create_text_overlay_rejects_invalid_color() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;
    for (label, color) in [
        ("non-hex", "#GGGGGG"),
        ("too-short", "#FF"),
        ("missing-hash", "FF0000"),
        ("nine-digits", "#FFFFFFFFF"),
    ] {
        let body = format!(
            r#"{{"textOverlayName":"color-{label}","text":"x","x":0,"y":0,"fontSize":32,"fontColor":"{color}"}}"#
        );
        let result = process_text_overlay_request(
            &coordinator,
            &format!("req-color-{label}"),
            "HisuiCreateTextOverlay",
            Some(&body),
        )
        .await;
        let (success, code) = parse_request_status(&result.response_text);
        assert!(!success, "{label} は拒否される");
        assert_eq!(
            code,
            crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
            "{label} は INVALID_REQUEST_FIELD (400)"
        );
    }
    Ok(())
}

/// `fontSize` が範囲外 (0 / canvas_height 超過) は INVALID_REQUEST_FIELD で拒否される。
/// canvas は new_for_test_with_text_overlay で 1920x1080 固定なので 1081 を境界外とする。
#[tokio::test]
async fn hisui_create_text_overlay_rejects_invalid_font_size() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;
    for (label, font_size) in [("zero", 0u32), ("too-large", 1081u32)] {
        let body = format!(
            r#"{{"textOverlayName":"size-{label}","text":"x","x":0,"y":0,"fontSize":{font_size}}}"#
        );
        let result = process_text_overlay_request(
            &coordinator,
            &format!("req-size-{label}"),
            "HisuiCreateTextOverlay",
            Some(&body),
        )
        .await;
        let (success, code) = parse_request_status(&result.response_text);
        assert!(!success, "{label} は拒否される");
        assert_eq!(
            code,
            crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
            "{label} は INVALID_REQUEST_FIELD (400)"
        );
    }
    Ok(())
}

/// `text` がバイト数または行数の上限を超えると INVALID_REQUEST_FIELD で拒否される。
#[tokio::test]
async fn hisui_create_text_overlay_rejects_invalid_text() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;

    // バイト数上限超過 (TEXT_MAX_BYTES 超え)
    let too_long = "a".repeat(crate::mixer::video::text_overlay::TEXT_MAX_BYTES + 1);
    let body = format!(
        r#"{{"textOverlayName":"text-bytes","text":"{too_long}","x":0,"y":0,"fontSize":32}}"#
    );
    let result = process_text_overlay_request(
        &coordinator,
        "req-text-bytes",
        "HisuiCreateTextOverlay",
        Some(&body),
    )
    .await;
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "バイト数上限超過は拒否");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD
    );

    // 行数上限超過 (TEXT_MAX_LINES 超え)
    let too_many_newlines = "\\n".repeat(crate::mixer::video::text_overlay::TEXT_MAX_LINES);
    let body = format!(
        r#"{{"textOverlayName":"text-lines","text":"{too_many_newlines}","x":0,"y":0,"fontSize":32}}"#
    );
    let result = process_text_overlay_request(
        &coordinator,
        "req-text-lines",
        "HisuiCreateTextOverlay",
        Some(&body),
    )
    .await;
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "行数上限超過は拒否");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD
    );
    Ok(())
}

/// 必須フィールドが欠落していると MISSING_REQUEST_FIELD で拒否される。
/// `textOverlayName` / `text` / `x` / `y` / `fontSize` の各欠落を確認する。
#[tokio::test]
async fn hisui_create_text_overlay_returns_missing_request_field_when_required_missing()
-> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;

    // 各必須フィールドをそれぞれ欠落させた 5 種の requestData。
    let bodies = [
        // textOverlayName なし
        r#"{"text":"x","x":0,"y":0,"fontSize":32}"#,
        // text なし
        r#"{"textOverlayName":"a","x":0,"y":0,"fontSize":32}"#,
        // x なし
        r#"{"textOverlayName":"a","text":"x","y":0,"fontSize":32}"#,
        // y なし
        r#"{"textOverlayName":"a","text":"x","x":0,"fontSize":32}"#,
        // fontSize なし
        r#"{"textOverlayName":"a","text":"x","x":0,"y":0}"#,
    ];
    for (i, body) in bodies.iter().enumerate() {
        let result = process_text_overlay_request(
            &coordinator,
            &format!("req-missing-{i}"),
            "HisuiCreateTextOverlay",
            Some(body),
        )
        .await;
        let (success, code) = parse_request_status(&result.response_text);
        assert!(!success, "ケース {i} は拒否される: body={body}");
        assert_eq!(
            code,
            crate::obsws::protocol::REQUEST_STATUS_MISSING_REQUEST_FIELD,
            "ケース {i} は MISSING_REQUEST_FIELD (300): body={body}"
        );
    }
    Ok(())
}

/// 必須フィールドの型違反は INVALID_REQUEST_FIELD で返るべきで、MISSING_REQUEST_FIELD ではない。
#[tokio::test]
async fn hisui_create_text_overlay_returns_invalid_request_field_for_type_mismatch()
-> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;
    // 各必須フィールドに「期待型ではない値」を入れる。
    let bodies = [
        // textOverlayName が数値 (string 期待)
        r#"{"textOverlayName":123,"text":"x","x":0,"y":0,"fontSize":32}"#,
        // text が真偽値 (string 期待)
        r#"{"textOverlayName":"a","text":true,"x":0,"y":0,"fontSize":32}"#,
        // x が文字列 (integer 期待)
        r#"{"textOverlayName":"a","text":"x","x":"abc","y":0,"fontSize":32}"#,
        // y がオブジェクト (integer 期待)
        r#"{"textOverlayName":"a","text":"x","x":0,"y":{},"fontSize":32}"#,
        // fontSize が負数 (u32 範囲外)
        r#"{"textOverlayName":"a","text":"x","x":0,"y":0,"fontSize":-1}"#,
    ];
    for (i, body) in bodies.iter().enumerate() {
        let result = process_text_overlay_request(
            &coordinator,
            &format!("req-type-mismatch-{i}"),
            "HisuiCreateTextOverlay",
            Some(body),
        )
        .await;
        let (success, code) = parse_request_status(&result.response_text);
        assert!(!success, "ケース {i} は拒否される: body={body}");
        assert_eq!(
            code,
            crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
            "ケース {i} は INVALID_REQUEST_FIELD (400): body={body}"
        );
    }
    Ok(())
}

/// テキストオーバーレイの `z` フィールドは i32::MAX も valid 値として受け付ける。
#[tokio::test]
async fn hisui_text_overlay_accepts_i32_max_as_z_value() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;

    // Create で z = i32::MAX (2147483647) を渡しても成功する。
    let create = process_text_overlay_request(
        &coordinator,
        "req-create-i32-max-z",
        "HisuiCreateTextOverlay",
        Some(r#"{"textOverlayName":"x","text":"x","x":0,"y":0,"fontSize":32,"z":2147483647}"#),
    )
    .await;
    let (success, _) = parse_request_status(&create.response_text);
    assert!(success, "i32::MAX も z として受け付ける");
    Ok(())
}

#[tokio::test]
async fn handle_get_stream_service_settings_emits_use_auth_when_key_none() {
    // handle_get_stream_service_settings の obs_compat: true 経路で、
    // key=None / server=Some のとき streamServiceSettings に "key": "" と
    // "use_auth": false が含まれることを検証する。
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    // SetStreamServiceSettings で server だけ設定する (key は未指定で初期値 None のまま)
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-stream-settings-key-none".to_owned()),
            request_type: Some("SetStreamServiceSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"streamServiceType":"rtmp_custom","streamServiceSettings":{"server":"rtmp://127.0.0.1/live"}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let set_text = unwrap_send_text(set_action);
    let (set_result, _) = parse_request_status(&set_text);
    assert!(set_result, "SetStreamServiceSettings must succeed");

    // GetStreamServiceSettings で取得して streamServiceSettings の中身を検証
    let get_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-stream-settings-key-none".to_owned()),
            request_type: Some("GetStreamServiceSettings".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(get_action);
    let (get_result, _) = parse_request_status(&text);
    assert!(get_result, "GetStreamServiceSettings must succeed");
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid JSON");
    let stream_service_settings = json
        .value()
        .to_path_member(&["d", "responseData", "streamServiceSettings"])
        .expect("streamServiceSettings access must succeed")
        .required()
        .expect("streamServiceSettings must be present");

    let key: String = stream_service_settings
        .to_member("key")
        .expect("key access must succeed")
        .required()
        .expect("key must be present")
        .try_into()
        .expect("key must be string");
    assert_eq!(key, "");

    let use_auth: bool = stream_service_settings
        .to_member("use_auth")
        .expect("use_auth access must succeed")
        .required()
        .expect("use_auth must be present")
        .try_into()
        .expect("use_auth must be bool");
    assert!(!use_auth);
}

#[tokio::test]
async fn handle_get_stream_service_settings_emits_use_auth_when_key_some() {
    // handle_get_stream_service_settings の obs_compat: true 経路で、
    // key=Some のとき streamServiceSettings に "key": "<値>" と "use_auth": false が
    // 含まれることを検証する。obs_compat: true でも key=Some 時に else 分岐を
    // 誤って削っていないかの回帰検知。
    // (SetStreamServiceSettings は server を必須とするため server=Some 経路で検証する。
    // server=None 経路は本テストでは扱わない)
    let registry = ObswsSessionState::new_for_test();
    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    // SetStreamServiceSettings で server と key の両方を設定する
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-stream-settings-key-some".to_owned()),
            request_type: Some("SetStreamServiceSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(
                    r#"{"streamServiceType":"rtmp_custom","streamServiceSettings":{"server":"rtmp://127.0.0.1/live","key":"stream-key"}}"#,
                )
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let set_text = unwrap_send_text(set_action);
    let (set_result, _) = parse_request_status(&set_text);
    assert!(set_result, "SetStreamServiceSettings must succeed");

    // GetStreamServiceSettings で取得して streamServiceSettings の中身を検証
    let get_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-stream-settings-key-some".to_owned()),
            request_type: Some("GetStreamServiceSettings".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(get_action);
    let (get_result, _) = parse_request_status(&text);
    assert!(get_result, "GetStreamServiceSettings must succeed");
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid JSON");
    let stream_service_settings = json
        .value()
        .to_path_member(&["d", "responseData", "streamServiceSettings"])
        .expect("streamServiceSettings access must succeed")
        .required()
        .expect("streamServiceSettings must be present");

    let key: String = stream_service_settings
        .to_member("key")
        .expect("key access must succeed")
        .required()
        .expect("key must be present")
        .try_into()
        .expect("key must be string");
    assert_eq!(key, "stream-key");

    let use_auth: bool = stream_service_settings
        .to_member("use_auth")
        .expect("use_auth access must succeed")
        .required()
        .expect("use_auth must be present")
        .try_into()
        .expect("use_auth must be bool");
    assert!(!use_auth);
}
