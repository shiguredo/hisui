// coordinator モジュールの設計方針:
//
// ObswsCoordinator は obsws の状態変更・副作用・Program 出力同期を調停する actor である。
// coordinator.rs には型定義、actor のイベントループ、リクエストのディスパッチ、
// および複数のサブモジュールから共通で利用される補助メソッド・free 関数を配置する。
//
// 個別のハンドラ実装は以下のサブモジュールに分割されている:
// - handle:       ObswsCoordinatorHandle（非同期 RPC インターフェース）
// - scene:        Scene 系ハンドラ
// - input:        Input / Media Input 系ハンドラ + source processor 管理
// - scene_item:   SceneItem 系ハンドラ
// - output:       Output リクエスト接着層 + 共通 processor ユーティリティ
// - output_*:     各 output エンジン（stream, record, hls, dash, rtmp, sora）

mod handle;
pub use handle::ObswsCoordinatorHandle;
mod input;
mod output;
mod output_dash;
mod output_hls;
#[cfg(feature = "player")]
mod output_player;
mod output_record;
mod output_rtmp;
mod output_sora;
mod output_stream;
mod scene;
mod scene_item;

use crate::obsws::event::TaggedEvent;
use crate::obsws::input_registry::ObswsInputRegistry;
use crate::obsws::message::ObswsSessionStats;
use crate::obsws::protocol::{OBSWS_EVENT_SUB_GENERAL, REQUEST_STATUS_MISSING_REQUEST_DATA};

#[derive(Clone)]
struct ObswsProgramOutputContext {
    video_track_id: crate::TrackId,
    audio_track_id: crate::TrackId,
    canvas_width: crate::types::EvenUsize,
    canvas_height: crate::types::EvenUsize,
    frame_rate: crate::video::FrameRate,
}

/// coordinator に送信するコマンド
pub enum ObswsCoordinatorCommand {
    /// 単一リクエストを処理する
    ProcessRequest {
        request: crate::obsws::message::RequestMessage,
        session_stats: ObswsSessionStats,
        reply_tx: tokio::sync::oneshot::Sender<CommandResult>,
    },
    /// RequestBatch 内のリクエストを逐次処理する
    ProcessRequestBatch {
        requests: Vec<crate::obsws::message::RequestMessage>,
        session_stats: ObswsSessionStats,
        halt_on_failure: bool,
        reply_tx: tokio::sync::oneshot::Sender<BatchCommandResult>,
    },
    /// bootstrap 用の入力 snapshot を取得する
    GetBootstrapSnapshot {
        reply_tx: tokio::sync::oneshot::Sender<Vec<BootstrapInputSnapshot>>,
    },
    /// webrtc_source の settings を取得する
    GetWebRtcSourceSettings {
        input_name: String,
        reply_tx: tokio::sync::oneshot::Sender<
            Option<crate::obsws::input_registry::ObswsWebRtcSourceSettings>,
        >,
    },
    /// webrtc_source の trackId を更新する（InputSettingsChanged イベントを発火する）
    UpdateWebRtcSourceTrackId {
        input_name: String,
        track_id: Option<String>,
    },
    /// inputName から最新の input 情報を解決する
    ResolveInputByName {
        input_name: String,
        reply_tx: tokio::sync::oneshot::Sender<Option<ResolvedInputInfo>>,
    },
    #[cfg(feature = "player")]
    /// player のライフサイクルイベントを処理する
    HandlePlayerLifecycleEvent {
        event: crate::obsws::player::PlayerLifecycleEvent,
    },
}

/// inputName から解決した input 情報
#[derive(Clone, Debug)]
pub struct ResolvedInputInfo {
    pub input_uuid: String,
    pub input_kind: String,
    pub video_track_id: Option<crate::TrackId>,
}

/// bootstrap 用の入力 snapshot
#[derive(Clone, Debug)]
pub struct BootstrapInputSnapshot {
    pub input_uuid: String,
    pub input_name: String,
    pub input_kind: String,
    pub video_track_id: Option<crate::TrackId>,
    pub audio_track_id: Option<crate::TrackId>,
}

/// bootstrap 用の入力差分イベント
#[derive(Clone, Debug)]
pub enum BootstrapInputEvent {
    InputCreated(BootstrapInputSnapshot),
    InputRemoved { input_uuid: String },
}

/// 単一リクエストの処理結果
pub struct CommandResult {
    pub response_text: nojson::RawJsonOwned,
    pub events: Vec<TaggedEvent>,
    pub batch_result: crate::obsws::response::RequestBatchResult,
}

/// バッチリクエストの処理結果
pub struct BatchCommandResult {
    pub results: Vec<crate::obsws::response::RequestBatchResult>,
    pub events: Vec<TaggedEvent>,
}

/// Program 出力の固定トラック ID
#[derive(Clone)]
pub struct ProgramTrackIds {
    pub video_track_id: crate::TrackId,
    pub audio_track_id: crate::TrackId,
}

/// 入力ごとの source processor 状態
pub struct InputSourceState {
    pub processor_ids: Vec<crate::ProcessorId>,
    pub video_track_id: Option<crate::TrackId>,
    pub audio_track_id: Option<crate::TrackId>,
    /// メディア入力の再生制御ハンドル（mp4_file_source の場合のみ）
    pub media_handle: Option<crate::mp4::reader::MediaInputHandle>,
    /// input_name の最新値を配信する watch sender（SetInputName 時に更新）
    pub input_name_tx: Option<tokio::sync::watch::Sender<String>>,
}

/// obsws の状態変更・副作用・Program 出力同期を調停する coordinator
pub struct ObswsCoordinator {
    input_registry: ObswsInputRegistry,
    program_output: crate::obsws::server::ProgramOutputState,
    pipeline_handle: Option<crate::MediaPipelineHandle>,
    command_rx: tokio::sync::mpsc::UnboundedReceiver<ObswsCoordinatorCommand>,
    /// 入力ごとの source processor 管理（キーは input_uuid）
    input_source_processors: std::collections::BTreeMap<String, InputSourceState>,
    /// bootstrap 用の差分イベント送信チャネル
    bootstrap_event_tx: tokio::sync::broadcast::Sender<BootstrapInputEvent>,
    /// obsws イベント broadcast チャネル（ProcessRequest 経由でない外部変更の通知用）
    obsws_event_tx: tokio::sync::broadcast::Sender<TaggedEvent>,
    /// state file 保存失敗等の致命的エラーにより終了が必要
    should_terminate: bool,
    /// 致命的エラー発生時にサーバーへ通知するための送信側
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// SoraSubscriber の状態管理（subscriberName → 状態）
    sora_subscribers: std::collections::BTreeMap<String, output_sora::SoraSubscriberState>,
    /// SoraSubscriber からのイベント受信チャネル
    sora_source_event_rx: tokio::sync::mpsc::UnboundedReceiver<crate::sora_source::SoraSourceEvent>,
    /// SoraSubscriber からのイベント送信チャネル（processor に渡す）
    sora_source_event_tx: tokio::sync::mpsc::UnboundedSender<crate::sora_source::SoraSourceEvent>,
    /// player output 用の制御・メディアチャネル
    #[cfg(feature = "player")]
    pub(crate) player_command_tx: std::sync::mpsc::SyncSender<crate::obsws::player::PlayerCommand>,
    #[cfg(feature = "player")]
    pub(crate) player_media_tx:
        std::sync::mpsc::SyncSender<crate::obsws::player::PlayerMediaMessage>,
    /// player output のサブスクライバタスクハンドル
    #[cfg(feature = "player")]
    pub(crate) player_subscriber_handle: Option<tokio::task::JoinHandle<()>>,
    /// player セッションの世代 ID（古い Stopped イベントを無視するために使用）
    #[cfg(feature = "player")]
    pub(crate) player_generation: u64,
}

impl ObswsCoordinator {
    /// actor と handle を生成する。program_output の初期化は呼び出し側で行う。
    ///
    /// 返り値の `watch::Receiver<bool>` は致命的エラー発生時に `true` を受信する。
    /// サーバーの accept loop はこれを監視して graceful shutdown を行う。
    pub fn new(
        input_registry: ObswsInputRegistry,
        program_output: crate::obsws::server::ProgramOutputState,
        pipeline_handle: Option<crate::MediaPipelineHandle>,
        #[cfg(feature = "player")] player_command_tx: std::sync::mpsc::SyncSender<
            crate::obsws::player::PlayerCommand,
        >,
        #[cfg(feature = "player")] player_media_tx: std::sync::mpsc::SyncSender<
            crate::obsws::player::PlayerMediaMessage,
        >,
    ) -> (
        Self,
        handle::ObswsCoordinatorHandle,
        tokio::sync::watch::Receiver<bool>,
    ) {
        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let (bootstrap_event_tx, _) = tokio::sync::broadcast::channel(64);
        let (obsws_event_tx, _) = tokio::sync::broadcast::channel(64);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let (sora_source_event_tx, sora_source_event_rx) = tokio::sync::mpsc::unbounded_channel();
        let program_track_ids = ProgramTrackIds {
            video_track_id: program_output.video_track_id.clone(),
            audio_track_id: program_output.audio_track_id.clone(),
        };
        let actor = Self {
            input_registry,
            program_output,
            pipeline_handle,
            command_rx,
            input_source_processors: std::collections::BTreeMap::new(),
            obsws_event_tx: obsws_event_tx.clone(),
            bootstrap_event_tx: bootstrap_event_tx.clone(),
            should_terminate: false,
            shutdown_tx,
            sora_subscribers: std::collections::BTreeMap::new(),
            sora_source_event_rx,
            sora_source_event_tx,
            #[cfg(feature = "player")]
            player_command_tx,
            #[cfg(feature = "player")]
            player_media_tx,
            #[cfg(feature = "player")]
            player_subscriber_handle: None,
            #[cfg(feature = "player")]
            player_generation: 0,
        };
        let handle = handle::ObswsCoordinatorHandle::new(
            command_tx,
            program_track_ids,
            bootstrap_event_tx,
            obsws_event_tx,
        );
        (actor, handle, shutdown_rx)
    }

    /// actor のイベントループを実行する
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                command = self.command_rx.recv() => {
                    let Some(command) = command else { break; };
                    match command {
                        ObswsCoordinatorCommand::ProcessRequest {
                            request,
                            session_stats,
                            reply_tx,
                        } => {
                            let result = self.handle_request(request, &session_stats).await;
                            let _ = reply_tx.send(result);
                        }
                        ObswsCoordinatorCommand::ProcessRequestBatch {
                            requests,
                            session_stats,
                            halt_on_failure,
                            reply_tx,
                        } => {
                            let result = self
                                .handle_request_batch(requests, &session_stats, halt_on_failure)
                                .await;
                            let _ = reply_tx.send(result);
                        }
                        ObswsCoordinatorCommand::GetBootstrapSnapshot { reply_tx } => {
                            let snapshot = self.build_bootstrap_snapshot();
                            let _ = reply_tx.send(snapshot);
                        }
                        ObswsCoordinatorCommand::GetWebRtcSourceSettings {
                            input_name,
                            reply_tx,
                        } => {
                            let settings = self.get_webrtc_source_settings(&input_name);
                            let _ = reply_tx.send(settings);
                        }
                        ObswsCoordinatorCommand::UpdateWebRtcSourceTrackId {
                            input_name,
                            track_id,
                        } => {
                            self.update_webrtc_source_track_id(&input_name, track_id);
                        }
                        ObswsCoordinatorCommand::ResolveInputByName {
                            input_name,
                            reply_tx,
                        } => {
                            let info = self.resolve_input_by_name(&input_name);
                            let _ = reply_tx.send(info);
                        }
                        #[cfg(feature = "player")]
                        ObswsCoordinatorCommand::HandlePlayerLifecycleEvent { event } => {
                            self.handle_player_lifecycle_event(event);
                        }
                    }
                },
                event = self.sora_source_event_rx.recv() => {
                    if let Some(event) = event {
                        self.handle_sora_source_event(event);
                    }
                },
            }

            // state file 保存失敗等の致命的エラーが発生した場合はループを抜ける。
            if self.should_terminate {
                tracing::error!(
                    "coordinator shutting down due to fatal error; \
                     subsequent requests will fail"
                );
                let _ = self.shutdown_tx.send(true);
                return;
            }
        }
    }

    #[cfg(feature = "player")]
    fn handle_player_lifecycle_event(&mut self, event: crate::obsws::player::PlayerLifecycleEvent) {
        match event {
            crate::obsws::player::PlayerLifecycleEvent::Stopped { generation } => {
                // 古い世代の Stopped イベントは無視する（Stop→Start の直後に届く場合がある）
                if generation != self.player_generation {
                    return;
                }
                if let Some(handle) = self.player_subscriber_handle.take() {
                    handle.abort();
                }
                self.input_registry.deactivate_player();
            }
        }
    }

    /// 単一リクエストを処理する
    async fn handle_request(
        &mut self,
        request: crate::obsws::message::RequestMessage,
        session_stats: &ObswsSessionStats,
    ) -> CommandResult {
        let request_type = request.request_type.clone().unwrap_or_default();
        let result = self.dispatch_request(request, session_stats).await;
        let request_succeeded = result.batch_result.request_status_result;
        if let Err(e) = self
            .sync_program_output_state(&request_type, request_succeeded)
            .await
        {
            tracing::warn!("failed to rebuild program output: {}", e.display());
        }

        // state file 保存: 対象リクエストが成功した場合に永続化する
        if request_succeeded
            && is_state_persisted_request(&request_type)
            && let Some(path) = self.input_registry.state_file_path()
        {
            let path = path.to_path_buf();
            let state = crate::obsws::state_file::build_state_from_registry(&self.input_registry);
            if let Err(e) = crate::obsws::state_file::save_state_file(&path, &state) {
                tracing::error!("failed to save state file: {}", e.display());
                self.should_terminate = true;
                return self.build_error_result(
                    &request_type,
                    &result.batch_result.request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &format!("state file write failed: {}", e.display()),
                );
            }
        }

        result
    }

    /// RequestBatch を逐次処理する
    async fn handle_request_batch(
        &mut self,
        requests: Vec<crate::obsws::message::RequestMessage>,
        session_stats: &ObswsSessionStats,
        halt_on_failure: bool,
    ) -> BatchCommandResult {
        let mut results = Vec::new();
        let mut events = Vec::new();
        let mut needs_save = false;
        for request in requests {
            let request_type = request.request_type.clone().unwrap_or_default();
            let result = self.dispatch_request(request, session_stats).await;
            let request_succeeded = result.batch_result.request_status_result;
            if request_succeeded && is_state_persisted_request(&request_type) {
                needs_save = true;
            }
            results.push(result.batch_result);
            events.extend(result.events);
            if let Err(e) = self
                .sync_program_output_state(&request_type, request_succeeded)
                .await
            {
                tracing::warn!("failed to rebuild program output: {}", e.display());
            }
            if (halt_on_failure && !request_succeeded) || self.should_terminate {
                break;
            }
        }

        // バッチ内で state 変更があった場合にまとめて保存する。
        // halt_on_failure で途中中断した場合でも、成功済みリクエストの副作用は
        // ロールバックしないため、それまでの変更を保存する。
        if needs_save
            && !self.should_terminate
            && let Some(path) = self.input_registry.state_file_path()
        {
            let path = path.to_path_buf();
            let state = crate::obsws::state_file::build_state_from_registry(&self.input_registry);
            if let Err(e) = crate::obsws::state_file::save_state_file(&path, &state) {
                tracing::error!("failed to save state file: {}", e.display());
                self.should_terminate = true;
                // バッチ結果に保存失敗エラーを追加する。
                // TODO: バッチの保存はループ後にまとめて行うため、特定のリクエストに
                // 紐付けられない。request_id / request_type が空文字列になるが、
                // クライアント側で対応付けできない点は将来的に改善を検討する。
                let error_result = crate::obsws::response::RequestBatchResult {
                    request_id: String::new(),
                    request_type: String::new(),
                    request_status_result: false,
                    request_status_code:
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    request_status_comment: Some(format!(
                        "state file write failed: {}",
                        e.display()
                    )),
                    response_data: None,
                };
                results.push(error_result);
            }
        }

        BatchCommandResult { results, events }
    }

    /// リクエストを種別に応じてディスパッチする
    async fn dispatch_request(
        &mut self,
        request: crate::obsws::message::RequestMessage,
        session_stats: &ObswsSessionStats,
    ) -> CommandResult {
        let request_id = request.request_id.clone().unwrap_or_default();
        let request_type = request.request_type.clone().unwrap_or_default();

        if request_id.is_empty() {
            return self.build_error_result(
                &request_type,
                &request_id,
                crate::obsws::protocol::REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required requestId field",
            );
        }
        if request_type.is_empty() {
            return self.build_error_result(
                &request_type,
                &request_id,
                crate::obsws::protocol::REQUEST_STATUS_MISSING_REQUEST_TYPE,
                "Missing required requestType field",
            );
        }

        match request_type.as_str() {
            // --- state write（状態変更系） ---
            "SetCurrentProgramScene" => {
                self.handle_set_current_program_scene(&request_id, request.request_data.as_ref())
            }
            "CreateScene" => self.handle_create_scene(&request_id, request.request_data.as_ref()),
            "RemoveScene" => self.handle_remove_scene(&request_id, request.request_data.as_ref()),
            "SetCurrentPreviewScene" => {
                // スタジオモード未対応のため常にエラーを返す
                let response_text =
                    crate::obsws::response::build_set_current_preview_scene_response(&request_id);
                self.build_result_from_response(response_text, Vec::new())
            }
            // Input 系
            "CreateInput" => {
                self.handle_create_input(&request_id, request.request_data.as_ref())
                    .await
            }
            "RemoveInput" => {
                self.handle_remove_input(&request_id, request.request_data.as_ref())
                    .await
            }
            "SetInputSettings" => {
                self.handle_set_input_settings(&request_id, request.request_data.as_ref())
                    .await
            }
            "SetInputMute" => {
                self.handle_set_input_mute(&request_id, request.request_data.as_ref())
                    .await
            }
            "ToggleInputMute" => {
                self.handle_toggle_input_mute(&request_id, request.request_data.as_ref())
                    .await
            }
            "SetInputVolume" => {
                self.handle_set_input_volume(&request_id, request.request_data.as_ref())
                    .await
            }
            "SetInputName" => {
                self.handle_set_input_name(&request_id, request.request_data.as_ref())
            }
            // Media Inputs 系
            "GetMediaInputStatus" => {
                self.handle_get_media_input_status(&request_id, request.request_data.as_ref())
            }
            "TriggerMediaInputAction" => {
                self.handle_trigger_media_input_action(&request_id, request.request_data.as_ref())
            }
            "SetMediaInputCursor" => {
                self.handle_set_media_input_cursor(&request_id, request.request_data.as_ref())
            }
            "OffsetMediaInputCursor" => {
                self.handle_offset_media_input_cursor(&request_id, request.request_data.as_ref())
            }
            // SceneItem 系
            "CreateSceneItem" => {
                self.handle_create_scene_item(&request_id, request.request_data.as_ref())
            }
            "RemoveSceneItem" => {
                self.handle_remove_scene_item(&request_id, request.request_data.as_ref())
            }
            "DuplicateSceneItem" => {
                self.handle_duplicate_scene_item(&request_id, request.request_data.as_ref())
            }
            "SetSceneItemEnabled" => {
                self.handle_set_scene_item_enabled(&request_id, request.request_data.as_ref())
            }
            "SetSceneItemLocked" => {
                self.handle_set_scene_item_locked(&request_id, request.request_data.as_ref())
            }
            "SetSceneItemIndex" => {
                self.handle_set_scene_item_index(&request_id, request.request_data.as_ref())
            }
            "SetSceneItemBlendMode" => {
                self.handle_set_scene_item_blend_mode(&request_id, request.request_data.as_ref())
            }
            "SetSceneItemTransform" => {
                self.handle_set_scene_item_transform(&request_id, request.request_data.as_ref())
            }
            // --- output side effect（pipeline 操作を伴う副作用系） ---
            "StartStream" => self.handle_start_stream_request(&request_id).await,
            "StopStream" => self.handle_stop_stream_request(&request_id).await,
            "ToggleStream" => self.handle_toggle_stream_request(&request_id).await,
            "StartRecord" => self.handle_start_record_request(&request_id).await,
            "StopRecord" => self.handle_stop_record_request(&request_id).await,
            "ToggleRecord" => self.handle_toggle_record_request(&request_id).await,
            "StartOutput" => {
                self.handle_start_output_request(&request_id, request.request_data.as_ref())
                    .await
            }
            "StopOutput" => {
                self.handle_stop_output_request(&request_id, request.request_data.as_ref())
                    .await
            }
            "ToggleOutput" => {
                self.handle_toggle_output_request(&request_id, request.request_data.as_ref())
                    .await
            }
            // --- SoraSubscriber 管理 ---
            "StartSoraSubscriber" => {
                self.handle_start_sora_subscriber(
                    &request_type,
                    &request_id,
                    request.request_data.as_ref(),
                )
                .await
            }
            "StopSoraSubscriber" => {
                self.handle_stop_sora_subscriber(
                    &request_type,
                    &request_id,
                    request.request_data.as_ref(),
                )
                .await
            }
            "ListSoraSubscribers" => self.handle_list_sora_subscribers(&request_id),
            "ListSoraSourceTracks" => {
                self.handle_list_sora_source_tracks(&request_id, request.request_data.as_ref())
            }
            "AttachSoraSourceTrack" => {
                self.handle_attach_sora_source_track(
                    &request_type,
                    &request_id,
                    request.request_data.as_ref(),
                )
                .await
            }
            "DetachSoraSourceTrack" => self.handle_detach_sora_source_track(
                &request_type,
                &request_id,
                request.request_data.as_ref(),
            ),
            // --- レジストリ状態変更なし ---
            "BroadcastCustomEvent" => {
                self.handle_broadcast_custom_event(&request_id, request.request_data.as_ref())
            }
            // --- pure read / 残りの state write: message.rs に委譲 ---
            _ => {
                let response = crate::obsws::message::handle_request_message_with_pipeline_handle(
                    request,
                    session_stats,
                    &mut self.input_registry,
                    self.pipeline_handle.as_ref(),
                );
                self.build_result_from_response(response.message, Vec::new())
            }
        }
    }

    // -----------------------------------------------------------------------
    // セッションローカルハンドラ（状態変更なし）
    // -----------------------------------------------------------------------

    fn handle_broadcast_custom_event(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(request_data) = request_data else {
            return self.build_error_result(
                "BroadcastCustomEvent",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };
        let event_data = match parse_custom_event_data(request_data) {
            Ok(data) => data,
            Err(error) => {
                return self.build_parse_error_result("BroadcastCustomEvent", request_id, &error);
            }
        };
        let response_text =
            crate::obsws::response::build_broadcast_custom_event_response(request_id);
        let events = vec![TaggedEvent {
            text: crate::obsws::response::build_custom_event(&event_data),
            subscription_flag: OBSWS_EVENT_SUB_GENERAL,
        }];
        self.build_result_from_response(response_text, events)
    }

    // -----------------------------------------------------------------------
    // Program 出力同期
    // -----------------------------------------------------------------------

    /// write リクエスト処理後に program output を同期する。
    /// actor が registry と program_output を両方所有するため TOCTOU リスクがない。
    async fn sync_program_output_state(
        &mut self,
        request_type: &str,
        request_succeeded: bool,
    ) -> crate::Result<()> {
        let scene_change_only = matches!(request_type, "SetCurrentProgramScene" | "RemoveScene");
        if !request_succeeded
            || !matches!(
                request_type,
                "SetCurrentProgramScene"
                    | "RemoveScene"
                    | "CreateInput"
                    | "RemoveInput"
                    | "SetInputSettings"
                    | "CreateSceneItem"
                    | "RemoveSceneItem"
                    | "DuplicateSceneItem"
                    | "SetSceneItemEnabled"
                    | "SetSceneItemIndex"
                    | "SetSceneItemBlendMode"
                    | "SetSceneItemTransform"
            )
        {
            return Ok(());
        }
        let current_scene_uuid = self
            .input_registry
            .current_program_scene()
            .map(|scene| scene.scene_uuid)
            .unwrap_or_default();
        if scene_change_only && self.program_output.scene_uuid == current_scene_uuid {
            return Ok(());
        }
        if self.pipeline_handle.is_none() {
            // pipeline がない場合はミキサー再構築をスキップし、シーン UUID だけ同期する
            self.program_output.scene_uuid = current_scene_uuid;
            return Ok(());
        }
        self.rebuild_program_output().await
    }

    /// bootstrap 用の入力 snapshot を構築する
    fn build_bootstrap_snapshot(&self) -> Vec<BootstrapInputSnapshot> {
        self.input_source_processors
            .iter()
            .filter_map(|(input_uuid, state)| {
                let entry = self.input_registry.find_input(Some(input_uuid), None)?;
                Some(BootstrapInputSnapshot {
                    input_uuid: entry.input_uuid.clone(),
                    input_name: entry.input_name.clone(),
                    input_kind: entry.input.kind_name().to_owned(),
                    video_track_id: state.video_track_id.clone(),
                    audio_track_id: state.audio_track_id.clone(),
                })
            })
            .collect()
    }

    /// webrtc_source の settings を取得する
    fn get_webrtc_source_settings(
        &self,
        input_name: &str,
    ) -> Option<crate::obsws::input_registry::ObswsWebRtcSourceSettings> {
        let entry = self.input_registry.find_input(None, Some(input_name))?;
        match &entry.input.settings {
            crate::obsws::input_registry::ObswsInputSettings::WebRtcSource(settings) => {
                Some(settings.clone())
            }
            _ => None,
        }
    }

    /// webrtc_source の trackId を更新し、InputSettingsChanged イベントを broadcast する
    fn update_webrtc_source_track_id(&mut self, input_name: &str, track_id: Option<String>) {
        let Some(uuid) = self.input_registry.uuids_by_name.get(input_name).cloned() else {
            return;
        };
        let Some(entry) = self.input_registry.inputs_by_uuid.get_mut(&uuid) else {
            return;
        };
        if let crate::obsws::input_registry::ObswsInputSettings::WebRtcSource(ref mut settings) =
            entry.input.settings
        {
            settings.track_id = track_id;
        }
        // InputSettingsChanged イベントを broadcast する
        let event = TaggedEvent {
            text: crate::obsws::response::build_input_settings_changed_event(
                &entry.input_name,
                &entry.input_uuid,
                &entry.input.settings,
            ),
            subscription_flag: crate::obsws::protocol::OBSWS_EVENT_SUB_INPUTS,
        };
        let _ = self.obsws_event_tx.send(event);
    }

    /// inputName から最新の input 情報を解決する（名前変更後も正しく解決する）
    fn resolve_input_by_name(&self, input_name: &str) -> Option<ResolvedInputInfo> {
        let entry = self.input_registry.find_input(None, Some(input_name))?;
        let video_track_id = self
            .input_source_processors
            .get(&entry.input_uuid)
            .and_then(|state| state.video_track_id.clone());
        Some(ResolvedInputInfo {
            input_uuid: entry.input_uuid.clone(),
            input_kind: entry.input.kind_name().to_owned(),
            video_track_id,
        })
    }

    /// Program 出力を再構築する。
    /// source processor は入力ライフサイクルで管理するため、ここではミキサーの入力トラックのみ更新する。
    async fn rebuild_program_output(&mut self) -> crate::Result<()> {
        let pipeline_handle = self
            .pipeline_handle
            .as_ref()
            .ok_or_else(|| crate::Error::new("BUG: obsws pipeline handle is not initialized"))?;

        let current_scene_uuid = self
            .input_registry
            .current_program_scene()
            .map(|scene| scene.scene_uuid)
            .unwrap_or_default();
        let scene_inputs = self
            .input_registry
            .list_current_program_scene_input_entries();
        let output_plan = crate::obsws::output_plan::build_composed_output_plan(
            &scene_inputs,
            self.input_registry.canvas_width(),
            self.input_registry.canvas_height(),
            self.input_registry.frame_rate(),
        )
        .map_err(|e| {
            crate::Error::new(format!(
                "failed to build program output plan: {}",
                e.message()
            ))
        })?;

        // ミキサーの入力トラックを更新する
        crate::obsws::session::output::update_program_mixers(
            pipeline_handle,
            &output_plan,
            &self.program_output.video_mixer_processor_id,
            &self.program_output.audio_mixer_processor_id,
        )
        .await?;

        self.program_output.scene_uuid = current_scene_uuid;

        // mixer の入力トラック更新後に、各入力のミュート・音量を同期する。
        // state file から復元した値や、前回の SetInputMute/SetInputVolume の結果を反映する。
        self.sync_all_input_mute_volume().await;

        tracing::info!("program output rebuilt for scene change");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // ヘルパー
    // -----------------------------------------------------------------------

    fn build_result_from_response(
        &self,
        response_text: nojson::RawJsonOwned,
        events: Vec<TaggedEvent>,
    ) -> CommandResult {
        let batch_result =
            crate::obsws::response::parse_request_response_for_batch_result(&response_text)
                .unwrap_or_else(|_| crate::obsws::response::RequestBatchResult {
                    request_id: String::new(),
                    request_type: String::new(),
                    request_status_result: false,
                    request_status_code: 0,
                    request_status_comment: None,
                    response_data: None,
                });
        CommandResult {
            response_text,
            events,
            batch_result,
        }
    }

    fn build_error_result(
        &self,
        request_type: &str,
        request_id: &str,
        status_code: i64,
        status_comment: &str,
    ) -> CommandResult {
        let response_text = crate::obsws::response::build_request_response_error(
            request_type,
            request_id,
            status_code,
            status_comment,
        );
        CommandResult {
            response_text,
            events: Vec::new(),
            batch_result: crate::obsws::response::RequestBatchResult {
                request_id: request_id.to_owned(),
                request_type: request_type.to_owned(),
                request_status_result: false,
                request_status_code: status_code,
                request_status_comment: Some(status_comment.to_owned()),
                response_data: None,
            },
        }
    }

    fn build_parse_error_result(
        &self,
        request_type: &str,
        request_id: &str,
        error: &nojson::JsonParseError,
    ) -> CommandResult {
        let code = crate::obsws::response::request_status_code_for_parse_error(error);
        self.build_error_result(request_type, request_id, code, &error.to_string())
    }
}

// -----------------------------------------------------------------------
// ユーティリティ関数
// -----------------------------------------------------------------------

/// state file への保存対象となるリクエストかどうかを判定する。
/// スタジオモードを実装した場合は SetCurrentPreviewScene も追加すること。
fn is_state_persisted_request(request_type: &str) -> bool {
    matches!(
        request_type,
        // config
        "SetPersistentData"
            // output 設定
            | "SetStreamServiceSettings"
            | "SetRecordDirectory"
            | "SetOutputSettings"
            // scene
            | "CreateScene"
            | "RemoveScene"
            | "SetCurrentProgramScene"
            // input
            | "CreateInput"
            | "RemoveInput"
            | "SetInputSettings"
            | "SetInputName"
            | "SetInputMute"
            | "ToggleInputMute"
            | "SetInputVolume"
            // scene item
            | "CreateSceneItem"
            | "RemoveSceneItem"
            | "DuplicateSceneItem"
            | "SetSceneItemEnabled"
            | "SetSceneItemLocked"
            | "SetSceneItemIndex"
            | "SetSceneItemBlendMode"
            | "SetSceneItemTransform"
            // transition override
            | "SetSceneSceneTransitionOverride"
    )
}

fn parse_required_non_empty_string_field(
    request_data: Option<&nojson::RawJsonOwned>,
    field_name: &str,
) -> Option<String> {
    let request_data = request_data?;
    let value: Option<String> = request_data
        .value()
        .to_member(field_name)
        .ok()?
        .try_into()
        .ok()?;
    let value = value?;
    if value.is_empty() {
        return None;
    }
    Some(value)
}

fn parse_custom_event_data(
    request_data: &nojson::RawJsonOwned,
) -> Result<nojson::RawJsonOwned, nojson::JsonParseError> {
    let event_data = request_data.value().to_member("eventData")?.required()?;
    if event_data.kind() != nojson::JsonValueKind::Object {
        return Err(event_data.invalid("object is required"));
    }
    nojson::RawJsonOwned::try_from(event_data)
}
