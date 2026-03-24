use super::*;
use crate::obsws_auth::build_authentication_response;
use crate::obsws_input_registry::{ObswsInput, ObswsStreamServiceSettings};
use crate::obsws_message::RequestMessage;
use crate::obsws_protocol::{
    OBSWS_CLOSE_ALREADY_IDENTIFIED, OBSWS_CLOSE_AUTHENTICATION_FAILED, OBSWS_CLOSE_NOT_IDENTIFIED,
    OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION, OBSWS_EVENT_SUB_GENERAL, OBSWS_EVENT_SUB_INPUTS,
    OBSWS_EVENT_SUB_OUTPUTS, OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED,
    OBSWS_EVENT_SUB_SCENE_ITEMS, OBSWS_EVENT_SUB_SCENES, REQUEST_STATUS_INVALID_REQUEST_FIELD,
    REQUEST_STATUS_MISSING_REQUEST_FIELD, REQUEST_STATUS_OUTPUT_NOT_RUNNING,
    REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

fn input_registry() -> Arc<RwLock<ObswsInputRegistry>> {
    Arc::new(RwLock::new(ObswsInputRegistry::new_for_test()))
}

fn program_output() -> Arc<RwLock<crate::obsws_server::ProgramOutputState>> {
    Arc::new(RwLock::new(crate::obsws_server::ProgramOutputState {
        scene_name: "Scene".to_owned(),
        video_track_id: crate::TrackId::new("obsws:program:0:mixed_video"),
        audio_track_id: crate::TrackId::new("obsws:program:0:mixed_audio"),
        video_mixer_processor_id: crate::ProcessorId::new("obsws:program:0:video_mixer"),
        audio_mixer_processor_id: crate::ProcessorId::new("obsws:program:0:audio_mixer"),
        source_processor_ids: Vec::new(),
    }))
}

fn parse_request_status(text: &nojson::RawJsonOwned) -> (bool, i64) {
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid json");
    let status = json
        .value()
        .to_path_member(&["d", "requestStatus"])
        .expect("requestStatus access must succeed")
        .required()
        .expect("requestStatus must exist");
    let result: bool = status
        .to_member("result")
        .and_then(|v| v.required()?.try_into())
        .expect("result must be bool");
    let code: i64 = status
        .to_member("code")
        .and_then(|v| v.required()?.try_into())
        .expect("code must be i64");
    (result, code)
}

fn parse_request_type(text: &nojson::RawJsonOwned) -> String {
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid json");
    json.value()
        .to_path_member(&["d", "requestType"])
        .and_then(|v| v.required()?.try_into())
        .expect("requestType must be string")
}

fn parse_response_scene_item_id(text: &nojson::RawJsonOwned) -> i64 {
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid json");
    json.value()
        .to_path_member(&["d", "responseData", "sceneItemId"])
        .and_then(|v| v.required()?.try_into())
        .expect("sceneItemId must be i64")
}

fn parse_identified_message(text: &nojson::RawJsonOwned) -> (i64, u32) {
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid json");
    let op: i64 = json
        .value()
        .to_member("op")
        .and_then(|v| v.required()?.try_into())
        .expect("op must be i64");
    let negotiated_rpc_version: u32 = json
        .value()
        .to_path_member(&["d", "negotiatedRpcVersion"])
        .and_then(|v| v.required()?.try_into())
        .expect("negotiatedRpcVersion must be u32");
    (op, negotiated_rpc_version)
}

fn parse_event_type_and_intent(text: &nojson::RawJsonOwned) -> (i64, String, u32) {
    let json = nojson::RawJson::parse(text.text()).expect("event must be valid json");
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
    (op, event_type, event_intent)
}

fn parse_request_batch_results(text: &nojson::RawJsonOwned) -> Vec<(String, bool, i64)> {
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid json");
    let mut results = json
        .value()
        .to_path_member(&["d", "results"])
        .expect("results access must succeed")
        .required()
        .expect("results must exist")
        .to_array()
        .expect("results must be array");
    results
        .by_ref()
        .map(|result| {
            let request_type: String = result
                .to_member("requestType")
                .and_then(|v| v.required()?.try_into())
                .expect("requestType must be string");
            let request_status = result
                .to_member("requestStatus")
                .expect("requestStatus access must succeed")
                .required()
                .expect("requestStatus must exist");
            let success: bool = request_status
                .to_member("result")
                .and_then(|v| v.required()?.try_into())
                .expect("result must be bool");
            let code: i64 = request_status
                .to_member("code")
                .and_then(|v| v.required()?.try_into())
                .expect("code must be i64");
            (request_type, success, code)
        })
        .collect()
}

/// SessionAction::SendText から text を取り出す。SendText でなければパニック。
fn unwrap_send_text(action: SessionAction) -> nojson::RawJsonOwned {
    let SessionAction::SendText { text, .. } = action else {
        panic!("expected SendText");
    };
    text
}

/// SessionAction::SendTexts から messages を取り出す。SendTexts でなければパニック。
fn unwrap_send_texts(action: SessionAction) -> Vec<(nojson::RawJsonOwned, &'static str)> {
    let SessionAction::SendTexts { messages } = action else {
        panic!("expected SendTexts");
    };
    messages
}

/// SessionAction::Close から code と reason を取り出す。Close でなければパニック。
fn unwrap_close(action: SessionAction) -> (CloseCode, &'static str) {
    let SessionAction::Close { code, reason, .. } = action else {
        panic!("expected Close");
    };
    (code, reason)
}

#[test]
fn on_connected_returns_hello_message_action() {
    let session = ObswsSession::new(None, input_registry(), None, program_output());
    let action = session.on_connected();
    let SessionAction::SendText { text, message_name } = action else {
        panic!("must be SendText");
    };
    assert_eq!(message_name, "hello message");
    assert!(text.text().contains("\"op\":0"));
}

#[tokio::test]
async fn on_request_before_identify_returns_close_action() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-1".to_owned()),
            request_type: Some("GetVersion".to_owned()),
            request_data: None,
        })
        .await;
    let (code, reason) = unwrap_close(action);
    assert_eq!(code, OBSWS_CLOSE_NOT_IDENTIFIED);
    assert_eq!(reason, "identify is required");
}

#[tokio::test]
async fn broadcast_custom_event_returns_event_when_general_subscription_enabled() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
    let identified = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":1}}"#)
        .await;
    assert!(identified.is_ok());

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-custom-event".to_owned()),
            request_type: Some("BroadcastCustomEvent".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"eventData":{"message":"hello"}}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let messages = unwrap_send_texts(action);
    assert_eq!(messages.len(), 2);

    let (_, event_type, event_intent) = parse_event_type_and_intent(&messages[1].0);
    let event_json =
        nojson::RawJson::parse(messages[1].0.text()).expect("event must be valid json");
    let message: String = event_json
        .value()
        .to_path_member(&["d", "eventData", "message"])
        .and_then(|v| v.required()?.try_into())
        .expect("message must be string");
    assert_eq!(event_type, "CustomEvent");
    assert_eq!(event_intent, OBSWS_EVENT_SUB_GENERAL);
    assert_eq!(message, "hello");
}

#[tokio::test]
async fn sleep_request_returns_success_response() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
    let identified = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await;
    assert!(identified.is_ok());

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-sleep".to_owned()),
            request_type: Some("Sleep".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"sleepMillis":0}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);
}

#[tokio::test]
async fn sleep_request_rejects_too_large_sleep_millis() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
    let identified = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await;
    assert!(identified.is_ok());

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-sleep-invalid".to_owned()),
            request_type: Some("Sleep".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"sleepMillis":50001}"#)
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
async fn duplicate_identify_returns_already_identified_close() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
    let first = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await;
    assert!(first.is_ok());

    let second = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await;
    let action = second.expect("second identify must return action");
    let (code, reason) = unwrap_close(action);
    assert_eq!(code, OBSWS_CLOSE_ALREADY_IDENTIFIED);
    assert_eq!(reason, "already identified");
}

#[tokio::test]
async fn reidentify_before_identify_returns_not_identified_close() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
    let action = session
        .on_text_message(r#"{"op":3,"d":{}}"#)
        .await
        .expect("reidentify must be parsed");
    let (code, reason) = unwrap_close(action);
    assert_eq!(code, OBSWS_CLOSE_NOT_IDENTIFIED);
    assert_eq!(reason, "identify is required");
}

#[tokio::test]
async fn reidentify_after_identify_returns_identified_message() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let action = session
        .on_text_message(r#"{"op":3,"d":{"eventSubscriptions":1023}}"#)
        .await
        .expect("reidentify must be parsed");
    let SessionAction::SendText { text, message_name } = action else {
        panic!("must be SendText");
    };
    assert_eq!(message_name, "identified message");
    let (op, negotiated_rpc_version) = parse_identified_message(&text);
    assert_eq!(op, 2);
    assert_eq!(negotiated_rpc_version, 1);
}

#[tokio::test]
async fn identify_without_event_subscriptions_defaults_to_all() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
    let action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(action, SessionAction::SendText { .. }));
    assert_eq!(session.event_subscriptions, OBSWS_EVENT_SUB_ALL);
}

#[tokio::test]
async fn identify_with_event_subscriptions_updates_session_state() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
    let action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":64}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(action, SessionAction::SendText { .. }));
    assert_eq!(session.event_subscriptions, OBSWS_EVENT_SUB_OUTPUTS);
}

#[tokio::test]
async fn reidentify_updates_event_subscriptions_when_specified() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":1}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));
    assert_eq!(session.event_subscriptions, 1);

    let reidentify_action = session
        .on_text_message(r#"{"op":3,"d":{"eventSubscriptions":64}}"#)
        .await
        .expect("reidentify must succeed");
    assert!(matches!(reidentify_action, SessionAction::SendText { .. }));
    assert_eq!(session.event_subscriptions, OBSWS_EVENT_SUB_OUTPUTS);
}

#[tokio::test]
async fn reidentify_without_event_subscriptions_keeps_previous_value() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":64}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let reidentify_action = session
        .on_text_message(r#"{"op":3,"d":{}}"#)
        .await
        .expect("reidentify must succeed");
    assert!(matches!(reidentify_action, SessionAction::SendText { .. }));
    assert_eq!(session.event_subscriptions, OBSWS_EVENT_SUB_OUTPUTS);
}

#[tokio::test]
async fn create_scene_with_scene_subscription_returns_scene_created_event() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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

#[tokio::test]
async fn create_and_remove_input_with_input_subscription_send_input_events() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
async fn unsupported_rpc_version_returns_close_action() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
    let action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":2}}"#)
        .await
        .expect("identify must be parsed");
    let (code, reason) = unwrap_close(action);
    assert_eq!(code, OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION);
    assert_eq!(reason, "unsupported rpc version");
}

#[tokio::test]
async fn invalid_authentication_returns_close_action() {
    let auth = ObswsAuthentication {
        salt: "test-salt".to_owned(),
        challenge: "test-challenge".to_owned(),
        expected_response: build_authentication_response(
            "test-password",
            "test-salt",
            "test-challenge",
        ),
    };
    let mut session = ObswsSession::new(Some(auth), input_registry(), None, program_output());
    let action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"authentication":"invalid"}}"#)
        .await
        .expect("identify must be parsed");
    let (code, reason) = unwrap_close(action);
    assert_eq!(code, OBSWS_CLOSE_AUTHENTICATION_FAILED);
    assert_eq!(reason, "authentication failed");
}

#[tokio::test]
async fn stop_record_when_inactive_returns_error_response() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let input_registry = Arc::new(RwLock::new(ObswsInputRegistry::new(
        temp_dir.path().to_path_buf(),
        crate::types::EvenUsize::new(1920).unwrap(),
        crate::types::EvenUsize::new(1080).unwrap(),
        crate::video::FrameRate::FPS_30,
    )));
    {
        let mut registry = input_registry.write().await;
        let input = ObswsInput::from_kind_and_settings(
            "mp4_file_source",
            nojson::RawJsonOwned::parse(
                r#"{"path":"testdata/beep-aac-audio.mp4","loopPlayback":true}"#,
            )
            .expect("requestData must be valid json")
            .value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input("Scene", "audio-file-1", input, true)
            .expect("input creation must succeed");
    }

    let pipeline = crate::MediaPipeline::new()?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let mut session = ObswsSession::new(
        None,
        input_registry.clone(),
        Some(pipeline_handle),
        program_output(),
    );
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
    let input_registry = Arc::new(RwLock::new(ObswsInputRegistry::new(
        temp_dir.path().to_path_buf(),
        crate::types::EvenUsize::new(1920).unwrap(),
        crate::types::EvenUsize::new(1080).unwrap(),
        crate::video::FrameRate::FPS_30,
    )));
    {
        let mut registry = input_registry.write().await;
        let input = ObswsInput::from_kind_and_settings(
            "mp4_file_source",
            nojson::RawJsonOwned::parse(
                r#"{"path":"testdata/beep-aac-audio.mp4","loopPlayback":true}"#,
            )
            .expect("requestData must be valid json")
            .value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input("Scene", "audio-file-immediate-stop", input, true)
            .expect("input creation must succeed");
    }

    let pipeline = crate::MediaPipeline::new()?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let mut session = ObswsSession::new(
        None,
        input_registry,
        Some(pipeline_handle),
        program_output(),
    );
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
    let input_registry = Arc::new(RwLock::new(ObswsInputRegistry::new(
        temp_dir.path().to_path_buf(),
        crate::types::EvenUsize::new(1920).unwrap(),
        crate::types::EvenUsize::new(1080).unwrap(),
        crate::video::FrameRate::FPS_30,
    )));
    {
        let mut registry = input_registry.write().await;
        for input_name in ["audio-file-1", "audio-file-2"] {
            let input = ObswsInput::from_kind_and_settings(
                "mp4_file_source",
                nojson::RawJsonOwned::parse(
                    r#"{"path":"testdata/beep-aac-audio.mp4","loopPlayback":true}"#,
                )
                .expect("requestData must be valid json")
                .value(),
            )
            .expect("input settings must be valid");
            registry
                .create_input("Scene", input_name, input, true)
                .expect("input creation must succeed");
        }
    }

    let pipeline = crate::MediaPipeline::new()?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let mut session = ObswsSession::new(
        None,
        input_registry.clone(),
        Some(pipeline_handle.clone()),
        program_output(),
    );
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

    let record_run = input_registry
        .read()
        .await
        .record_run()
        .expect("active record must have run state");
    assert_eq!(
        record_run.audio_mixer_processor_id.get(),
        "obsws:record:0:audio_mixer"
    );

    let mut found_audio_mixer = false;
    for _ in 0..20 {
        let live_processors = pipeline_handle
            .list_processors()
            .await
            .map_err(|_| crate::Error::new("failed to list processors: pipeline has terminated"))?;
        if live_processors
            .iter()
            .any(|id| id.get() == "obsws:record:0:audio_mixer")
        {
            found_audio_mixer = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(found_audio_mixer);

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
    let input_registry = Arc::new(RwLock::new(ObswsInputRegistry::new(
        temp_dir.path().to_path_buf(),
        crate::types::EvenUsize::new(1920).unwrap(),
        crate::types::EvenUsize::new(1080).unwrap(),
        crate::video::FrameRate::FPS_30,
    )));

    let pipeline = crate::MediaPipeline::new()?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let mut session = ObswsSession::new(
        None,
        input_registry.clone(),
        Some(pipeline_handle.clone()),
        program_output(),
    );
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

    let record_run = input_registry
        .read()
        .await
        .record_run()
        .expect("active record must have run state");
    assert_eq!(
        record_run.audio_mixer_processor_id.get(),
        "obsws:record:0:audio_mixer"
    );

    let mut found_audio_mixer = false;
    for _ in 0..20 {
        let live_processors = pipeline_handle
            .list_processors()
            .await
            .map_err(|_| crate::Error::new("failed to list processors: pipeline has terminated"))?;
        if live_processors
            .iter()
            .any(|id| id.get() == "obsws:record:0:audio_mixer")
        {
            found_audio_mixer = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(found_audio_mixer);

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
    let input_registry = Arc::new(RwLock::new(ObswsInputRegistry::new_for_test()));
    {
        let mut registry = input_registry.write().await;
        registry.set_stream_service_settings(ObswsStreamServiceSettings {
            stream_service_type: "rtmp_custom".to_owned(),
            server: Some("rtmp://127.0.0.1:1935/live".to_owned()),
            key: Some("stream-no-inputs".to_owned()),
        });
    }

    let pipeline = crate::MediaPipeline::new()?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let mut session = ObswsSession::new(
        None,
        input_registry.clone(),
        Some(pipeline_handle.clone()),
        program_output(),
    );
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

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

    let stream_run = input_registry
        .read()
        .await
        .stream_run()
        .expect("active stream must have run state");
    assert_eq!(
        stream_run.audio_mixer_processor_id.get(),
        "obsws:stream:0:audio_mixer"
    );

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
    let input_registry = Arc::new(RwLock::new(ObswsInputRegistry::new_for_test()));
    {
        let mut registry = input_registry.write().await;
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
    }

    let mut session = ObswsSession::new(None, input_registry, None, program_output());
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
    // プラン構築は成功するが、パイプラインがないため実行時エラーになる
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_REQUEST_PROCESSING_FAILED);
}

#[tokio::test]
async fn start_stream_with_multiple_audio_inputs_uses_audio_mixer() -> crate::Result<()> {
    let input_registry = Arc::new(RwLock::new(ObswsInputRegistry::new_for_test()));
    {
        let mut registry = input_registry.write().await;
        registry.set_stream_service_settings(ObswsStreamServiceSettings {
            stream_service_type: "rtmp_custom".to_owned(),
            server: Some("rtmp://127.0.0.1:1935/live".to_owned()),
            key: Some("stream-main".to_owned()),
        });
        for input_name in ["audio-file-1", "audio-file-2"] {
            let input = ObswsInput::from_kind_and_settings(
                "mp4_file_source",
                nojson::RawJsonOwned::parse(
                    r#"{"path":"testdata/beep-aac-audio.mp4","loopPlayback":true}"#,
                )
                .expect("requestData must be valid json")
                .value(),
            )
            .expect("input settings must be valid");
            registry
                .create_input("Scene", input_name, input, true)
                .expect("input creation must succeed");
        }
    }

    let pipeline = crate::MediaPipeline::new()?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let mut session = ObswsSession::new(
        None,
        input_registry.clone(),
        Some(pipeline_handle.clone()),
        program_output(),
    );
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

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

    let stream_run = input_registry
        .read()
        .await
        .stream_run()
        .expect("active stream must have run state");
    assert_eq!(
        stream_run.audio_mixer_processor_id.get(),
        "obsws:stream:0:audio_mixer"
    );

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
async fn start_stream_with_multiple_video_inputs_builds_plan_successfully() {
    // 複数映像入力は受理されるが、パイプラインがないため実行時エラーになる
    let input_registry = Arc::new(RwLock::new(ObswsInputRegistry::new_for_test()));
    {
        let mut registry = input_registry.write().await;
        registry.set_stream_service_settings(ObswsStreamServiceSettings {
            stream_service_type: "rtmp_custom".to_owned(),
            server: Some("rtmp://127.0.0.1:1935/live".to_owned()),
            key: Some("stream-main".to_owned()),
        });
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
    }

    let mut session = ObswsSession::new(None, input_registry, None, program_output());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-stream-multiple-video".to_owned()),
            request_type: Some("StartStream".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    // プラン構築は成功するが、パイプラインがないため実行時エラーになる
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_REQUEST_PROCESSING_FAILED);
}

#[tokio::test]
async fn toggle_stream_without_image_input_returns_toggle_request_type_error() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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

#[tokio::test]
async fn request_batch_with_halt_on_failure_stops_after_first_failure() {
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
    let mut session = ObswsSession::new(None, input_registry(), None, program_output());
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
