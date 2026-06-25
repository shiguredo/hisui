//! PersistentData 系のテスト (SetPersistentData / GetPersistentData)。
//!
//! 旧 `src/obsws/session/tests.rs` の line 2915-3055 から物理移動した 4 件を集約する。

use crate::obsws::message::RequestMessage;
use crate::obsws::protocol::{
    REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_FIELD,
};
use crate::obsws::session::ObswsSession;

use super::common::*;

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
