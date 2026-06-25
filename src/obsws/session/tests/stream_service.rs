//! StreamServiceSettings 系のテスト (handle_get_stream_service_settings)。
//!
//! 旧 `src/obsws/session/tests.rs` の line 4608-4739 から物理移動した 2 件を集約する。

use crate::obsws::message::RequestMessage;
use crate::obsws::session::ObswsSession;
use crate::obsws::state::ObswsSessionState;

use super::common::*;

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
