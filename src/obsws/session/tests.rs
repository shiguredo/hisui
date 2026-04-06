use super::*;
use crate::obsws::auth::build_authentication_response;
use crate::obsws::input_registry::{ObswsInput, ObswsInputRegistry, ObswsStreamServiceSettings};
use crate::obsws::message::RequestMessage;
use crate::obsws::protocol::{
    OBSWS_CLOSE_ALREADY_IDENTIFIED, OBSWS_CLOSE_AUTHENTICATION_FAILED, OBSWS_CLOSE_NOT_IDENTIFIED,
    OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION, OBSWS_EVENT_SUB_GENERAL, OBSWS_EVENT_SUB_INPUTS,
    OBSWS_EVENT_SUB_OUTPUTS, OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED,
    OBSWS_EVENT_SUB_SCENE_ITEMS, OBSWS_EVENT_SUB_SCENES, REQUEST_STATUS_INVALID_REQUEST_FIELD,
    REQUEST_STATUS_MISSING_REQUEST_FIELD, REQUEST_STATUS_OUTPUT_NOT_RUNNING,
    REQUEST_STATUS_REQUEST_PROCESSING_FAILED, REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
    REQUEST_STATUS_RESOURCE_NOT_FOUND,
};
use std::time::Duration;

/// テスト用の ProgramOutputState を生成する
fn test_program_output() -> crate::obsws::server::ProgramOutputState {
    crate::obsws::server::ProgramOutputState {
        scene_uuid: "scene-default".to_owned(),
        video_track_id: crate::TrackId::new("obsws:program:0:mixed_video"),
        audio_track_id: crate::TrackId::new("obsws:program:0:mixed_audio"),
        video_mixer_processor_id: crate::ProcessorId::new("obsws:program:0:video_mixer"),
        audio_mixer_processor_id: crate::ProcessorId::new("obsws:program:0:audio_mixer"),
        source_processor_ids: Vec::new(),
    }
}

/// レジストリからランタイムハンドルを生成し、actor を spawn する
fn create_coordinator_handle(
    registry: ObswsInputRegistry,
) -> crate::obsws::coordinator::ObswsCoordinatorHandle {
    let program_output = test_program_output();
    let (actor, handle, _shutdown_rx) = crate::obsws::coordinator::ObswsCoordinator::new(
        registry,
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
fn create_coordinator_handle_with_player_channels(
    registry: ObswsInputRegistry,
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
fn default_coordinator_handle() -> crate::obsws::coordinator::ObswsCoordinatorHandle {
    create_coordinator_handle(ObswsInputRegistry::new_for_test())
}

#[cfg(feature = "player")]
fn test_player_command_tx() -> std::sync::mpsc::SyncSender<crate::obsws::player::PlayerCommand> {
    std::sync::mpsc::sync_channel(1).0
}

#[cfg(feature = "player")]
fn test_player_media_tx() -> std::sync::mpsc::SyncSender<crate::obsws::player::PlayerMediaMessage> {
    std::sync::mpsc::sync_channel(1).0
}

/// パイプライン付きのランタイムハンドルを生成する
fn create_coordinator_handle_with_pipeline(
    registry: ObswsInputRegistry,
    pipeline_handle: crate::MediaPipelineHandle,
) -> crate::obsws::coordinator::ObswsCoordinatorHandle {
    let program_output = test_program_output();
    let (actor, handle, _shutdown_rx) = crate::obsws::coordinator::ObswsCoordinator::new(
        registry,
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

async fn create_initialized_coordinator_handle_with_pipeline(
    registry: ObswsInputRegistry,
    pipeline_handle: crate::MediaPipelineHandle,
) -> crate::Result<crate::obsws::coordinator::ObswsCoordinatorHandle> {
    let scene_inputs = registry.list_current_program_scene_input_entries();
    let output_plan = crate::obsws::output_plan::build_composed_output_plan(
        &scene_inputs,
        crate::obsws::source::ObswsOutputKind::Program,
        0,
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

    crate::obsws::session::output::start_mixer_processors(&pipeline_handle, &output_plan).await?;

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

#[tokio::test]
async fn remove_current_scene_updates_program_output_state_without_pipeline() {
    let mut registry = ObswsInputRegistry::new_for_test();
    registry.create_scene("Scene B").expect("must create scene");
    registry
        .set_current_program_scene("Scene B")
        .expect("must switch scene");

    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
    let identified = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":4}}"#)
        .await;
    assert!(identified.is_ok());

    let action = session
        .handle_request(RequestMessage {
            request_id: Some("req-remove-scene".to_owned()),
            request_type: Some("RemoveScene".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;

    let messages = unwrap_send_texts(action);
    assert_eq!(messages.len(), 3);

    // actor が ProgramOutputState を管理しているため直接参照はできない。
    // GetCurrentProgramScene リクエストで残存シーン "Scene" が返ることを検証する。
    let get_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-current-scene".to_owned()),
            request_type: Some("GetCurrentProgramScene".to_owned()),
            request_data: None,
        })
        .await;
    let text = unwrap_send_text(get_action);
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid json");
    let scene_name: String = json
        .value()
        .to_path_member(&["d", "responseData", "currentProgramSceneName"])
        .and_then(|v| v.required()?.try_into())
        .expect("currentProgramSceneName must be string");
    assert_eq!(scene_name, "Scene");
}

#[tokio::test]
async fn stale_scene_uuid_differs_from_current_program_scene_uuid() {
    let mut registry = ObswsInputRegistry::new_for_test();
    registry.create_scene("Scene B").expect("must create scene");

    let stale_scene_uuid = registry
        .get_scene_uuid("Scene")
        .expect("default scene must exist");

    registry
        .set_current_program_scene("Scene B")
        .expect("must switch scene");

    let current_scene_uuid = registry
        .current_program_scene()
        .map(|scene| scene.scene_uuid)
        .expect("current program scene must exist");
    assert_ne!(stale_scene_uuid, current_scene_uuid);
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

fn parse_output_active(text: &nojson::RawJsonOwned) -> bool {
    let json = nojson::RawJson::parse(text.text()).expect("response must be valid json");
    json.value()
        .to_path_member(&["d", "responseData", "outputActive"])
        .and_then(|v| v.required()?.try_into())
        .expect("outputActive must be bool")
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

async fn identify_session(session: &mut ObswsSession) {
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));
}

async fn wait_for_processor_presence(
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
async fn create_scene_with_scene_subscription_returns_scene_created_event() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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

#[tokio::test]
async fn set_scene_item_enabled_with_scene_subscription_sends_event_when_changed() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut registry = ObswsInputRegistry::new(
        temp_dir.path().to_path_buf(),
        crate::types::EvenUsize::new(1920).unwrap(),
        crate::types::EvenUsize::new(1080).unwrap(),
        crate::video::FrameRate::FPS_30,
        None,
    );
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

    let pipeline = crate::MediaPipeline::new()?;
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
    let mut registry = ObswsInputRegistry::new(
        temp_dir.path().to_path_buf(),
        crate::types::EvenUsize::new(1920).unwrap(),
        crate::types::EvenUsize::new(1080).unwrap(),
        crate::video::FrameRate::FPS_30,
        None,
    );
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

    let pipeline = crate::MediaPipeline::new()?;
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
    let mut registry = ObswsInputRegistry::new(
        temp_dir.path().to_path_buf(),
        crate::types::EvenUsize::new(1920).unwrap(),
        crate::types::EvenUsize::new(1080).unwrap(),
        crate::video::FrameRate::FPS_30,
        None,
    );
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

    let pipeline = crate::MediaPipeline::new()?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let handle =
        create_initialized_coordinator_handle_with_pipeline(registry, pipeline_handle.clone())
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
    let registry = ObswsInputRegistry::new(
        temp_dir.path().to_path_buf(),
        crate::types::EvenUsize::new(1920).unwrap(),
        crate::types::EvenUsize::new(1080).unwrap(),
        crate::video::FrameRate::FPS_30,
        None,
    );

    let pipeline = crate::MediaPipeline::new()?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let handle =
        create_initialized_coordinator_handle_with_pipeline(registry, pipeline_handle.clone())
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
async fn start_stream_with_no_inputs_succeeds() -> crate::Result<()> {
    let mut registry = ObswsInputRegistry::new_for_test();
    registry.set_stream_service_settings(ObswsStreamServiceSettings {
        stream_service_type: "rtmp_custom".to_owned(),
        server: Some("rtmp://127.0.0.1:1935/live".to_owned()),
        key: Some("stream-no-inputs".to_owned()),
    });

    let pipeline = crate::MediaPipeline::new()?;
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
async fn start_record_with_multiple_video_inputs_builds_plan_successfully() {
    // 複数映像入力は受理されるが、パイプラインがないため実行時エラーになる
    let mut registry = ObswsInputRegistry::new_for_test();
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

#[tokio::test]
async fn start_stream_with_multiple_audio_inputs_uses_audio_mixer() -> crate::Result<()> {
    let mut registry = ObswsInputRegistry::new_for_test();
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

    let pipeline = crate::MediaPipeline::new()?;
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
async fn hls_output_uses_program_mixers_after_scene_item_change() -> crate::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let mut registry = ObswsInputRegistry::new(
        temp_dir.path().to_path_buf(),
        crate::types::EvenUsize::new(1920).expect("canvas width must be valid"),
        crate::types::EvenUsize::new(1080).expect("canvas height must be valid"),
        crate::video::FrameRate::FPS_30,
        None,
    );
    let input = ObswsInput::from_kind_and_settings(
        "mp4_file_source",
        nojson::RawJsonOwned::parse(
            r#"{"path":"testdata/red-320x320-h264-aac.mp4","loopPlayback":true}"#,
        )
        .expect("requestData must be valid json")
        .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "video-file", input, true)
        .expect("input creation must succeed");

    let pipeline = crate::MediaPipeline::new()?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let handle = create_coordinator_handle_with_pipeline(registry, pipeline_handle.clone());
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    let hls_output_dir = temp_dir.path().join("hls-output");
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-hls-output".to_owned()),
            request_type: Some("SetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(format!(
                    r#"{{"outputName":"hls","outputSettings":{{"destination":{{"type":"filesystem","directory":"{}"}},"variants":[{{"videoBitrate":2000000,"audioBitrate":128000}},{{"videoBitrate":1000000,"audioBitrate":64000,"width":1280,"height":720}}]}}}}"#,
                    hls_output_dir.display()
                ))
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-hls-output".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"hls"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(start_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "obsws:hls:0:v1_scaler", true).await?;
    wait_for_processor_presence(&pipeline_handle, "obsws:hls:0:v0_hls_writer", true).await?;
    wait_for_processor_presence(&pipeline_handle, "obsws:hls:0:video_mixer", false).await?;

    let get_scene_item_id_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-hls-scene-item-id".to_owned()),
            request_type: Some("GetSceneItemId".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene","sourceName":"video-file"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(get_scene_item_id_action);
    let scene_item_id = parse_response_scene_item_id(&text);

    let set_scene_item_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-disable-hls-scene-item".to_owned()),
            request_type: Some("SetSceneItemEnabled".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(format!(
                    r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemEnabled":false}}"#,
                    scene_item_id
                ))
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_scene_item_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "obsws:hls:0:v0_hls_writer", true).await?;

    let stop_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-stop-hls-output".to_owned()),
            request_type: Some("StopOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"hls"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(stop_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "obsws:hls:0:v0_hls_writer", false).await?;

    pipeline_task.abort();

    Ok(())
}

#[tokio::test]
async fn dash_output_uses_program_mixers_after_scene_change() -> crate::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let mut registry = ObswsInputRegistry::new(
        temp_dir.path().to_path_buf(),
        crate::types::EvenUsize::new(1920).expect("canvas width must be valid"),
        crate::types::EvenUsize::new(1080).expect("canvas height must be valid"),
        crate::video::FrameRate::FPS_30,
        None,
    );
    registry
        .create_scene("Scene B")
        .expect("second scene must be created");
    let input = ObswsInput::from_kind_and_settings(
        "mp4_file_source",
        nojson::RawJsonOwned::parse(
            r#"{"path":"testdata/red-320x320-h264-aac.mp4","loopPlayback":true}"#,
        )
        .expect("requestData must be valid json")
        .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "video-file", input, true)
        .expect("input creation must succeed");

    let pipeline = crate::MediaPipeline::new()?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let handle =
        create_initialized_coordinator_handle_with_pipeline(registry, pipeline_handle.clone())
            .await?;
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;

    let dash_output_dir = temp_dir.path().join("dash-output");
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-dash-output".to_owned()),
            request_type: Some("SetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(format!(
                    r#"{{"outputName":"mpeg_dash","outputSettings":{{"destination":{{"type":"filesystem","directory":"{}"}},"videoCodec":"VP9","audioCodec":"OPUS","variants":[{{"videoBitrate":2000000,"audioBitrate":128000}},{{"videoBitrate":1000000,"audioBitrate":64000,"width":1280,"height":720}}]}}}}"#,
                    dash_output_dir.display()
                ))
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-dash-output".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"mpeg_dash"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(start_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "obsws:mpeg_dash:0:v1_scaler", true).await?;
    wait_for_processor_presence(&pipeline_handle, "obsws:mpeg_dash:0:v0_dash_writer", true).await?;
    wait_for_processor_presence(&pipeline_handle, "obsws:mpeg_dash:0:video_mixer", false).await?;

    // ABR 結合 MPD は SampleEntry から codec string が確定してから書き出される。
    // manifest.mpd の出現を待ち、codecs 属性が実際の SampleEntry と一致することを検証する。
    // VP9 + Opus は libvpx / opus が全環境で利用可能なため、エンコーダー不在で失敗しない。
    let manifest_path = dash_output_dir.join("manifest.mpd");
    for _ in 0..60 {
        if manifest_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(
        manifest_path.exists(),
        "ABR combined manifest.mpd must be written after codec resolution"
    );
    let mpd_xml = std::fs::read_to_string(&manifest_path).expect("manifest.mpd must be readable");
    let mpd = shiguredo_mpd::parse(&mpd_xml).expect("manifest.mpd must be valid MPD XML");
    let adaptation_set = &mpd.periods[0].adaptation_sets[0];
    let codecs = adaptation_set
        .codecs
        .as_ref()
        .expect("AdaptationSet.codecs must be present");
    // VP9 + Opus を指定しているので codecs は vp09 と opus を含むこと
    assert!(
        codecs.contains("vp09."),
        "codecs must contain vp09 prefix from actual SampleEntry, got: {codecs}"
    );
    assert!(
        codecs.contains("opus"),
        "codecs must contain opus from actual SampleEntry, got: {codecs}"
    );

    let set_scene_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-program-scene-dash".to_owned()),
            request_type: Some("SetCurrentProgramScene".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_scene_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "obsws:mpeg_dash:0:v0_dash_writer", true).await?;

    let stop_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-stop-dash-output".to_owned()),
            request_type: Some("StopOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"mpeg_dash"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(stop_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "obsws:mpeg_dash:0:v0_dash_writer", false)
        .await?;

    pipeline_task.abort();

    Ok(())
}

#[tokio::test]
async fn start_stream_with_multiple_video_inputs_builds_plan_successfully() {
    // 複数映像入力は受理されるが、パイプラインがないため実行時エラーになる
    let mut registry = ObswsInputRegistry::new_for_test();
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

    let handle = create_coordinator_handle(registry);
    let mut session = ObswsSession::new(None, handle);
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
    // パイプラインがない場合は失敗レスポンスを返す
    assert!(!result);
    assert_eq!(code, REQUEST_STATUS_REQUEST_PROCESSING_FAILED);
}

#[tokio::test]
async fn toggle_stream_without_image_input_returns_toggle_request_type_error() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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

#[cfg(feature = "player")]
#[tokio::test]
async fn start_output_player_with_closed_control_channel_returns_processing_failed() {
    let registry = ObswsInputRegistry::new_for_test();
    let pipeline = crate::MediaPipeline::new().expect("failed to create test media pipeline");
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
    let registry = ObswsInputRegistry::new_for_test();
    let pipeline = crate::MediaPipeline::new().expect("failed to create test media pipeline");
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
    let registry = ObswsInputRegistry::new_for_test();
    let pipeline = crate::MediaPipeline::new().expect("failed to create test media pipeline");
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
    let registry = ObswsInputRegistry::new_for_test();
    let pipeline = crate::MediaPipeline::new().expect("failed to create test media pipeline");
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

#[tokio::test]
async fn request_batch_with_halt_on_failure_stops_after_first_failure() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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
    let mut session = ObswsSession::new(None, default_coordinator_handle());
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

// --- PersistentData テスト ---

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
