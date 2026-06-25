//! テスト共通ヘルパー。
//!
//! 旧 `src/obsws/session/tests.rs` の line 17-185 / 255-389 / 392-438 から物理移動した汎用ヘルパー 23 件を集約する。
//! 関数本体・引数・戻り値型・属性 (`#[cfg(...)]`) は一切変更せず、可視性のみ `pub(super)` 化している。

use shiguredo_websocket::CloseCode;

use crate::obsws::message::RequestMessage;
use crate::obsws::session::{ObswsSession, SessionAction};
use crate::obsws::state::ObswsSessionState;
use std::time::Duration;

/// テスト用の ProgramOutputState を生成する
pub(super) fn test_program_output() -> crate::obsws::server::ProgramOutputState {
    crate::obsws::server::ProgramOutputState {
        scene_uuid: "scene-default".to_owned(),
        video_track_id: crate::TrackId::new("program:mixed_video"),
        audio_track_id: crate::TrackId::new("program:mixed_audio"),
        video_mixer_processor_id: crate::ProcessorId::new("program:video_mixer"),
        audio_mixer_processor_id: crate::ProcessorId::new("program:audio_mixer"),
        source_processor_ids: Vec::new(),
    }
}

/// レジストリからランタイムハンドルを生成し、actor を spawn する
pub(super) fn create_coordinator_handle(
    registry: ObswsSessionState,
) -> crate::obsws::coordinator::ObswsCoordinatorHandle {
    let program_output = test_program_output();
    let (actor, handle, _shutdown_rx) = crate::obsws::coordinator::ObswsCoordinator::new(
        registry,
        std::path::PathBuf::from("recordings-for-test"),
        program_output,
        None,
        #[cfg(feature = "player")]
        test_player_command_tx(),
        #[cfg(feature = "player")]
        test_player_media_tx(),
    );
    tokio::spawn(actor.run());
    handle
}

#[cfg(feature = "player")]
pub(super) fn create_coordinator_handle_with_player_channels(
    registry: ObswsSessionState,
    pipeline_handle: Option<crate::MediaPipelineHandle>,
    player_command_tx: std::sync::mpsc::SyncSender<crate::obsws::player::PlayerCommand>,
    player_media_tx: std::sync::mpsc::SyncSender<crate::obsws::player::PlayerMediaMessage>,
    player_lifecycle_rx: tokio::sync::mpsc::UnboundedReceiver<
        crate::obsws::player::PlayerLifecycleEvent,
    >,
) -> crate::obsws::coordinator::ObswsCoordinatorHandle {
    let program_output = test_program_output();
    let (actor, handle, _shutdown_rx) = crate::obsws::coordinator::ObswsCoordinator::new(
        registry,
        std::path::PathBuf::from("recordings-for-test"),
        program_output,
        pipeline_handle,
        player_command_tx,
        player_media_tx,
    );
    let forward_handle = handle.clone();
    tokio::spawn(async move {
        let mut player_lifecycle_rx = player_lifecycle_rx;
        while let Some(event) = player_lifecycle_rx.recv().await {
            forward_handle.notify_player_lifecycle_event(event);
        }
    });
    tokio::spawn(actor.run());
    handle
}

/// デフォルトのテスト用ランタイムハンドルを生成する
pub(super) fn default_coordinator_handle() -> crate::obsws::coordinator::ObswsCoordinatorHandle {
    create_coordinator_handle(ObswsSessionState::new_for_test())
}

#[cfg(feature = "player")]
pub(super) fn test_player_command_tx()
-> std::sync::mpsc::SyncSender<crate::obsws::player::PlayerCommand> {
    std::sync::mpsc::sync_channel(1).0
}

#[cfg(feature = "player")]
pub(super) fn test_player_media_tx()
-> std::sync::mpsc::SyncSender<crate::obsws::player::PlayerMediaMessage> {
    std::sync::mpsc::sync_channel(1).0
}

/// パイプライン付きのランタイムハンドルを生成する
pub(super) fn create_coordinator_handle_with_pipeline(
    registry: ObswsSessionState,
    pipeline_handle: crate::MediaPipelineHandle,
) -> crate::obsws::coordinator::ObswsCoordinatorHandle {
    create_coordinator_handle_with_pipeline_and_record_dir(
        registry,
        pipeline_handle,
        std::path::PathBuf::from("recordings-for-test"),
    )
}

/// パイプライン付きのランタイムハンドルを生成する（録画ディレクトリ指定）
pub(super) fn create_coordinator_handle_with_pipeline_and_record_dir(
    registry: ObswsSessionState,
    pipeline_handle: crate::MediaPipelineHandle,
    record_directory: std::path::PathBuf,
) -> crate::obsws::coordinator::ObswsCoordinatorHandle {
    let program_output = test_program_output();
    let (actor, handle, _shutdown_rx) = crate::obsws::coordinator::ObswsCoordinator::new(
        registry,
        record_directory,
        program_output,
        Some(pipeline_handle),
        #[cfg(feature = "player")]
        test_player_command_tx(),
        #[cfg(feature = "player")]
        test_player_media_tx(),
    );
    tokio::spawn(actor.run());
    handle
}

pub(super) async fn create_initialized_coordinator_handle_with_pipeline(
    registry: ObswsSessionState,
    pipeline_handle: crate::MediaPipelineHandle,
) -> crate::Result<crate::obsws::coordinator::ObswsCoordinatorHandle> {
    create_initialized_coordinator_handle_with_pipeline_and_record_dir(
        registry,
        pipeline_handle,
        std::path::PathBuf::from("recordings-for-test"),
    )
    .await
}

pub(super) async fn create_initialized_coordinator_handle_with_pipeline_and_record_dir(
    registry: ObswsSessionState,
    pipeline_handle: crate::MediaPipelineHandle,
    record_directory: std::path::PathBuf,
) -> crate::Result<crate::obsws::coordinator::ObswsCoordinatorHandle> {
    let scene_inputs = registry.list_current_program_scene_input_entries();
    let output_plan = crate::obsws::output_plan::build_composed_output_plan(
        &scene_inputs,
        registry.canvas_width(),
        registry.canvas_height(),
        registry.frame_rate(),
    )
    .map_err(|e| {
        crate::Error::new(format!(
            "failed to build program output plan: {}",
            e.message()
        ))
    })?;

    crate::obsws::session::output::start_mixer_processors(&pipeline_handle, &output_plan, None)
        .await?;

    let scene_uuid = registry
        .current_program_scene()
        .map(|scene| scene.scene_uuid)
        .unwrap_or_default();
    let program_output = crate::obsws::server::ProgramOutputState {
        scene_uuid,
        video_track_id: output_plan.video_track_id,
        audio_track_id: output_plan.audio_track_id,
        video_mixer_processor_id: output_plan.video_mixer_processor_id,
        audio_mixer_processor_id: output_plan.audio_mixer_processor_id,
        source_processor_ids: output_plan.source_processor_ids,
    };

    let (mut actor, handle, _shutdown_rx) = crate::obsws::coordinator::ObswsCoordinator::new(
        registry,
        record_directory,
        program_output,
        Some(pipeline_handle),
        #[cfg(feature = "player")]
        test_player_command_tx(),
        #[cfg(feature = "player")]
        test_player_media_tx(),
    );
    actor.start_initial_input_source_processors().await?;
    tokio::spawn(actor.run());
    Ok(handle)
}

pub(super) fn parse_request_status(text: &nojson::RawJsonOwned) -> (bool, i64) {
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

pub(super) fn parse_request_type(text: &nojson::RawJsonOwned) -> String {
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid json");
    json.value()
        .to_path_member(&["d", "requestType"])
        .and_then(|v| v.required()?.try_into())
        .expect("requestType must be string")
}

pub(super) fn parse_output_active(text: &nojson::RawJsonOwned) -> bool {
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid json");
    json.value()
        .to_path_member(&["d", "responseData", "outputActive"])
        .and_then(|v| v.required()?.try_into())
        .expect("outputActive must be bool")
}

pub(super) fn parse_response_scene_item_id(text: &nojson::RawJsonOwned) -> i64 {
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid json");
    json.value()
        .to_path_member(&["d", "responseData", "sceneItemId"])
        .and_then(|v| v.required()?.try_into())
        .expect("sceneItemId must be i64")
}

pub(super) fn parse_identified_message(text: &nojson::RawJsonOwned) -> (i64, u32) {
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

pub(super) fn parse_event_type_and_intent(text: &nojson::RawJsonOwned) -> (i64, String, u32) {
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

pub(super) fn parse_request_batch_results(text: &nojson::RawJsonOwned) -> Vec<(String, bool, i64)> {
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
pub(super) fn unwrap_send_text(action: SessionAction) -> nojson::RawJsonOwned {
    let SessionAction::SendText { text, .. } = action else {
        panic!("expected SendText");
    };
    text
}

/// SessionAction::SendTexts から messages を取り出す。SendTexts でなければパニック。
pub(super) fn unwrap_send_texts(
    action: SessionAction,
) -> Vec<(nojson::RawJsonOwned, &'static str)> {
    let SessionAction::SendTexts { messages } = action else {
        panic!("expected SendTexts");
    };
    messages
}

/// SessionAction::Close から code と reason を取り出す。Close でなければパニック。
pub(super) fn unwrap_close(action: SessionAction) -> (CloseCode, &'static str) {
    let SessionAction::Close { code, reason, .. } = action else {
        panic!("expected Close");
    };
    (code, reason)
}

pub(super) async fn identify_session(session: &mut ObswsSession) {
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));
}

/// HisuiCreateOutput で output を作成するヘルパー
pub(super) async fn create_output(
    session: &mut ObswsSession,
    output_name: &str,
    output_kind: &str,
) {
    let action = session
        .handle_request(RequestMessage {
            request_id: Some(format!("req-create-{output_name}")),
            request_type: Some("HisuiCreateOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(format!(
                    r#"{{"outputName":"{output_name}","outputKind":"{output_kind}"}}"#
                ))
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(action);
    let (result, _) = parse_request_status(&text);
    assert!(result, "HisuiCreateOutput for {output_name} must succeed");
}

pub(super) async fn wait_for_processor_presence(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_id: &str,
    expected: bool,
) -> crate::Result<()> {
    for _ in 0..20 {
        let live_processors = pipeline_handle
            .list_processors()
            .await
            .map_err(|_| crate::Error::new("failed to list processors: pipeline has terminated"))?;
        let found = live_processors.iter().any(|id| id.get() == processor_id);
        if found == expected {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(crate::Error::new(format!(
        "processor presence did not converge: {processor_id} expected={expected}"
    )))
}
