//! Input 系のテスト (Input の作成 / 削除 / 設定変更 / 名前変更)。

use crate::obsws::message::RequestMessage;
use crate::obsws::protocol::{
    OBSWS_EVENT_SUB_INPUTS, REQUEST_STATUS_INVALID_REQUEST_FIELD,
    REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
};
use crate::obsws::session::{ObswsSession, SessionAction};

use super::common::*;

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
