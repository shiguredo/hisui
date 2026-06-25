use super::*;
use crate::obsws::message::RequestMessage;
use crate::obsws::protocol::{
    REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_FIELD,
};
use crate::obsws::state::ObswsSessionState;

// Phase 1 で `tests/common.rs` に共通ヘルパー 23 件を物理移動した。
// 暫定の `use common::*;` はエントリポイント直下に残るテストが
// ヘルパーを修飾なしで呼び続けられるようにするもの。
// 全テストが各サブモジュールへ移動完了する Phase 14 で `mod common;` 含めて整理する。
#[path = "tests/common.rs"]
mod common;
#[path = "tests/input.rs"]
mod input;
#[path = "tests/lifecycle.rs"]
mod lifecycle;
#[path = "tests/output_create.rs"]
mod output_create;
#[path = "tests/output_hls_dash.rs"]
mod output_hls_dash;
#[path = "tests/output_misc_lifecycle.rs"]
mod output_misc_lifecycle;
#[cfg(feature = "player")]
#[path = "tests/output_player.rs"]
mod output_player;
#[path = "tests/output_record.rs"]
mod output_record;
#[path = "tests/output_stream.rs"]
mod output_stream;
#[path = "tests/scene.rs"]
mod scene;
#[path = "tests/scene_item.rs"]
mod scene_item;
use common::*;

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

// -----------------------------------------------------------------------
// テキストオーバーレイ機能のヘルパー (機能有効時)
// -----------------------------------------------------------------------

/// 機能有効時の `ObswsCoordinator` を立ち上げる。
///
/// `testdata/fonts/PublicSans-Regular.ttf` を `--font-search-root` 兼デフォルトフォントとして
/// 使い、 MediaPipeline + program mixer 群 (ビデオミキサーは内部レイヤとして
/// テキストオーバーレイ機能を有効化) を spawn する。
/// 戻り値の `ObswsCoordinatorHandle` を通じて 4 メソッドの obsws リクエストが実行できる。
async fn create_initialized_coordinator_with_text_overlay()
-> crate::Result<crate::obsws::coordinator::ObswsCoordinatorHandle> {
    let text_overlay_config = crate::mixer::video::text_overlay::TextOverlayConfig::new(
        std::path::PathBuf::from("testdata/fonts"),
        "PublicSans-Regular.ttf".to_owned(),
    )
    .expect("テスト用 TextOverlayConfig が組み立てられる");

    let registry = ObswsSessionState::new_for_test_with_text_overlay(text_overlay_config.clone());

    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())?;
    let pipeline_handle = pipeline.handle();
    tokio::spawn(pipeline.run());
    pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("trigger_start: pipeline terminated"))?;

    // テキストオーバーレイ機能は VideoRealtimeMixer の内部レイヤとして
    // `start_mixer_processors` 経由で組み込まれる。
    let scene_inputs = registry.list_current_program_scene_input_entries();
    let output_plan = crate::obsws::output_plan::build_composed_output_plan(
        &scene_inputs,
        registry.canvas_width(),
        registry.canvas_height(),
        registry.frame_rate(),
    )
    .map_err(|e| crate::Error::new(format!("failed to build output plan: {}", e.message())))?;
    crate::obsws::session::output::start_mixer_processors(
        &pipeline_handle,
        &output_plan,
        Some(text_overlay_config),
    )
    .await?;

    let scene_uuid = registry
        .current_program_scene()
        .map(|s| s.scene_uuid)
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
        std::path::PathBuf::from("recordings-for-test"),
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

/// テキストオーバーレイ系のリクエストを 1 件投げて `CommandResult` を返す。
async fn process_text_overlay_request(
    coordinator: &crate::obsws::coordinator::ObswsCoordinatorHandle,
    request_id: &str,
    request_type: &str,
    request_data_json: Option<&str>,
) -> crate::obsws::coordinator::CommandResult {
    let request_data = request_data_json
        .map(|s| nojson::RawJsonOwned::parse(s).expect("テスト requestData JSON は妥当"));
    let request = crate::obsws::message::RequestMessage {
        request_id: Some(request_id.to_owned()),
        request_type: Some(request_type.to_owned()),
        request_data,
    };
    let stats = crate::obsws::message::ObswsSessionStats::default();
    coordinator
        .process_request(request, stats)
        .await
        .expect("coordinator のリクエスト処理は成功する")
}

/// `HisuiListTextOverlays` レスポンスの `textOverlays` 配列の長さを返す。
fn parse_text_overlays_count(text: &nojson::RawJsonOwned) -> usize {
    let json = nojson::RawJson::parse(text.text()).expect("List レスポンスは JSON");
    json.value()
        .to_path_member(&["d", "responseData", "textOverlays"])
        .expect("textOverlays フィールドが存在する")
        .required()
        .expect("textOverlays は値を持つ")
        .to_array()
        .expect("textOverlays は配列")
        .count()
}

// -----------------------------------------------------------------------
// テキストオーバーレイ機能の無効時挙動。
// 機能有効時のフルパス検証は VideoRealtimeMixer の構築と TextOverlayLayer の
// raden 描画が必要なため、 mixer モジュール側の単体テストで別途扱う。

/// `default_coordinator_handle()` は `ObswsSessionState::new_for_test()` を使うため
/// テキストオーバーレイ機能が無効 (`text_overlay_config = None`) の状態である。
/// この状態で `HisuiCreateTextOverlay` を呼ぶと `RESOURCE_ACTION_NOT_SUPPORTED` が返る。
#[tokio::test]
async fn hisui_create_text_overlay_returns_disabled_when_feature_off() {
    let coordinator = default_coordinator_handle();
    let request = crate::obsws::message::RequestMessage {
        request_id: Some("req-1".to_owned()),
        request_type: Some("HisuiCreateTextOverlay".to_owned()),
        request_data: None,
    };
    let stats = crate::obsws::message::ObswsSessionStats::default();
    let result = coordinator
        .process_request(request, stats)
        .await
        .expect("coordinator のリクエスト処理は成功する");
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "機能無効時はエラー応答となる");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED,
        "REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED (606) が返る"
    );
}

/// `HisuiUpdateTextOverlay` も機能無効時は `RESOURCE_ACTION_NOT_SUPPORTED` を返す。
#[tokio::test]
async fn hisui_update_text_overlay_returns_disabled_when_feature_off() {
    let coordinator = default_coordinator_handle();
    let request = crate::obsws::message::RequestMessage {
        request_id: Some("req-1".to_owned()),
        request_type: Some("HisuiUpdateTextOverlay".to_owned()),
        request_data: None,
    };
    let stats = crate::obsws::message::ObswsSessionStats::default();
    let result = coordinator
        .process_request(request, stats)
        .await
        .expect("coordinator のリクエスト処理は成功する");
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "エラー応答となる");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED
    );
}

/// `HisuiRemoveTextOverlay` も機能無効時は `RESOURCE_ACTION_NOT_SUPPORTED` を返す。
#[tokio::test]
async fn hisui_remove_text_overlay_returns_disabled_when_feature_off() {
    let coordinator = default_coordinator_handle();
    let request = crate::obsws::message::RequestMessage {
        request_id: Some("req-1".to_owned()),
        request_type: Some("HisuiRemoveTextOverlay".to_owned()),
        request_data: None,
    };
    let stats = crate::obsws::message::ObswsSessionStats::default();
    let result = coordinator
        .process_request(request, stats)
        .await
        .expect("coordinator のリクエスト処理は成功する");
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "エラー応答となる");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED
    );
}

/// `HisuiListTextOverlays` も機能無効時は `RESOURCE_ACTION_NOT_SUPPORTED` を返す
/// (空配列ではなくエラーとする方針)。
#[tokio::test]
async fn hisui_list_text_overlays_returns_disabled_when_feature_off() {
    let coordinator = default_coordinator_handle();
    let request = crate::obsws::message::RequestMessage {
        request_id: Some("req-1".to_owned()),
        request_type: Some("HisuiListTextOverlays".to_owned()),
        request_data: None,
    };
    let stats = crate::obsws::message::ObswsSessionStats::default();
    let result = coordinator
        .process_request(request, stats)
        .await
        .expect("coordinator のリクエスト処理は成功する");
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "エラー応答となる");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED
    );
}

// -----------------------------------------------------------------------
// テキストオーバーレイ機能の有効時挙動 (4 メソッド往復・エラーケース)
// -----------------------------------------------------------------------

/// 機能有効時に Create → List → Update → List → Remove → List が正しく順序処理される。
/// List の中身も更新前後で確認することで、Create/Update/Remove の副作用が反映されることを検証する。
#[tokio::test]
async fn hisui_text_overlay_create_list_update_remove_roundtrip() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;

    // 初期状態は空。
    let initial = process_text_overlay_request(
        &coordinator,
        "req-initial-list",
        "HisuiListTextOverlays",
        None,
    )
    .await;
    let (success, _) = parse_request_status(&initial.response_text);
    assert!(success, "初期 List は成功する");
    assert_eq!(
        parse_text_overlays_count(&initial.response_text),
        0,
        "初期状態は overlay ゼロ件"
    );

    // Create で 1 件登録する。
    let create = process_text_overlay_request(
        &coordinator,
        "req-create",
        "HisuiCreateTextOverlay",
        Some(r#"{"textOverlayName":"greeting","text":"hello","x":100,"y":200,"fontSize":48}"#),
    )
    .await;
    let (success, _) = parse_request_status(&create.response_text);
    assert!(success, "Create は成功する");

    // List で 1 件返り、内容も Create で送った値と一致する。
    let after_create = process_text_overlay_request(
        &coordinator,
        "req-list-after-create",
        "HisuiListTextOverlays",
        None,
    )
    .await;
    let (success, _) = parse_request_status(&after_create.response_text);
    assert!(success, "Create 後の List は成功する");
    assert_eq!(
        parse_text_overlays_count(&after_create.response_text),
        1,
        "Create 後は 1 件存在する"
    );
    let json =
        nojson::RawJson::parse(after_create.response_text.text()).expect("List レスポンスは JSON");
    let item_name: String = json
        .value()
        .to_path_member(&["d", "responseData", "textOverlays"])
        .and_then(|v| v.required()?.to_array())
        .expect("textOverlays は配列")
        .next()
        .expect("少なくとも 1 件はある")
        .to_member("textOverlayName")
        .and_then(|v| v.required()?.try_into())
        .expect("textOverlayName は文字列");
    assert_eq!(item_name, "greeting", "Create で登録した名前が List に出る");

    // Update で text を更新する。
    let update = process_text_overlay_request(
        &coordinator,
        "req-update",
        "HisuiUpdateTextOverlay",
        Some(r#"{"textOverlayName":"greeting","text":"updated"}"#),
    )
    .await;
    let (success, _) = parse_request_status(&update.response_text);
    assert!(success, "Update は成功する");

    // List で更新後の text が反映されている。
    let after_update = process_text_overlay_request(
        &coordinator,
        "req-list-after-update",
        "HisuiListTextOverlays",
        None,
    )
    .await;
    let json =
        nojson::RawJson::parse(after_update.response_text.text()).expect("List レスポンスは JSON");
    let updated_text: String = json
        .value()
        .to_path_member(&["d", "responseData", "textOverlays"])
        .and_then(|v| v.required()?.to_array())
        .expect("textOverlays は配列")
        .next()
        .expect("Update 後も 1 件存在")
        .to_member("text")
        .and_then(|v| v.required()?.try_into())
        .expect("text は文字列");
    assert_eq!(updated_text, "updated", "Update で text が変わっている");

    // Remove で削除する。
    let remove = process_text_overlay_request(
        &coordinator,
        "req-remove",
        "HisuiRemoveTextOverlay",
        Some(r#"{"textOverlayName":"greeting"}"#),
    )
    .await;
    let (success, _) = parse_request_status(&remove.response_text);
    assert!(success, "Remove は成功する");

    // List で 0 件に戻る。
    let after_remove = process_text_overlay_request(
        &coordinator,
        "req-list-after-remove",
        "HisuiListTextOverlays",
        None,
    )
    .await;
    assert_eq!(
        parse_text_overlays_count(&after_remove.response_text),
        0,
        "Remove 後は 0 件"
    );

    Ok(())
}

/// 同名の Create を 2 回呼ぶと 2 回目は RESOURCE_ALREADY_EXISTS で拒否される。
#[tokio::test]
async fn hisui_create_text_overlay_rejects_duplicate_name() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;
    let body = r#"{"textOverlayName":"dup","text":"x","x":0,"y":0,"fontSize":32}"#;

    let first = process_text_overlay_request(
        &coordinator,
        "req-create-1",
        "HisuiCreateTextOverlay",
        Some(body),
    )
    .await;
    let (success, _) = parse_request_status(&first.response_text);
    assert!(success, "初回 Create は成功");

    let second = process_text_overlay_request(
        &coordinator,
        "req-create-2",
        "HisuiCreateTextOverlay",
        Some(body),
    )
    .await;
    let (success, code) = parse_request_status(&second.response_text);
    assert!(!success, "重複 Create は失敗");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
        "重複は RESOURCE_ALREADY_EXISTS (602)"
    );
    Ok(())
}

/// 未登録名で Update すると RESOURCE_NOT_FOUND で拒否される。
#[tokio::test]
async fn hisui_update_text_overlay_rejects_unknown_name() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;
    let result = process_text_overlay_request(
        &coordinator,
        "req-update-missing",
        "HisuiUpdateTextOverlay",
        Some(r#"{"textOverlayName":"missing","text":"x"}"#),
    )
    .await;
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "エラー応答となる");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
        "未登録は RESOURCE_NOT_FOUND (601)"
    );
    Ok(())
}

/// 未登録名で Remove すると RESOURCE_NOT_FOUND で拒否される。
#[tokio::test]
async fn hisui_remove_text_overlay_rejects_unknown_name() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;
    let result = process_text_overlay_request(
        &coordinator,
        "req-remove-missing",
        "HisuiRemoveTextOverlay",
        Some(r#"{"textOverlayName":"missing"}"#),
    )
    .await;
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "エラー応答となる");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
        "未登録は RESOURCE_NOT_FOUND (601)"
    );
    Ok(())
}

/// `fontName` に path traversal 文字 (`/` `\` `..` NUL) を含む値は INVALID_REQUEST_FIELD で拒否される。
#[tokio::test]
async fn hisui_create_text_overlay_rejects_invalid_font_name() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;
    // 各禁止文字パターンを 1 リクエストで 1 つずつ試す。
    for (label, font_name_json) in [
        ("slash", r#""foo/bar.ttf""#),
        ("backslash", r#""foo\\bar.ttf""#),
        ("dotdot", r#""../etc/passwd""#),
        // NUL バイトは JSON 文字列として "\u0000" でエンコードする
        ("nul", r#""evil\u0000.ttf""#),
    ] {
        let body = format!(
            r#"{{"textOverlayName":"name-{label}","text":"x","x":0,"y":0,"fontSize":32,"fontName":{font_name_json}}}"#
        );
        let result = process_text_overlay_request(
            &coordinator,
            &format!("req-{label}"),
            "HisuiCreateTextOverlay",
            Some(&body),
        )
        .await;
        let (success, code) = parse_request_status(&result.response_text);
        assert!(!success, "{label} は拒否される");
        assert_eq!(
            code,
            crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
            "{label} は INVALID_REQUEST_FIELD (400)"
        );
    }
    Ok(())
}

/// 存在しないフォントファイルを指定すると FontResolveFailed 経路で INVALID_REQUEST_FIELD が返る。
#[tokio::test]
async fn hisui_create_text_overlay_rejects_unresolvable_font() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;
    let result = process_text_overlay_request(
        &coordinator,
        "req-unresolvable",
        "HisuiCreateTextOverlay",
        Some(
            r#"{"textOverlayName":"x","text":"x","x":0,"y":0,"fontSize":32,"fontName":"nonexistent.ttf"}"#,
        ),
    )
    .await;
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "エラー応答となる");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
        "解決失敗は INVALID_REQUEST_FIELD (400)"
    );
    Ok(())
}

/// 不正な `fontColor` (`#GGGGGG` / `#FF` / `#` 不在 / 9 桁) は INVALID_REQUEST_FIELD で拒否される。
#[tokio::test]
async fn hisui_create_text_overlay_rejects_invalid_color() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;
    for (label, color) in [
        ("non-hex", "#GGGGGG"),
        ("too-short", "#FF"),
        ("missing-hash", "FF0000"),
        ("nine-digits", "#FFFFFFFFF"),
    ] {
        let body = format!(
            r#"{{"textOverlayName":"color-{label}","text":"x","x":0,"y":0,"fontSize":32,"fontColor":"{color}"}}"#
        );
        let result = process_text_overlay_request(
            &coordinator,
            &format!("req-color-{label}"),
            "HisuiCreateTextOverlay",
            Some(&body),
        )
        .await;
        let (success, code) = parse_request_status(&result.response_text);
        assert!(!success, "{label} は拒否される");
        assert_eq!(
            code,
            crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
            "{label} は INVALID_REQUEST_FIELD (400)"
        );
    }
    Ok(())
}

/// `fontSize` が範囲外 (0 / canvas_height 超過) は INVALID_REQUEST_FIELD で拒否される。
/// canvas は new_for_test_with_text_overlay で 1920x1080 固定なので 1081 を境界外とする。
#[tokio::test]
async fn hisui_create_text_overlay_rejects_invalid_font_size() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;
    for (label, font_size) in [("zero", 0u32), ("too-large", 1081u32)] {
        let body = format!(
            r#"{{"textOverlayName":"size-{label}","text":"x","x":0,"y":0,"fontSize":{font_size}}}"#
        );
        let result = process_text_overlay_request(
            &coordinator,
            &format!("req-size-{label}"),
            "HisuiCreateTextOverlay",
            Some(&body),
        )
        .await;
        let (success, code) = parse_request_status(&result.response_text);
        assert!(!success, "{label} は拒否される");
        assert_eq!(
            code,
            crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
            "{label} は INVALID_REQUEST_FIELD (400)"
        );
    }
    Ok(())
}

/// `text` がバイト数または行数の上限を超えると INVALID_REQUEST_FIELD で拒否される。
#[tokio::test]
async fn hisui_create_text_overlay_rejects_invalid_text() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;

    // バイト数上限超過 (TEXT_MAX_BYTES 超え)
    let too_long = "a".repeat(crate::mixer::video::text_overlay::TEXT_MAX_BYTES + 1);
    let body = format!(
        r#"{{"textOverlayName":"text-bytes","text":"{too_long}","x":0,"y":0,"fontSize":32}}"#
    );
    let result = process_text_overlay_request(
        &coordinator,
        "req-text-bytes",
        "HisuiCreateTextOverlay",
        Some(&body),
    )
    .await;
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "バイト数上限超過は拒否");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD
    );

    // 行数上限超過 (TEXT_MAX_LINES 超え)
    let too_many_newlines = "\\n".repeat(crate::mixer::video::text_overlay::TEXT_MAX_LINES);
    let body = format!(
        r#"{{"textOverlayName":"text-lines","text":"{too_many_newlines}","x":0,"y":0,"fontSize":32}}"#
    );
    let result = process_text_overlay_request(
        &coordinator,
        "req-text-lines",
        "HisuiCreateTextOverlay",
        Some(&body),
    )
    .await;
    let (success, code) = parse_request_status(&result.response_text);
    assert!(!success, "行数上限超過は拒否");
    assert_eq!(
        code,
        crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD
    );
    Ok(())
}

/// 必須フィールドが欠落していると MISSING_REQUEST_FIELD で拒否される。
/// `textOverlayName` / `text` / `x` / `y` / `fontSize` の各欠落を確認する。
#[tokio::test]
async fn hisui_create_text_overlay_returns_missing_request_field_when_required_missing()
-> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;

    // 各必須フィールドをそれぞれ欠落させた 5 種の requestData。
    let bodies = [
        // textOverlayName なし
        r#"{"text":"x","x":0,"y":0,"fontSize":32}"#,
        // text なし
        r#"{"textOverlayName":"a","x":0,"y":0,"fontSize":32}"#,
        // x なし
        r#"{"textOverlayName":"a","text":"x","y":0,"fontSize":32}"#,
        // y なし
        r#"{"textOverlayName":"a","text":"x","x":0,"fontSize":32}"#,
        // fontSize なし
        r#"{"textOverlayName":"a","text":"x","x":0,"y":0}"#,
    ];
    for (i, body) in bodies.iter().enumerate() {
        let result = process_text_overlay_request(
            &coordinator,
            &format!("req-missing-{i}"),
            "HisuiCreateTextOverlay",
            Some(body),
        )
        .await;
        let (success, code) = parse_request_status(&result.response_text);
        assert!(!success, "ケース {i} は拒否される: body={body}");
        assert_eq!(
            code,
            crate::obsws::protocol::REQUEST_STATUS_MISSING_REQUEST_FIELD,
            "ケース {i} は MISSING_REQUEST_FIELD (300): body={body}"
        );
    }
    Ok(())
}

/// 必須フィールドの型違反は INVALID_REQUEST_FIELD で返るべきで、MISSING_REQUEST_FIELD ではない。
#[tokio::test]
async fn hisui_create_text_overlay_returns_invalid_request_field_for_type_mismatch()
-> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;
    // 各必須フィールドに「期待型ではない値」を入れる。
    let bodies = [
        // textOverlayName が数値 (string 期待)
        r#"{"textOverlayName":123,"text":"x","x":0,"y":0,"fontSize":32}"#,
        // text が真偽値 (string 期待)
        r#"{"textOverlayName":"a","text":true,"x":0,"y":0,"fontSize":32}"#,
        // x が文字列 (integer 期待)
        r#"{"textOverlayName":"a","text":"x","x":"abc","y":0,"fontSize":32}"#,
        // y がオブジェクト (integer 期待)
        r#"{"textOverlayName":"a","text":"x","x":0,"y":{},"fontSize":32}"#,
        // fontSize が負数 (u32 範囲外)
        r#"{"textOverlayName":"a","text":"x","x":0,"y":0,"fontSize":-1}"#,
    ];
    for (i, body) in bodies.iter().enumerate() {
        let result = process_text_overlay_request(
            &coordinator,
            &format!("req-type-mismatch-{i}"),
            "HisuiCreateTextOverlay",
            Some(body),
        )
        .await;
        let (success, code) = parse_request_status(&result.response_text);
        assert!(!success, "ケース {i} は拒否される: body={body}");
        assert_eq!(
            code,
            crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
            "ケース {i} は INVALID_REQUEST_FIELD (400): body={body}"
        );
    }
    Ok(())
}

/// テキストオーバーレイの `z` フィールドは i32::MAX も valid 値として受け付ける。
#[tokio::test]
async fn hisui_text_overlay_accepts_i32_max_as_z_value() -> crate::Result<()> {
    let coordinator = create_initialized_coordinator_with_text_overlay().await?;

    // Create で z = i32::MAX (2147483647) を渡しても成功する。
    let create = process_text_overlay_request(
        &coordinator,
        "req-create-i32-max-z",
        "HisuiCreateTextOverlay",
        Some(r#"{"textOverlayName":"x","text":"x","x":0,"y":0,"fontSize":32,"z":2147483647}"#),
    )
    .await;
    let (success, _) = parse_request_status(&create.response_text);
    assert!(success, "i32::MAX も z として受け付ける");
    Ok(())
}

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
