//! Lifecycle 系のテスト (identify / sleep / broadcast / authentication / RPC version)。
//!
//! 旧 `src/obsws/session/tests.rs` の line 441-656 / 1559-1592 から物理移動した 14 件を集約する。

use crate::obsws::auth::{ObswsAuthentication, build_authentication_response};
use crate::obsws::message::RequestMessage;
use crate::obsws::protocol::{
    OBSWS_CLOSE_ALREADY_IDENTIFIED, OBSWS_CLOSE_AUTHENTICATION_FAILED, OBSWS_CLOSE_NOT_IDENTIFIED,
    OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION, OBSWS_EVENT_SUB_ALL, OBSWS_EVENT_SUB_GENERAL,
    OBSWS_EVENT_SUB_OUTPUTS, REQUEST_STATUS_INVALID_REQUEST_FIELD,
};
use crate::obsws::session::{ObswsSession, SessionAction};

use super::common::*;

#[tokio::test]
async fn on_connected_returns_hello_message_action() {
    let session = ObswsSession::new(None, default_coordinator_handle());
    let action = session.on_connected();
    let SessionAction::SendText { text, message_name } = action else {
        panic!("must be SendText");
    };
    assert_eq!(message_name, "hello message");
    assert!(text.text().contains("\"op\":0"));
}

#[tokio::test]
async fn on_request_before_identify_returns_close_action() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(action, SessionAction::SendText { .. }));
    assert_eq!(session.event_subscriptions, OBSWS_EVENT_SUB_ALL);
}

#[tokio::test]
async fn identify_with_event_subscriptions_updates_session_state() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":64}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(action, SessionAction::SendText { .. }));
    assert_eq!(session.event_subscriptions, OBSWS_EVENT_SUB_OUTPUTS);
}

#[tokio::test]
async fn reidentify_updates_event_subscriptions_when_specified() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    // eventSubscriptions を指定しない場合はデフォルトの OBSWS_EVENT_SUB_ALL になる
    assert_eq!(session.event_subscriptions, OBSWS_EVENT_SUB_ALL);
}

#[tokio::test]
async fn unsupported_rpc_version_returns_close_action() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(Some(auth), default_coordinator_handle());
    let action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"authentication":"invalid"}}"#)
        .await
        .expect("identify must be parsed");
    let (code, reason) = unwrap_close(action);
    assert_eq!(code, OBSWS_CLOSE_AUTHENTICATION_FAILED);
    assert_eq!(reason, "authentication failed");
}
