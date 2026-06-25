//! SceneItem 系のテスト (SceneItem の作成 / 削除 / プロパティ更新 / 再インデックス)。
//!
//! 旧 `src/obsws/session/tests.rs` の line 1054-1558 から物理移動した 9 件を集約する。

use crate::obsws::message::RequestMessage;
use crate::obsws::protocol::{
    OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED, OBSWS_EVENT_SUB_SCENE_ITEMS,
    REQUEST_STATUS_MISSING_REQUEST_FIELD,
};
use crate::obsws::session::{ObswsSession, SessionAction};

use super::common::*;

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
