//! Output (stream) 系のテスト (StartStream / StopStream)。
//!
//! 旧 `src/obsws/session/tests.rs` の line 1916-2392 周辺から物理移動した 3 件を集約する。
//! 元ファイルでは record 系と stream 系のテストが行番号順に交錯しており、
//! ここには stream 系 (`start_stream_*`) のみを集める。

use crate::obsws::message::RequestMessage;
use crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED;
use crate::obsws::session::{ObswsSession, SessionAction};
use crate::obsws::state::{ObswsInput, ObswsSessionState};

use super::common::*;

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
