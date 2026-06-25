//! Output (record) 系のテスト (StartRecord / StopRecord)。
//!
//! 旧 `src/obsws/session/tests.rs` の line 1593-1990 周辺から物理移動した 6 件を集約する。
//! 元ファイルでは record 系と stream 系のテストが行番号順に交錯しており、
//! ここには record 系 (`start_record_*` / `stop_record_*`) のみを集める。

use crate::obsws::message::RequestMessage;
use crate::obsws::protocol::{
    REQUEST_STATUS_OUTPUT_NOT_RUNNING, REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
};
use crate::obsws::session::{ObswsSession, SessionAction};
use crate::obsws::state::{ObswsInput, ObswsSessionState};
use std::time::Duration;

use super::common::*;

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
