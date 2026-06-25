//! Output (create / settings) 系のテスト (HisuiCreateOutput / SetOutputSettings / SetRecordDirectory)。

use crate::obsws::message::RequestMessage;
use crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD;
use crate::obsws::session::ObswsSession;
use crate::obsws::state::ObswsSessionState;

use super::common::*;

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
