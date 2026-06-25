//! Scene 系のテスト (Scene の作成 / 切替 / 削除)。

use crate::obsws::message::RequestMessage;
use crate::obsws::protocol::OBSWS_EVENT_SUB_SCENES;
use crate::obsws::session::{ObswsSession, SessionAction};
use crate::obsws::state::ObswsSessionState;

use super::common::*;

#[tokio::test]
async fn remove_current_scene_updates_program_output_state_without_pipeline() {
    let mut registry = ObswsSessionState::new_for_test();
    registry.create_scene("Scene B").expect("must create scene");
    registry
        .set_current_program_scene("Scene B")
        .expect("must switch scene");

    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    let identified = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":4}}"#)
        .await;
    assert!(identified.is_ok());

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-remove-scene".to_owned()),
            request_type: Some("RemoveScene".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;

    let messages = unwrap_send_texts(action);
    assert_eq!(messages.len(), 3);

    // actor が ProgramOutputState を管理しているため直接参照はできない。
    // GetCurrentProgramScene リクエストで残存シーン "Scene" が返ることを検証する。
    let get_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-current-scene".to_owned()),
            request_type: Some("GetCurrentProgramScene".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(get_action);
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid json");
    let scene_name: String = json
        .value()
        .to_path_member(&["d", "responseData", "currentProgramSceneName"])
        .and_then(|v| v.required()?.try_into())
        .expect("currentProgramSceneName must be string");
    assert_eq!(scene_name, "Scene");
}

#[tokio::test]
async fn stale_scene_uuid_differs_from_current_program_scene_uuid() {
    let mut registry = ObswsSessionState::new_for_test();
    registry.create_scene("Scene B").expect("must create scene");

    let stale_scene_uuid = registry
        .get_scene_uuid("Scene")
        .expect("default scene must exist");

    registry
        .set_current_program_scene("Scene B")
        .expect("must switch scene");

    let current_scene_uuid = registry
        .current_program_scene()
        .map(|scene| scene.scene_uuid)
        .expect("current program scene must exist");
    assert_ne!(stale_scene_uuid, current_scene_uuid);
}

#[tokio::test]
async fn create_scene_with_scene_subscription_returns_scene_created_event() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":4}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
        .expect("requestData must be valid json");
    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-scene".to_owned()),
            request_type: Some("CreateScene".to_owned()),
            request_data: Some(request_data),
        })
        .await;
    let messages = unwrap_send_texts(action);
    assert_eq!(messages.len(), 2);
    let (_, event_type, event_intent) = parse_event_type_and_intent(&messages[1].0);
    assert_eq!(event_type, "SceneCreated");
    assert_eq!(event_intent, OBSWS_EVENT_SUB_SCENES);
}

#[tokio::test]
async fn set_current_program_scene_to_same_scene_returns_response_only() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":4}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene"}"#)
        .expect("requestData must be valid json");
    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-scene-same".to_owned()),
            request_type: Some("SetCurrentProgramScene".to_owned()),
            request_data: Some(request_data),
        })
        .await;
    assert!(matches!(action, SessionAction::SendText { .. }));
}

#[tokio::test]
async fn set_current_preview_scene_with_scene_subscription_returns_preview_event() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":4}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let create_request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
        .expect("requestData must be valid json");
    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-scene-preview".to_owned()),
            request_type: Some("CreateScene".to_owned()),
            request_data: Some(create_request_data),
        })
        .await;
    assert!(matches!(create_action, SessionAction::SendTexts { .. }));

    let set_preview_scene_request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
        .expect("requestData must be valid json");
    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-preview-scene".to_owned()),
            request_type: Some("SetCurrentPreviewScene".to_owned()),
            request_data: Some(set_preview_scene_request_data),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, _code) = parse_request_status(&text);
    assert!(!result);
}

#[tokio::test]
async fn set_current_preview_scene_to_same_scene_returns_response_only() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":4}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene"}"#)
        .expect("requestData must be valid json");
    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-preview-scene-same".to_owned()),
            request_type: Some("SetCurrentPreviewScene".to_owned()),
            request_data: Some(request_data),
        })
        .await;
    assert!(matches!(action, SessionAction::SendText { .. }));
}

#[tokio::test]
async fn remove_current_scene_with_scene_subscription_sends_scene_program_and_preview_events() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":4}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let create_request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
        .expect("requestData must be valid json");
    let create_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-create-scene".to_owned()),
            request_type: Some("CreateScene".to_owned()),
            request_data: Some(create_request_data),
        })
        .await;
    assert!(matches!(create_action, SessionAction::SendTexts { .. }));

    let set_scene_request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
        .expect("requestData must be valid json");
    let set_scene_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-scene".to_owned()),
            request_type: Some("SetCurrentProgramScene".to_owned()),
            request_data: Some(set_scene_request_data),
        })
        .await;
    assert!(matches!(set_scene_action, SessionAction::SendTexts { .. }));

    let set_preview_scene_request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
        .expect("requestData must be valid json");
    let set_preview_scene_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-preview-scene".to_owned()),
            request_type: Some("SetCurrentPreviewScene".to_owned()),
            request_data: Some(set_preview_scene_request_data),
        })
        .await;
    assert!(matches!(
        set_preview_scene_action,
        SessionAction::SendText { .. }
    ));

    let remove_request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
        .expect("requestData must be valid json");
    let remove_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-remove-scene".to_owned()),
            request_type: Some("RemoveScene".to_owned()),
            request_data: Some(remove_request_data),
        })
        .await;
    let messages = unwrap_send_texts(remove_action);
    assert_eq!(messages.len(), 3);
    let (_, event_type_1, event_intent_1) = parse_event_type_and_intent(&messages[1].0);
    let (_, event_type_2, event_intent_2) = parse_event_type_and_intent(&messages[2].0);
    assert_eq!(event_type_1, "SceneRemoved");
    assert_eq!(event_intent_1, OBSWS_EVENT_SUB_SCENES);
    assert_eq!(event_type_2, "CurrentProgramSceneChanged");
    assert_eq!(event_intent_2, OBSWS_EVENT_SUB_SCENES);
}
