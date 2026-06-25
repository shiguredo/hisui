//! Output (player) 系のテスト。
//!
//! 旧 `src/obsws/session/tests.rs` の line 2550-2862 から物理移動した 4 件を集約する。
//! ファイル全体は `tests.rs` の `mod output_player;` 宣言に付いた `#[cfg(feature = "player")]` によりゲートされる。
//! 各関数定義に残っている `#[cfg(feature = "player")]` は冗長だが、関数単位での見落とし防止のため残す。

use crate::obsws::message::RequestMessage;
use crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED;
use crate::obsws::session::{ObswsSession, SessionAction};
use crate::obsws::state::ObswsSessionState;

use super::common::*;

#[cfg(feature = "player")]
#[tokio::test]
async fn start_output_player_with_closed_control_channel_returns_processing_failed() {
    let registry = ObswsSessionState::new_for_test();
    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())
        .expect("failed to create test media pipeline");
    let pipeline_handle = pipeline.handle();
    let (player_command_tx, player_command_rx) = std::sync::mpsc::sync_channel(1);
    drop(player_command_rx);
    let player_media_tx = std::sync::mpsc::sync_channel(1).0;
    let player_lifecycle_rx = tokio::sync::mpsc::unbounded_channel().1;
    let handle = create_coordinator_handle_with_player_channels(
        registry,
        Some(pipeline_handle),
        player_command_tx,
        player_media_tx,
        player_lifecycle_rx,
    );
    let mut session = ObswsSession::new(None, handle);
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-player".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, code) = parse_request_status(&text);
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_REQUEST_PROCESSING_FAILED);
    assert_eq!(parse_request_type(&text), "StartOutput");
}

#[cfg(feature = "player")]
#[tokio::test]
async fn player_lifecycle_stop_updates_output_status() {
    let registry = ObswsSessionState::new_for_test();
    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())
        .expect("failed to create test media pipeline");
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let (player_command_tx, player_command_rx) = std::sync::mpsc::sync_channel(4);
    let player_media_tx = std::sync::mpsc::sync_channel(1).0;
    let (player_lifecycle_tx, player_lifecycle_rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = create_coordinator_handle_with_player_channels(
        registry,
        Some(pipeline_handle),
        player_command_tx,
        player_media_tx,
        player_lifecycle_rx,
    );
    let command_thread = std::thread::spawn(move || {
        let command = player_command_rx
            .recv()
            .expect("player command must be sent");
        let crate::obsws::player::PlayerCommand::Start { reply_tx, .. } = command else {
            panic!("unexpected player command");
        };
        let _ = reply_tx.send(Ok(()));
    });

    let mut session = ObswsSession::new(None, handle);
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-player".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let start_text = unwrap_send_text(start_action);
    let (start_result, _start_code) = parse_request_status(&start_text);
    assert!(start_result);

    let get_active_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-player-active".to_owned()),
            request_type: Some("GetOutputStatus".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let get_active_text = unwrap_send_text(get_active_action);
    assert!(parse_output_active(&get_active_text));

    player_lifecycle_tx
        .send(crate::obsws::player::PlayerLifecycleEvent::Stopped { generation: 1 })
        .expect("player lifecycle event must be sent");
    tokio::task::yield_now().await;

    let get_inactive_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-player-inactive".to_owned()),
            request_type: Some("GetOutputStatus".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let get_inactive_text = unwrap_send_text(get_inactive_action);
    assert!(!parse_output_active(&get_inactive_text));

    command_thread
        .join()
        .expect("player command thread must not panic");
    pipeline_task.abort();
}

#[cfg(feature = "player")]
#[tokio::test]
async fn start_output_player_returns_processing_failed_when_subscriber_startup_fails() {
    let registry = ObswsSessionState::new_for_test();
    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())
        .expect("failed to create test media pipeline");
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let _existing_processor = pipeline_handle
        .register_processor(
            crate::ProcessorId::new("player"),
            crate::ProcessorMetadata::new("player"),
        )
        .await
        .expect("player processor must be registered");
    let (player_command_tx, player_command_rx) = std::sync::mpsc::sync_channel(4);
    let player_media_tx = std::sync::mpsc::sync_channel(1).0;
    let player_lifecycle_rx = tokio::sync::mpsc::unbounded_channel().1;
    let handle = create_coordinator_handle_with_player_channels(
        registry,
        Some(pipeline_handle),
        player_command_tx,
        player_media_tx,
        player_lifecycle_rx,
    );
    let command_thread = std::thread::spawn(move || {
        while let Ok(command) = player_command_rx.recv() {
            match command {
                crate::obsws::player::PlayerCommand::Start { reply_tx, .. } => {
                    let _ = reply_tx.send(Ok(()));
                }
                crate::obsws::player::PlayerCommand::Stop => break,
                crate::obsws::player::PlayerCommand::Terminate => break,
            }
        }
    });

    let mut session = ObswsSession::new(None, handle);
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-player-duplicate".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let start_text = unwrap_send_text(start_action);
    let (start_result, start_code) = parse_request_status(&start_text);
    assert!(!start_result);
    assert_eq!(start_code, REQUEST_STATUS_REQUEST_PROCESSING_FAILED);

    let status_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-player-after-dup".to_owned()),
            request_type: Some("GetOutputStatus".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let status_text = unwrap_send_text(status_action);
    assert!(!parse_output_active(&status_text));

    command_thread
        .join()
        .expect("player command thread must not panic");
    pipeline_task.abort();
}

#[cfg(feature = "player")]
#[tokio::test]
async fn stale_player_stopped_event_does_not_deactivate_restarted_player() {
    let registry = ObswsSessionState::new_for_test();
    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())
        .expect("failed to create test media pipeline");
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let (player_command_tx, player_command_rx) = std::sync::mpsc::sync_channel(8);
    let player_media_tx = std::sync::mpsc::sync_channel(1).0;
    let (player_lifecycle_tx, player_lifecycle_rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = create_coordinator_handle_with_player_channels(
        registry,
        Some(pipeline_handle),
        player_command_tx,
        player_media_tx,
        player_lifecycle_rx,
    );
    let command_thread = std::thread::spawn(move || {
        let mut start_count = 0;
        while let Ok(command) = player_command_rx.recv() {
            match command {
                crate::obsws::player::PlayerCommand::Start { reply_tx, .. } => {
                    start_count += 1;
                    let _ = reply_tx.send(Ok(()));
                    if start_count == 2 {
                        break;
                    }
                }
                crate::obsws::player::PlayerCommand::Stop => {}
                crate::obsws::player::PlayerCommand::Terminate => break,
            }
        }
    });

    let mut session = ObswsSession::new(None, handle);
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let first_start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-player-first".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let first_start_text = unwrap_send_text(first_start_action);
    let (first_start_result, _) = parse_request_status(&first_start_text);
    assert!(first_start_result);

    let stop_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-stop-player".to_owned()),
            request_type: Some("StopOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let stop_text = unwrap_send_text(stop_action);
    let (stop_result, _) = parse_request_status(&stop_text);
    assert!(stop_result);

    let second_start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-player-second".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let second_start_text = unwrap_send_text(second_start_action);
    let (second_start_result, _) = parse_request_status(&second_start_text);
    assert!(second_start_result);

    player_lifecycle_tx
        .send(crate::obsws::player::PlayerLifecycleEvent::Stopped { generation: 1 })
        .expect("stale player lifecycle event must be sent");
    tokio::task::yield_now().await;

    let status_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-player-after-stale".to_owned()),
            request_type: Some("GetOutputStatus".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"player"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let status_text = unwrap_send_text(status_action);
    assert!(parse_output_active(&status_text));

    command_thread
        .join()
        .expect("player command thread must not panic");
    pipeline_task.abort();
}
