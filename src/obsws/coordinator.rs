use crate::obsws::input_registry::ObswsInputRegistry;
use crate::obsws::message::ObswsSessionStats;
use crate::obsws::protocol::{
    OBSWS_EVENT_SUB_GENERAL, OBSWS_EVENT_SUB_INPUTS, OBSWS_EVENT_SUB_OUTPUTS,
    OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED, OBSWS_EVENT_SUB_SCENE_ITEMS,
    OBSWS_EVENT_SUB_SCENES, OBSWS_EVENT_SUB_SORA_SOURCE, REQUEST_STATUS_MISSING_REQUEST_DATA,
    REQUEST_STATUS_MISSING_REQUEST_FIELD, REQUEST_STATUS_RESOURCE_NOT_FOUND,
};

use std::time::Duration;

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

/// イベントの subscription flag 付きテキスト
#[derive(Clone)]
pub struct TaggedEvent {
    pub text: nojson::RawJsonOwned,
    pub subscription_flag: u32,
}

/// Program 出力の固定トラック ID
#[derive(Clone)]
pub struct ProgramTrackIds {
    pub video_track_id: crate::TrackId,
    pub audio_track_id: crate::TrackId,
}

/// coordinator への handle。セッションや bootstrap が保持する。
#[derive(Clone)]
pub struct ObswsCoordinatorHandle {
    command_tx: tokio::sync::mpsc::UnboundedSender<ObswsCoordinatorCommand>,
    program_track_ids: ProgramTrackIds,
    bootstrap_event_tx: tokio::sync::broadcast::Sender<BootstrapInputEvent>,
    obsws_event_tx: tokio::sync::broadcast::Sender<TaggedEvent>,
}

impl ObswsCoordinatorHandle {
    /// 単一リクエストを actor に送信し、結果を待つ
    pub async fn process_request(
        &self,
        request: crate::obsws::message::RequestMessage,
        session_stats: ObswsSessionStats,
    ) -> crate::Result<CommandResult> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.command_tx
            .send(ObswsCoordinatorCommand::ProcessRequest {
                request,
                session_stats,
                reply_tx,
            })
            .map_err(|_| crate::Error::new("coordinator has terminated"))?;
        reply_rx
            .await
            .map_err(|_| crate::Error::new("coordinator dropped reply channel"))
    }

    /// RequestBatch を actor に送信し、結果を待つ
    pub async fn process_request_batch(
        &self,
        requests: Vec<crate::obsws::message::RequestMessage>,
        session_stats: ObswsSessionStats,
        halt_on_failure: bool,
    ) -> crate::Result<BatchCommandResult> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.command_tx
            .send(ObswsCoordinatorCommand::ProcessRequestBatch {
                requests,
                session_stats,
                halt_on_failure,
                reply_tx,
            })
            .map_err(|_| crate::Error::new("coordinator has terminated"))?;
        reply_rx
            .await
            .map_err(|_| crate::Error::new("coordinator dropped reply channel"))
    }

    /// coordinator が保持する固定 Program 出力の video track ID を取得する
    pub fn program_video_track_id(&self) -> crate::TrackId {
        self.program_track_ids.video_track_id.clone()
    }

    /// coordinator が保持する固定 Program 出力の audio track ID を取得する
    pub fn program_audio_track_id(&self) -> crate::TrackId {
        self.program_track_ids.audio_track_id.clone()
    }

    /// bootstrap 用の入力 snapshot を取得する
    pub async fn get_bootstrap_snapshot(&self) -> crate::Result<Vec<BootstrapInputSnapshot>> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.command_tx
            .send(ObswsCoordinatorCommand::GetBootstrapSnapshot { reply_tx })
            .map_err(|_| crate::Error::new("coordinator has terminated"))?;
        reply_rx
            .await
            .map_err(|_| crate::Error::new("coordinator dropped reply channel"))
    }

    /// webrtc_source の settings を取得する
    pub async fn get_webrtc_source_settings(
        &self,
        input_name: &str,
    ) -> crate::Result<Option<crate::obsws::input_registry::ObswsWebRtcSourceSettings>> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.command_tx
            .send(ObswsCoordinatorCommand::GetWebRtcSourceSettings {
                input_name: input_name.to_owned(),
                reply_tx,
            })
            .map_err(|_| crate::Error::new("coordinator has terminated"))?;
        reply_rx
            .await
            .map_err(|_| crate::Error::new("coordinator dropped reply channel"))
    }

    /// webrtc_source の trackId を更新する
    pub fn update_webrtc_source_track_id(&self, input_name: &str, track_id: Option<String>) {
        let _ = self
            .command_tx
            .send(ObswsCoordinatorCommand::UpdateWebRtcSourceTrackId {
                input_name: input_name.to_owned(),
                track_id,
            });
    }

    /// inputName から最新の input 情報を解決する
    pub async fn resolve_input_by_name(
        &self,
        input_name: &str,
    ) -> crate::Result<Option<ResolvedInputInfo>> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.command_tx
            .send(ObswsCoordinatorCommand::ResolveInputByName {
                input_name: input_name.to_owned(),
                reply_tx,
            })
            .map_err(|_| crate::Error::new("coordinator has terminated"))?;
        reply_rx
            .await
            .map_err(|_| crate::Error::new("coordinator dropped reply channel"))
    }

    /// obsws イベント broadcast を購読する
    pub fn subscribe_obsws_events(&self) -> tokio::sync::broadcast::Receiver<TaggedEvent> {
        self.obsws_event_tx.subscribe()
    }

    /// bootstrap 用の差分イベントを購読する
    pub fn subscribe_bootstrap_events(
        &self,
    ) -> tokio::sync::broadcast::Receiver<BootstrapInputEvent> {
        self.bootstrap_event_tx.subscribe()
    }
}

/// 入力ごとの source processor 状態
pub struct InputSourceState {
    pub processor_ids: Vec<crate::ProcessorId>,
    pub video_track_id: Option<crate::TrackId>,
    pub audio_track_id: Option<crate::TrackId>,
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
    sora_subscribers: std::collections::BTreeMap<String, SoraSubscriberState>,
    /// SoraSubscriber からのイベント受信チャネル
    sora_source_event_rx: tokio::sync::mpsc::UnboundedReceiver<crate::sora_source::SoraSourceEvent>,
    /// SoraSubscriber からのイベント送信チャネル（processor に渡す）
    sora_source_event_tx: tokio::sync::mpsc::UnboundedSender<crate::sora_source::SoraSourceEvent>,
}

/// SoraSubscriber の状態
struct SoraSubscriberState {
    settings: crate::obsws::input_registry::ObswsSoraSubscriberSettings,
    run: Option<SoraSubscriberRun>,
    /// 受信中のリモートトラック（trackId → トラック情報）
    remote_tracks: std::collections::HashMap<String, SoraSourceRemoteTrack>,
    /// on_notify から抽出した接続情報（connection_id → info）
    connections: std::collections::HashMap<String, SoraConnectionInfo>,
}

/// 実行中の SoraSubscriber の情報
#[derive(Clone)]
struct SoraSubscriberRun {
    processor_id: crate::ProcessorId,
}

/// SoraSubscriber から受信したリモートトラックのメタデータ。
///
/// WebRTC の型（RtpTransceiver, VideoSink 等）は !Sync のため coordinator に直接保持できない。
/// 実際のフレーム転送は coordinator の外のタスクで管理し、coordinator はメタデータのみ保持する。
struct SoraSourceRemoteTrack {
    connection_id: String,
    client_id: Option<String>,
    track_kind: String,
    /// attach 先の input 名
    attached_input_name: Option<String>,
    /// attach 先の pipeline track ID
    attached_pipeline_track_id: Option<crate::TrackId>,
    /// holder タスクへのコマンド送信チャネル（Send+Sync）
    command_tx: tokio::sync::mpsc::UnboundedSender<crate::sora_source::SoraTrackCommand>,
    /// holder タスクの停止用ハンドル
    holder_abort: tokio::task::AbortHandle,
}

impl Drop for SoraSourceRemoteTrack {
    fn drop(&mut self) {
        self.holder_abort.abort();
    }
}

/// SoraSubscriber の on_notify から抽出した接続情報
struct SoraConnectionInfo {
    client_id: Option<String>,
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
    ) -> (
        Self,
        ObswsCoordinatorHandle,
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
        };
        let handle = ObswsCoordinatorHandle {
            command_tx,
            program_track_ids,
            bootstrap_event_tx,
            obsws_event_tx,
        };
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
                    }
                }
                event = self.sora_source_event_rx.recv() => {
                    if let Some(event) = event {
                        self.handle_sora_source_event(event);
                    }
                }
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
    // Scene 系ハンドラ
    // -----------------------------------------------------------------------

    fn handle_set_current_program_scene(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let previous_scene_name = self
            .input_registry
            .current_program_scene()
            .map(|scene| scene.scene_name);
        let response_text = crate::obsws::response::build_set_current_program_scene_response(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if let Some(current_scene) = self.input_registry.current_program_scene()
            && previous_scene_name.as_deref() != Some(current_scene.scene_name.as_str())
        {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_current_program_scene_changed_event(
                    &current_scene.scene_name,
                    &current_scene.scene_uuid,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENES,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    fn handle_create_scene(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let requested_scene_name = parse_required_non_empty_string_field(request_data, "sceneName");
        let existed_before = requested_scene_name.as_deref().is_some_and(|scene_name| {
            self.input_registry
                .list_scenes()
                .into_iter()
                .any(|scene| scene.scene_name == scene_name)
        });
        let response_text = crate::obsws::response::build_create_scene_response(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if !existed_before
            && let Some(requested_scene_name) = requested_scene_name
            && let Some(created_scene) = self
                .input_registry
                .list_scenes()
                .into_iter()
                .find(|scene| scene.scene_name == requested_scene_name)
        {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_created_event(
                    &created_scene.scene_name,
                    &created_scene.scene_uuid,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENES,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    fn handle_remove_scene(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let removed_scene = request_data.and_then(|rd| {
            let (scene_name, scene_uuid) =
                crate::obsws::response::parse_scene_lookup_fields_for_session(
                    rd.value(),
                    "sceneName",
                    "sceneUuid",
                )
                .ok()?;
            let resolved_name = self
                .input_registry
                .resolve_scene_name(scene_name.as_deref(), scene_uuid.as_deref())?;
            self.input_registry
                .list_scenes()
                .into_iter()
                .find(|scene| scene.scene_name == resolved_name)
        });
        let previous_current_scene_name = self
            .input_registry
            .current_program_scene()
            .map(|scene| scene.scene_name);
        let response_text = crate::obsws::response::build_remove_scene_response(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if let Some(removed_scene) = removed_scene {
            let removed_succeeded = self
                .input_registry
                .list_scenes()
                .into_iter()
                .all(|scene| scene.scene_uuid != removed_scene.scene_uuid);
            if removed_succeeded {
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_scene_removed_event(
                        &removed_scene.scene_name,
                        &removed_scene.scene_uuid,
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_SCENES,
                });
                if previous_current_scene_name.as_deref() == Some(removed_scene.scene_name.as_str())
                    && let Some(current_scene) = self.input_registry.current_program_scene()
                {
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_current_program_scene_changed_event(
                            &current_scene.scene_name,
                            &current_scene.scene_uuid,
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_SCENES,
                    });
                }
            }
        }
        self.build_result_from_response(response_text, events)
    }

    // -----------------------------------------------------------------------
    // Input 系ハンドラ
    // -----------------------------------------------------------------------

    async fn handle_create_input(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::execute_create_input(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let response_text = execution.response_text;
        let mut events = Vec::new();
        if let Some(created) = execution.created {
            // 入力ライフサイクルの source processor を起動する
            if let Err(e) = self
                .start_input_source_processor(&created.input_entry)
                .await
            {
                tracing::warn!(
                    "failed to start source processor for input {}: {}",
                    created.input_entry.input_uuid,
                    e.display()
                );
            }

            // bootstrap 用の差分イベントを送信する
            if let Some(source_state) = self
                .input_source_processors
                .get(&created.input_entry.input_uuid)
            {
                let _ = self
                    .bootstrap_event_tx
                    .send(BootstrapInputEvent::InputCreated(BootstrapInputSnapshot {
                        input_uuid: created.input_entry.input_uuid.clone(),
                        input_name: created.input_entry.input_name.clone(),
                        input_kind: created.input_entry.input.kind_name().to_owned(),
                        video_track_id: source_state.video_track_id.clone(),
                        audio_track_id: source_state.audio_track_id.clone(),
                    }));
            }

            events.push(TaggedEvent {
                text: crate::obsws::response::build_input_created_event(
                    &created.input_entry.input_name,
                    &created.input_entry.input_uuid,
                    created.input_entry.input.kind_name(),
                    &created.input_entry.input.settings,
                    &created.default_settings,
                ),
                subscription_flag: OBSWS_EVENT_SUB_INPUTS,
            });
            let scene_item = &created.scene_item_ref;
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_created_event(
                    &scene_item.scene_name,
                    &scene_item.scene_uuid,
                    scene_item.scene_item.scene_item_id,
                    &scene_item.scene_item.source_name,
                    &scene_item.scene_item.source_uuid,
                    scene_item.scene_item.scene_item_index,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
            });
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_transform_changed_event(
                    &scene_item.scene_name,
                    &scene_item.scene_uuid,
                    scene_item.scene_item.scene_item_id,
                    &scene_item.scene_item.scene_item_transform,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    async fn handle_remove_input(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(request_data) = request_data else {
            return self.build_error_result(
                "RemoveInput",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };
        let (input_uuid, input_name) =
            match crate::obsws::response::parse_input_lookup_fields_for_session(
                request_data.value(),
            ) {
                Ok(fields) => fields,
                Err(error) => {
                    return self.build_parse_error_result("RemoveInput", request_id, &error);
                }
            };
        let removed_input = self
            .input_registry
            .find_input(input_uuid.as_deref(), input_name.as_deref())
            .cloned();
        // 削除前にシーンアイテムを収集する（イベント用）
        let scene_items_to_remove = removed_input.as_ref().map(|input| {
            self.input_registry
                .find_scene_items_by_input_uuid(&input.input_uuid)
        });
        let response_text = crate::obsws::response::build_remove_input_response(
            request_id,
            Some(request_data),
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if let Some(removed_input) = removed_input {
            let removed_succeeded = self
                .input_registry
                .find_input(Some(&removed_input.input_uuid), None)
                .is_none();
            if removed_succeeded {
                // 入力ライフサイクルの source processor を停止する
                if let Err(e) = self
                    .stop_input_source_processor(&removed_input.input_uuid)
                    .await
                {
                    tracing::warn!(
                        "failed to stop source processor for input {}: {}",
                        removed_input.input_uuid,
                        e.display()
                    );
                }

                // bootstrap 用の差分イベントを送信する
                let _ = self
                    .bootstrap_event_tx
                    .send(BootstrapInputEvent::InputRemoved {
                        input_uuid: removed_input.input_uuid.clone(),
                    });

                events.push(TaggedEvent {
                    text: crate::obsws::response::build_input_removed_event(
                        &removed_input.input_name,
                        &removed_input.input_uuid,
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_INPUTS,
                });
                if let Some(scene_items) = scene_items_to_remove {
                    for (scene_name, scene_uuid, scene_item_id) in scene_items {
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_scene_item_removed_event(
                                &scene_name,
                                &scene_uuid,
                                scene_item_id,
                                &removed_input.input_name,
                                &removed_input.input_uuid,
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
                        });
                    }
                }
            }
        }
        self.build_result_from_response(response_text, events)
    }

    fn handle_set_input_settings(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(request_data) = request_data else {
            return self.build_error_result(
                "SetInputSettings",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };
        let requested_input_lookup =
            match crate::obsws::response::parse_input_lookup_fields_for_session(
                request_data.value(),
            ) {
                Ok(fields) => Some(fields),
                Err(error) => {
                    return self.build_parse_error_result("SetInputSettings", request_id, &error);
                }
            };
        let execution = crate::obsws::response::execute_set_input_settings(
            request_id,
            Some(request_data),
            &mut self.input_registry,
        );
        let response_text = execution.response_text;
        let mut events = Vec::new();
        if execution.request_succeeded
            && let Some((input_uuid, input_name)) = requested_input_lookup
            && let Some(updated_input) = self
                .input_registry
                .find_input(input_uuid.as_deref(), input_name.as_deref())
                .cloned()
        {
            let event = TaggedEvent {
                text: crate::obsws::response::build_input_settings_changed_event(
                    &updated_input.input_name,
                    &updated_input.input_uuid,
                    &updated_input.input.settings,
                ),
                subscription_flag: OBSWS_EVENT_SUB_INPUTS,
            };
            // p2p_session にも通知する（chroma key 動的更新用）
            let _ = self.obsws_event_tx.send(event.clone());
            events.push(event);
        }
        self.build_result_from_response(response_text, events)
    }

    async fn handle_set_input_mute(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::build_set_input_mute_response(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if execution.request_succeeded
            && let Some(entry) = self.input_registry.find_input(
                execution.input_uuid.as_deref(),
                execution.input_name.as_deref(),
            )
        {
            self.notify_audio_mixer_mute_volume(
                &entry.input_uuid,
                entry.input.input_muted,
                entry.input.input_volume_mul,
            )
            .await;
            events.push(TaggedEvent {
                text: crate::obsws::response::build_input_mute_state_changed_event(
                    &entry.input_name,
                    &entry.input_uuid,
                    entry.input.input_muted,
                ),
                subscription_flag: OBSWS_EVENT_SUB_INPUTS,
            });
        }
        self.build_result_from_response(execution.response_text, events)
    }

    async fn handle_toggle_input_mute(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::build_toggle_input_mute_response(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if execution.request_succeeded
            && let Some(entry) = self.input_registry.find_input(
                execution.input_uuid.as_deref(),
                execution.input_name.as_deref(),
            )
        {
            self.notify_audio_mixer_mute_volume(
                &entry.input_uuid,
                entry.input.input_muted,
                entry.input.input_volume_mul,
            )
            .await;
            events.push(TaggedEvent {
                text: crate::obsws::response::build_input_mute_state_changed_event(
                    &entry.input_name,
                    &entry.input_uuid,
                    entry.input.input_muted,
                ),
                subscription_flag: OBSWS_EVENT_SUB_INPUTS,
            });
        }
        self.build_result_from_response(execution.response_text, events)
    }

    async fn handle_set_input_volume(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::build_set_input_volume_response(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if execution.request_succeeded
            && let Some(entry) = self.input_registry.find_input(
                execution.input_uuid.as_deref(),
                execution.input_name.as_deref(),
            )
        {
            self.notify_audio_mixer_mute_volume(
                &entry.input_uuid,
                entry.input.input_muted,
                entry.input.input_volume_mul,
            )
            .await;
            events.push(TaggedEvent {
                text: crate::obsws::response::build_input_volume_changed_event(
                    &entry.input_name,
                    &entry.input_uuid,
                    entry.input.input_volume_db(),
                    entry.input.input_volume_mul.get(),
                ),
                subscription_flag: OBSWS_EVENT_SUB_INPUTS,
            });
        }
        self.build_result_from_response(execution.response_text, events)
    }

    /// audio mixer に入力のミュート・音量設定を通知する
    async fn notify_audio_mixer_mute_volume(
        &self,
        input_uuid: &str,
        muted: bool,
        volume_mul: crate::types::NonNegFiniteF64,
    ) {
        let Some(source_state) = self.input_source_processors.get(input_uuid) else {
            return;
        };
        let Some(audio_track_id) = &source_state.audio_track_id else {
            return;
        };
        let Some(pipeline_handle) = &self.pipeline_handle else {
            return;
        };
        if let Err(e) = crate::mixer::audio::set_track_mute_volume(
            pipeline_handle,
            &self.program_output.audio_mixer_processor_id,
            audio_track_id.clone(),
            muted,
            volume_mul,
        )
        .await
        {
            tracing::warn!("failed to notify audio mixer mute/volume: {}", e.display());
        }
    }

    fn handle_set_input_name(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(request_data) = request_data else {
            return self.build_error_result(
                "SetInputName",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };
        let requested_input_lookup =
            match crate::obsws::response::parse_input_lookup_fields_for_session(
                request_data.value(),
            ) {
                Ok(fields) => Some(fields),
                Err(error) => {
                    return self.build_parse_error_result("SetInputName", request_id, &error);
                }
            };
        let old_input = requested_input_lookup
            .as_ref()
            .and_then(|(input_uuid, input_name)| {
                self.input_registry
                    .find_input(input_uuid.as_deref(), input_name.as_deref())
                    .cloned()
            });
        let response_text = crate::obsws::response::build_set_input_name_response(
            request_id,
            Some(request_data),
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if let Some(old_input) = old_input
            && let Some(updated_input) = self
                .input_registry
                .find_input(Some(&old_input.input_uuid), None)
                .cloned()
            && old_input.input_name != updated_input.input_name
        {
            let event = TaggedEvent {
                text: crate::obsws::response::build_input_name_changed_event(
                    &updated_input.input_name,
                    &old_input.input_name,
                    &updated_input.input_uuid,
                ),
                subscription_flag: OBSWS_EVENT_SUB_INPUTS,
            };
            // p2p_session にも通知する（attached_input_name の追従用）
            let _ = self.obsws_event_tx.send(event.clone());
            events.push(event);
        }
        self.build_result_from_response(response_text, events)
    }

    // -----------------------------------------------------------------------
    // SceneItem 系ハンドラ
    // -----------------------------------------------------------------------

    fn handle_create_scene_item(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::execute_create_scene_item(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let response_text = execution.response_text;
        let mut events = Vec::new();
        if let Some(created_scene_item) = execution.created {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_created_event(
                    &created_scene_item.scene_name,
                    &created_scene_item.scene_uuid,
                    created_scene_item.scene_item.scene_item_id,
                    &created_scene_item.scene_item.source_name,
                    &created_scene_item.scene_item.source_uuid,
                    created_scene_item.scene_item.scene_item_index,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    fn handle_remove_scene_item(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(request_data) = request_data else {
            return self.build_error_result(
                "RemoveSceneItem",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };
        let (scene_name, scene_uuid) =
            match crate::obsws::response::parse_scene_lookup_fields_for_session(
                request_data.value(),
                "sceneName",
                "sceneUuid",
            ) {
                Ok(fields) => fields,
                Err(error) => {
                    return self.build_parse_error_result("RemoveSceneItem", request_id, &error);
                }
            };
        let scene_item_id = match crate::obsws::response::parse_required_i64_field_for_session(
            request_data.value(),
            "sceneItemId",
        ) {
            Ok(value) => value,
            Err(error) => {
                return self.build_parse_error_result("RemoveSceneItem", request_id, &error);
            }
        };
        let target_fields = self
            .input_registry
            .resolve_scene_name(scene_name.as_deref(), scene_uuid.as_deref())
            .map(|scene_name| {
                let scene_uuid = self
                    .input_registry
                    .get_scene_uuid(&scene_name)
                    .unwrap_or_default();
                (scene_name, scene_uuid, scene_item_id)
            });
        let removed_scene_item =
            target_fields
                .as_ref()
                .and_then(|(scene_name, _, scene_item_id)| {
                    let (source_name, source_uuid) = self
                        .input_registry
                        .get_scene_item_source(scene_name, *scene_item_id)
                        .ok()?;
                    Some((source_name, source_uuid))
                });
        let scene_items_before = target_fields.as_ref().and_then(|(scene_name, _, _)| {
            self.input_registry
                .list_scene_items(scene_name)
                .ok()
                .map(|scene_items| {
                    scene_items
                        .iter()
                        .map(|si| (si.scene_item_id, si.scene_item_index))
                        .collect::<Vec<_>>()
                })
        });
        let response_text = crate::obsws::response::build_remove_scene_item_response(
            request_id,
            Some(request_data),
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if let Some((scene_name, scene_uuid, scene_item_id)) = target_fields
            && let Some((source_name, source_uuid)) = removed_scene_item
        {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_removed_event(
                    &scene_name,
                    &scene_uuid,
                    scene_item_id,
                    &source_name,
                    &source_uuid,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
            });
            let scene_items_after = self
                .input_registry
                .list_scene_items(&scene_name)
                .unwrap_or_default()
                .iter()
                .map(
                    |si| crate::obsws::input_registry::ObswsSceneItemIndexEntry {
                        scene_item_id: si.scene_item_id,
                        scene_item_index: si.scene_item_index,
                    },
                )
                .collect::<Vec<_>>();
            let scene_items_after_simple = scene_items_after
                .iter()
                .map(|si| (si.scene_item_id, si.scene_item_index))
                .collect::<Vec<_>>();
            if let Some(scene_items_before) = scene_items_before {
                let still_present_before = scene_items_before
                    .into_iter()
                    .filter(|(id, _)| {
                        scene_items_after_simple
                            .iter()
                            .any(|(after_id, _)| after_id == id)
                    })
                    .collect::<Vec<_>>();
                if still_present_before != scene_items_after_simple {
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_scene_item_list_reindexed_event(
                            &scene_name,
                            &scene_uuid,
                            &scene_items_after,
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
                    });
                }
            }
        }
        self.build_result_from_response(response_text, events)
    }

    fn handle_duplicate_scene_item(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::execute_duplicate_scene_item(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let response_text = execution.response_text;
        let mut events = Vec::new();
        if let Some(duplicated) = execution.duplicated {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_created_event(
                    &duplicated.scene_name,
                    &duplicated.scene_uuid,
                    duplicated.scene_item.scene_item_id,
                    &duplicated.scene_item.source_name,
                    &duplicated.scene_item.source_uuid,
                    duplicated.scene_item.scene_item_index,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    fn handle_set_scene_item_enabled(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(request_data) = request_data else {
            return self.build_error_result(
                "SetSceneItemEnabled",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };
        let requested_fields =
            match crate::obsws::response::parse_set_scene_item_enabled_fields_for_session(
                request_data.value(),
            ) {
                Ok(fields) => Some(fields),
                Err(error) => {
                    return self.build_parse_error_result(
                        "SetSceneItemEnabled",
                        request_id,
                        &error,
                    );
                }
            };
        let previous_enabled =
            requested_fields
                .as_ref()
                .and_then(|(scene_name, scene_uuid, scene_item_id, _)| {
                    let resolved_name = self
                        .input_registry
                        .resolve_scene_name(scene_name.as_deref(), scene_uuid.as_deref())?;
                    self.input_registry
                        .get_scene_item_enabled(&resolved_name, *scene_item_id)
                        .ok()
                });
        let response_text = crate::obsws::response::build_set_scene_item_enabled_response(
            request_id,
            Some(request_data),
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if let Some((scene_name, scene_uuid, scene_item_id, scene_item_enabled)) = requested_fields
            && let Some(prev) = previous_enabled
            && prev != scene_item_enabled
        {
            let resolved_scene_name = self
                .input_registry
                .resolve_scene_name(scene_name.as_deref(), scene_uuid.as_deref())
                .unwrap_or_default();
            let resolved_scene_uuid = self
                .input_registry
                .get_scene_uuid(&resolved_scene_name)
                .unwrap_or_default();
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_enable_state_changed_event(
                    &resolved_scene_name,
                    &resolved_scene_uuid,
                    scene_item_id,
                    scene_item_enabled,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    fn handle_set_scene_item_locked(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::execute_set_scene_item_locked(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let response_text = execution.response_text;
        let mut events = Vec::new();
        if let Some(ctx) = execution.event_context
            && ctx.set_result.changed
        {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_lock_state_changed_event(
                    &ctx.scene_name,
                    &ctx.scene_uuid,
                    ctx.scene_item_id,
                    ctx.scene_item_locked,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    fn handle_set_scene_item_index(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::execute_set_scene_item_index(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let response_text = execution.response_text;
        let mut events = Vec::new();
        if let Some(ctx) = execution.event_context {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_list_reindexed_event(
                    &ctx.scene_name,
                    &ctx.scene_uuid,
                    &ctx.set_result.scene_items,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    fn handle_set_scene_item_blend_mode(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let response_text = crate::obsws::response::build_set_scene_item_blend_mode_response(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        self.build_result_from_response(response_text, Vec::new())
    }

    fn handle_set_scene_item_transform(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::execute_set_scene_item_transform(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let response_text = execution.response_text;
        let mut events = Vec::new();
        if let Some(ctx) = execution.event_context
            && ctx.set_result.changed
        {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_transform_changed_event(
                    &ctx.scene_name,
                    &ctx.scene_uuid,
                    ctx.scene_item_id,
                    &ctx.set_result.scene_item_transform,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    // -----------------------------------------------------------------------
    // Output 系ハンドラ
    // -----------------------------------------------------------------------

    async fn handle_start_stream_request(&mut self, request_id: &str) -> CommandResult {
        let outcome = self.handle_start_stream("StartStream", request_id).await;
        let mut events = Vec::new();
        if outcome.success {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_stream_state_changed_event(
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STARTING",
                ),
                subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
            });
            events.push(TaggedEvent {
                text: crate::obsws::response::build_stream_state_changed_event(
                    true,
                    "OBS_WEBSOCKET_OUTPUT_STARTED",
                ),
                subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
            });
        }
        self.build_result_from_response(outcome.response_text, events)
    }

    async fn handle_stop_stream_request(&mut self, request_id: &str) -> CommandResult {
        let outcome = self.handle_stop_stream("StopStream", request_id).await;
        let mut events = Vec::new();
        if outcome.success {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_stream_state_changed_event(
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STOPPING",
                ),
                subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
            });
            events.push(TaggedEvent {
                text: crate::obsws::response::build_stream_state_changed_event(
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STOPPED",
                ),
                subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
            });
        }
        self.build_result_from_response(outcome.response_text, events)
    }

    async fn handle_toggle_stream_request(&mut self, request_id: &str) -> CommandResult {
        let was_active = self.input_registry.is_stream_active();
        let outcome = if was_active {
            self.handle_stop_stream("ToggleStream", request_id).await
        } else {
            self.handle_start_stream("ToggleStream", request_id).await
        };
        let mut events = Vec::new();
        if outcome.success {
            if was_active {
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_stream_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STOPPING",
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                });
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_stream_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STOPPED",
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                });
            } else {
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_stream_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STARTING",
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                });
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_stream_state_changed_event(
                        true,
                        "OBS_WEBSOCKET_OUTPUT_STARTED",
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                });
            }
        }
        let response_text = if outcome.success {
            crate::obsws::response::build_toggle_stream_response(request_id, !was_active)
        } else {
            outcome.response_text
        };
        self.build_result_from_response(response_text, events)
    }

    async fn handle_start_record_request(&mut self, request_id: &str) -> CommandResult {
        let outcome = self.handle_start_record("StartRecord", request_id).await;
        let mut events = Vec::new();
        if outcome.success {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_record_state_changed_event(
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STARTING",
                    None,
                ),
                subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
            });
            events.push(TaggedEvent {
                text: crate::obsws::response::build_record_state_changed_event(
                    true,
                    "OBS_WEBSOCKET_OUTPUT_STARTED",
                    outcome.output_path.as_deref(),
                ),
                subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
            });
        }
        self.build_result_from_response(outcome.response_text, events)
    }

    async fn handle_stop_record_request(&mut self, request_id: &str) -> CommandResult {
        let outcome = self.handle_stop_record("StopRecord", request_id).await;
        let mut events = Vec::new();
        if outcome.success {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_record_state_changed_event(
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STOPPING",
                    None,
                ),
                subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
            });
            events.push(TaggedEvent {
                text: crate::obsws::response::build_record_state_changed_event(
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STOPPED",
                    outcome.output_path.as_deref(),
                ),
                subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
            });
        }
        self.build_result_from_response(outcome.response_text, events)
    }

    async fn handle_toggle_record_request(&mut self, request_id: &str) -> CommandResult {
        let was_active = self.input_registry.is_record_active();
        let outcome = if was_active {
            self.handle_stop_record("ToggleRecord", request_id).await
        } else {
            self.handle_start_record("ToggleRecord", request_id).await
        };
        let mut events = Vec::new();
        if outcome.success {
            if was_active {
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_record_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STOPPING",
                        None,
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                });
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_record_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STOPPED",
                        outcome.output_path.as_deref(),
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                });
            } else {
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_record_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STARTING",
                        None,
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                });
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_record_state_changed_event(
                        true,
                        "OBS_WEBSOCKET_OUTPUT_STARTED",
                        outcome.output_path.as_deref(),
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                });
            }
        }
        let response_text = if outcome.success {
            crate::obsws::response::build_toggle_record_response(request_id, !was_active)
        } else {
            outcome.response_text
        };
        self.build_result_from_response(response_text, events)
    }

    async fn handle_start_output_request(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(output_name) = parse_required_non_empty_string_field(request_data, "outputName")
        else {
            return self.build_error_result(
                "StartOutput",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required outputName field",
            );
        };
        let (outcome, events) = match output_name.as_str() {
            "stream" => {
                let outcome = self.handle_start_stream("StartOutput", request_id).await;
                let mut events = Vec::new();
                if outcome.success {
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_stream_state_changed_event(
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STARTING",
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                    });
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_stream_state_changed_event(
                            true,
                            "OBS_WEBSOCKET_OUTPUT_STARTED",
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                    });
                }
                (outcome, events)
            }
            "record" => {
                let outcome = self.handle_start_record("StartOutput", request_id).await;
                let mut events = Vec::new();
                if outcome.success {
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_record_state_changed_event(
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STARTING",
                            None,
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                    });
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_record_state_changed_event(
                            true,
                            "OBS_WEBSOCKET_OUTPUT_STARTED",
                            outcome.output_path.as_deref(),
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                    });
                }
                (outcome, events)
            }
            "rtmp_outbound" => {
                let outcome = self
                    .handle_start_rtmp_outbound("StartOutput", request_id)
                    .await;
                (outcome, Vec::new())
            }
            "sora" => {
                let outcome = self
                    .handle_start_sora_publisher("StartOutput", request_id)
                    .await;
                (outcome, Vec::new())
            }
            "hls" => {
                let outcome = self.handle_start_hls("StartOutput", request_id).await;
                (outcome, Vec::new())
            }
            "mpeg_dash" => {
                let outcome = self.handle_start_mpeg_dash("StartOutput", request_id).await;
                (outcome, Vec::new())
            }
            _ => {
                return self.build_error_result(
                    "StartOutput",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Output not found",
                );
            }
        };
        let response_text = if outcome.success {
            crate::obsws::response::build_start_output_response(request_id)
        } else {
            outcome.response_text
        };
        self.build_result_from_response(response_text, events)
    }

    async fn handle_stop_output_request(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(output_name) = parse_required_non_empty_string_field(request_data, "outputName")
        else {
            return self.build_error_result(
                "StopOutput",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required outputName field",
            );
        };
        let (outcome, events) = match output_name.as_str() {
            "stream" => {
                let outcome = self.handle_stop_stream("StopOutput", request_id).await;
                let mut events = Vec::new();
                if outcome.success {
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_stream_state_changed_event(
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STOPPING",
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                    });
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_stream_state_changed_event(
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STOPPED",
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                    });
                }
                (outcome, events)
            }
            "record" => {
                let outcome = self.handle_stop_record("StopOutput", request_id).await;
                let mut events = Vec::new();
                if outcome.success {
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_record_state_changed_event(
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STOPPING",
                            None,
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                    });
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_record_state_changed_event(
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STOPPED",
                            outcome.output_path.as_deref(),
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                    });
                }
                (outcome, events)
            }
            "rtmp_outbound" => {
                let outcome = self
                    .handle_stop_rtmp_outbound("StopOutput", request_id)
                    .await;
                (outcome, Vec::new())
            }
            "sora" => {
                let outcome = self
                    .handle_stop_sora_publisher("StopOutput", request_id)
                    .await;
                (outcome, Vec::new())
            }
            "hls" => {
                let outcome = self.handle_stop_hls("StopOutput", request_id).await;
                (outcome, Vec::new())
            }
            "mpeg_dash" => {
                let outcome = self.handle_stop_mpeg_dash("StopOutput", request_id).await;
                (outcome, Vec::new())
            }
            _ => {
                return self.build_error_result(
                    "StopOutput",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Output not found",
                );
            }
        };
        let response_text = if outcome.success {
            crate::obsws::response::build_stop_output_response(request_id)
        } else {
            outcome.response_text
        };
        self.build_result_from_response(response_text, events)
    }

    async fn handle_toggle_output_request(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(output_name) = parse_required_non_empty_string_field(request_data, "outputName")
        else {
            return self.build_error_result(
                "ToggleOutput",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required outputName field",
            );
        };
        let (outcome, output_active_on_success, events) = match output_name.as_str() {
            "stream" => {
                let was_active = self.input_registry.is_stream_active();
                let outcome = if was_active {
                    self.handle_stop_stream("ToggleOutput", request_id).await
                } else {
                    self.handle_start_stream("ToggleOutput", request_id).await
                };
                let mut events = Vec::new();
                if outcome.success {
                    if was_active {
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_stream_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STOPPING",
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                        });
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_stream_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STOPPED",
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                        });
                    } else {
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_stream_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STARTING",
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                        });
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_stream_state_changed_event(
                                true,
                                "OBS_WEBSOCKET_OUTPUT_STARTED",
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                        });
                    }
                }
                (outcome, !was_active, events)
            }
            "record" => {
                let was_active = self.input_registry.is_record_active();
                let outcome = if was_active {
                    self.handle_stop_record("ToggleOutput", request_id).await
                } else {
                    self.handle_start_record("ToggleOutput", request_id).await
                };
                let mut events = Vec::new();
                if outcome.success {
                    if was_active {
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_record_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STOPPING",
                                None,
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                        });
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_record_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STOPPED",
                                outcome.output_path.as_deref(),
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                        });
                    } else {
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_record_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STARTING",
                                None,
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                        });
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_record_state_changed_event(
                                true,
                                "OBS_WEBSOCKET_OUTPUT_STARTED",
                                outcome.output_path.as_deref(),
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                        });
                    }
                }
                (outcome, !was_active, events)
            }
            "rtmp_outbound" => {
                let was_active = self.input_registry.is_rtmp_outbound_active();
                let outcome = if was_active {
                    self.handle_stop_rtmp_outbound("ToggleOutput", request_id)
                        .await
                } else {
                    self.handle_start_rtmp_outbound("ToggleOutput", request_id)
                        .await
                };
                (outcome, !was_active, Vec::new())
            }
            "sora" => {
                let was_active = self.input_registry.is_sora_publisher_active();
                let outcome = if was_active {
                    self.handle_stop_sora_publisher("ToggleOutput", request_id)
                        .await
                } else {
                    self.handle_start_sora_publisher("ToggleOutput", request_id)
                        .await
                };
                (outcome, !was_active, Vec::new())
            }
            "hls" => {
                let was_active = self.input_registry.is_hls_active();
                let outcome = if was_active {
                    self.handle_stop_hls("ToggleOutput", request_id).await
                } else {
                    self.handle_start_hls("ToggleOutput", request_id).await
                };
                (outcome, !was_active, Vec::new())
            }
            "mpeg_dash" => {
                let was_active = self.input_registry.is_dash_active();
                let outcome = if was_active {
                    self.handle_stop_mpeg_dash("ToggleOutput", request_id).await
                } else {
                    self.handle_start_mpeg_dash("ToggleOutput", request_id)
                        .await
                };
                (outcome, !was_active, Vec::new())
            }
            _ => {
                return self.build_error_result(
                    "ToggleOutput",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Output not found",
                );
            }
        };
        let response_text = if outcome.success {
            crate::obsws::response::build_toggle_output_response(
                request_id,
                output_active_on_success,
            )
        } else {
            outcome.response_text
        };
        self.build_result_from_response(response_text, events)
    }

    // --- Output 内部操作 ---

    async fn handle_start_stream(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        use crate::obsws::input_registry::{
            ActivateStreamError, ObswsRecordTrackRun, ObswsStreamRun,
        };
        let stream_service_settings = self.input_registry.stream_service_settings().clone();
        // `stream` は OBS 互換の主配信 Output として扱い、現状は RTMP 起動経路に限定する。
        // `sora` は stream service の差し替えではなく別 Output として扱うため、
        // ここでは `rtmp_custom` 以外を受け付けない。
        if stream_service_settings.stream_service_type != "rtmp_custom" {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Unsupported streamServiceType field",
                ),
            );
        }
        let Some(output_url) = stream_service_settings.server else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Missing streamServiceSettings.server field",
                ),
            );
        };
        let run_id = match self.input_registry.next_stream_run_id() {
            Ok(run_id) => run_id,
            Err(_) => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        "Stream run ID overflow",
                    ),
                );
            }
        };
        let video = ObswsRecordTrackRun::new(
            "stream",
            run_id,
            "video",
            &self.program_output.video_track_id,
        );
        let audio = ObswsRecordTrackRun::new(
            "stream",
            run_id,
            "audio",
            &self.program_output.audio_track_id,
        );
        let run = ObswsStreamRun {
            video,
            audio,
            publisher_processor_id: crate::ProcessorId::new(format!(
                "obsws:stream:{run_id}:rtmp_publisher"
            )),
        };
        if let Err(ActivateStreamError::AlreadyActive) =
            self.input_registry.activate_stream(run.clone())
        {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_STREAM_RUNNING,
                    "Stream is already active",
                ),
            );
        }
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            self.input_registry.deactivate_stream();
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    "Pipeline is not initialized",
                ),
            );
        };
        let frame_rate = self.input_registry.frame_rate();
        if let Err(e) = start_stream_processors(
            pipeline_handle,
            &output_url,
            stream_service_settings.key.as_deref(),
            &run,
            frame_rate,
        )
        .await
        {
            self.input_registry.deactivate_stream();
            let _ = stop_processors_staged_stream(pipeline_handle, &run).await;
            let error_comment = format!("Failed to start stream: {}", e.display());
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &error_comment,
                ),
            );
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_start_stream_response(request_id),
            None,
        )
    }

    async fn handle_stop_stream(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        let run = match self.input_registry.stream_run() {
            Some(run) => run.clone(),
            None => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_STREAM_NOT_RUNNING,
                        "Stream is not active",
                    ),
                );
            }
        };
        if let Some(pipeline_handle) = self.pipeline_handle.as_ref()
            && let Err(e) = stop_processors_staged_stream(pipeline_handle, &run).await
        {
            let error_comment = format!("Failed to stop stream: {}", e.display());
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &error_comment,
                ),
            );
        }
        self.input_registry.deactivate_stream();
        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_stream_response(request_id),
            None,
        )
    }

    async fn handle_start_record(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        use crate::obsws::input_registry::{
            ActivateRecordError, ObswsRecordRun, ObswsRecordTrackRun,
        };
        use std::time::{SystemTime, UNIX_EPOCH};
        let run_id = match self.input_registry.next_record_run_id() {
            Ok(run_id) => run_id,
            Err(_) => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        "Record run ID overflow",
                    ),
                );
            }
        };
        let video = ObswsRecordTrackRun::new(
            "record",
            run_id,
            "video",
            &self.program_output.video_track_id,
        );
        let audio = ObswsRecordTrackRun::new(
            "record",
            run_id,
            "audio",
            &self.program_output.audio_track_id,
        );
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis();
        let output_path = self
            .input_registry
            .record_directory()
            .join(format!("obsws-record-{timestamp}.mp4"));
        let run = ObswsRecordRun {
            video,
            audio,
            writer_processor_id: crate::ProcessorId::new(format!(
                "obsws:record:{run_id}:mp4_writer"
            )),
            output_path: output_path.clone(),
        };
        if let Err(ActivateRecordError::AlreadyActive) =
            self.input_registry.activate_record(run.clone())
        {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_RUNNING,
                    "Record is already active",
                ),
            );
        }
        if let Some(parent) = output_path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            self.input_registry.deactivate_record();
            let error_comment = format!("Failed to create record directory: {e}");
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &error_comment,
                ),
            );
        }
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            self.input_registry.deactivate_record();
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    "Pipeline is not initialized",
                ),
            );
        };
        let frame_rate = self.input_registry.frame_rate();
        if let Err(e) =
            start_record_processors(pipeline_handle, &output_path, &run, frame_rate).await
        {
            self.input_registry.deactivate_record();
            let _ = stop_processors_staged_record(pipeline_handle, &run).await;
            let error_comment = format!("Failed to start record: {}", e.display());
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &error_comment,
                ),
            );
        }
        let output_path_str = output_path.display().to_string();
        OutputOperationOutcome::success(
            crate::obsws::response::build_start_record_response(request_id),
            Some(output_path_str),
        )
    }

    async fn handle_stop_record(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        let run = match self.input_registry.record_run() {
            Some(run) => run.clone(),
            None => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                        "Record is not active",
                    ),
                );
            }
        };
        let output_path = run.output_path.display().to_string();
        if let Some(pipeline_handle) = self.pipeline_handle.as_ref()
            && let Err(e) = stop_processors_staged_record(pipeline_handle, &run).await
        {
            // プロセッサ停止に失敗してもレコード状態は解除する。
            // MP4 ファイルの finalize を優先し、クライアントには成功を返す。
            tracing::warn!("failed to stop record processors: {}", e.display());
        }
        self.input_registry.deactivate_record();
        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_record_response(request_id, &output_path),
            Some(output_path),
        )
    }

    async fn handle_start_hls(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        use crate::obsws::input_registry::{
            ActivateHlsError, HlsDestination, ObswsHlsRun, ObswsHlsVariantRun, ObswsRecordTrackRun,
        };
        let hls_settings = self.input_registry.hls_settings().clone();
        let Some(ref destination) = hls_settings.destination else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Missing outputSettings.destination field",
                ),
            );
        };
        if hls_settings.variants.is_empty() {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "variants must not be empty",
                ),
            );
        }
        let run_id = match self.input_registry.next_hls_run_id() {
            Ok(run_id) => run_id,
            Err(_) => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        "HLS run ID overflow",
                    ),
                );
            }
        };
        let program_output = ObswsProgramOutputContext {
            video_track_id: self.program_output.video_track_id.clone(),
            audio_track_id: self.program_output.audio_track_id.clone(),
            canvas_width: self.input_registry.canvas_width(),
            canvas_height: self.input_registry.canvas_height(),
            frame_rate: self.input_registry.frame_rate(),
        };
        let is_abr = hls_settings.variants.len() > 1;
        let variant_runs: Vec<ObswsHlsVariantRun> = hls_settings
            .variants
            .iter()
            .enumerate()
            .map(|(i, variant)| {
                let variant_label = format!("v{i}");
                let video = ObswsRecordTrackRun::new(
                    "hls",
                    run_id,
                    &format!("{variant_label}_video"),
                    &program_output.video_track_id,
                );
                let audio = ObswsRecordTrackRun::new(
                    "hls",
                    run_id,
                    &format!("{variant_label}_audio"),
                    &program_output.audio_track_id,
                );
                // variant ごとの fps 調整が必要になった場合は、この後段に映像整形を追加する。
                let needs_scaler = variant.width.zip(variant.height).is_some_and(|(w, h)| {
                    w != program_output.canvas_width || h != program_output.canvas_height
                });
                let scaler_processor_id = if needs_scaler {
                    Some(crate::ProcessorId::new(format!(
                        "obsws:hls:{run_id}:{variant_label}_scaler"
                    )))
                } else {
                    None
                };
                let scaled_track_id = if needs_scaler {
                    Some(crate::TrackId::new(format!(
                        "obsws:hls:{run_id}:{variant_label}_scaled_video"
                    )))
                } else {
                    None
                };
                let writer_processor_id = crate::ProcessorId::new(format!(
                    "obsws:hls:{run_id}:{variant_label}_hls_writer"
                ));
                let variant_path = if is_abr {
                    destination.variant_path(i)
                } else {
                    match destination {
                        HlsDestination::Filesystem { directory } => directory.clone(),
                        HlsDestination::S3 { prefix, .. } => prefix.clone(),
                    }
                };
                ObswsHlsVariantRun {
                    video,
                    audio,
                    scaler_processor_id,
                    scaled_track_id,
                    writer_processor_id,
                    variant_path,
                }
            })
            .collect();
        let run = ObswsHlsRun {
            destination: destination.clone(),
            variant_runs,
        };
        if let Err(ActivateHlsError::AlreadyActive) = self.input_registry.activate_hls(run.clone())
        {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_RUNNING,
                    "HLS is already active",
                ),
            );
        }
        // filesystem の場合のみ出力ディレクトリを作成する
        if let HlsDestination::Filesystem { directory } = destination
            && let Err(e) = std::fs::create_dir_all(directory)
        {
            self.input_registry.deactivate_hls();
            let error_comment = format!("Failed to create HLS output directory: {e}");
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &error_comment,
                ),
            );
        }
        // S3 + lifetimeDays 指定時はバケットに lifecycle ルールを設定する
        if let HlsDestination::S3 {
            bucket,
            prefix,
            region,
            endpoint,
            use_path_style,
            access_key_id,
            secret_access_key,
            session_token,
            lifetime_days: Some(days),
        } = destination
        {
            let s3_client = build_s3_client(
                region,
                access_key_id,
                secret_access_key,
                session_token.as_deref(),
                endpoint.as_deref(),
                *use_path_style,
            );
            match s3_client {
                Ok(client) => {
                    // prefix スコープの expiration ルールを設定する
                    let rule_id = format!("hisui-hls-{}", prefix.replace('/', "-"));
                    let rule = shiguredo_s3::types::LifecycleRule {
                        id: Some(rule_id),
                        status: shiguredo_s3::types::ExpirationStatus::Enabled,
                        filter: Some(shiguredo_s3::types::LifecycleRuleFilter {
                            prefix: Some(prefix.clone()),
                            tag: None,
                            object_size_greater_than: None,
                            object_size_less_than: None,
                            and: None,
                        }),
                        expiration: Some(shiguredo_s3::types::LifecycleExpiration {
                            days: Some(*days as i32),
                            date: None,
                            expired_object_delete_marker: None,
                        }),
                        transitions: None,
                        noncurrent_version_transitions: None,
                        noncurrent_version_expiration: None,
                        abort_incomplete_multipart_upload: None,
                    };
                    let request = client
                        .client()
                        .put_bucket_lifecycle_configuration()
                        .bucket(bucket)
                        .rule(rule)
                        .build_request();
                    match request {
                        Ok(req) => match client.execute(&req).await {
                            Ok(response) if !response.is_success() => {
                                tracing::warn!(
                                    "PutBucketLifecycleConfiguration failed: status={}",
                                    response.status_code
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "failed to set S3 lifecycle configuration: {}",
                                    e.display()
                                );
                            }
                            _ => {}
                        },
                        Err(e) => {
                            tracing::warn!(
                                "failed to build PutBucketLifecycleConfiguration request: {e}"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "failed to build S3 client for lifecycle configuration: {}",
                        e.display()
                    );
                }
            }
        }
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            self.input_registry.deactivate_hls();
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    "Pipeline is not initialized",
                ),
            );
        };
        match start_hls_processors(pipeline_handle, &program_output, &run, &hls_settings).await {
            Ok(master_playlist_task) => {
                self.input_registry.hls_runtime.master_playlist_task = master_playlist_task;
            }
            Err(e) => {
                self.input_registry.deactivate_hls();
                let _ = stop_processors_staged_hls(pipeline_handle, &run).await;
                let error_comment = format!("Failed to start HLS: {}", e.display());
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        &error_comment,
                    ),
                );
            }
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_start_output_response(request_id),
            None,
        )
    }

    async fn handle_stop_hls(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        // deactivate_hls() でアクティブチェックと状態解除を兼ねる（不要な clone を避ける）
        let run = match self.input_registry.deactivate_hls() {
            Some(run) => run,
            None => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                        "HLS is not active",
                    ),
                );
            }
        };
        if let Some(pipeline_handle) = self.pipeline_handle.as_ref()
            && let Err(e) = stop_processors_staged_hls(pipeline_handle, &run).await
        {
            tracing::warn!("failed to stop HLS processors: {}", e.display());
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_output_response(request_id),
            None,
        )
    }

    async fn handle_start_mpeg_dash(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        use crate::obsws::input_registry::{
            ActivateDashError, DashDestination, ObswsDashRun, ObswsDashVariantRun,
            ObswsRecordTrackRun,
        };
        let dash_settings = self.input_registry.dash_settings().clone();
        let Some(ref destination) = dash_settings.destination else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Missing outputSettings.destination field",
                ),
            );
        };
        if dash_settings.variants.is_empty() {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "variants must not be empty",
                ),
            );
        }
        let run_id = match self.input_registry.next_dash_run_id() {
            Ok(run_id) => run_id,
            Err(_) => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        "MPEG-DASH run ID overflow",
                    ),
                );
            }
        };
        let program_output = ObswsProgramOutputContext {
            video_track_id: self.program_output.video_track_id.clone(),
            audio_track_id: self.program_output.audio_track_id.clone(),
            canvas_width: self.input_registry.canvas_width(),
            canvas_height: self.input_registry.canvas_height(),
            frame_rate: self.input_registry.frame_rate(),
        };
        let is_abr = dash_settings.variants.len() > 1;
        let variant_runs: Vec<ObswsDashVariantRun> = dash_settings
            .variants
            .iter()
            .enumerate()
            .map(|(i, variant)| {
                let variant_label = format!("v{i}");
                let video = ObswsRecordTrackRun::new(
                    "mpeg_dash",
                    run_id,
                    &format!("{variant_label}_video"),
                    &program_output.video_track_id,
                );
                let audio = ObswsRecordTrackRun::new(
                    "mpeg_dash",
                    run_id,
                    &format!("{variant_label}_audio"),
                    &program_output.audio_track_id,
                );
                // variant ごとの fps 調整が必要になった場合は、この後段に映像整形を追加する。
                let needs_scaler = variant.width.zip(variant.height).is_some_and(|(w, h)| {
                    w != program_output.canvas_width || h != program_output.canvas_height
                });
                let scaler_processor_id = if needs_scaler {
                    Some(crate::ProcessorId::new(format!(
                        "obsws:mpeg_dash:{run_id}:{variant_label}_scaler"
                    )))
                } else {
                    None
                };
                let scaled_track_id = if needs_scaler {
                    Some(crate::TrackId::new(format!(
                        "obsws:mpeg_dash:{run_id}:{variant_label}_scaled_video"
                    )))
                } else {
                    None
                };
                let writer_processor_id = crate::ProcessorId::new(format!(
                    "obsws:mpeg_dash:{run_id}:{variant_label}_dash_writer"
                ));
                let variant_path = if is_abr {
                    destination.variant_path(i)
                } else {
                    match destination {
                        DashDestination::Filesystem { directory } => directory.clone(),
                        DashDestination::S3 { prefix, .. } => prefix.clone(),
                    }
                };
                ObswsDashVariantRun {
                    video,
                    audio,
                    scaler_processor_id,
                    scaled_track_id,
                    writer_processor_id,
                    variant_path,
                }
            })
            .collect();
        let run = ObswsDashRun {
            destination: destination.clone(),
            variant_runs,
        };
        if let Err(ActivateDashError::AlreadyActive) =
            self.input_registry.activate_dash(run.clone())
        {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_RUNNING,
                    "MPEG-DASH is already active",
                ),
            );
        }
        // filesystem の場合のみ出力ディレクトリを作成する
        if let DashDestination::Filesystem { directory } = destination
            && let Err(e) = std::fs::create_dir_all(directory)
        {
            self.input_registry.deactivate_dash();
            let error_comment = format!("Failed to create MPEG-DASH output directory: {e}");
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &error_comment,
                ),
            );
        }
        // S3 + lifetimeDays 指定時はバケットに lifecycle ルールを設定する
        if let DashDestination::S3 {
            bucket,
            prefix,
            region,
            endpoint,
            use_path_style,
            access_key_id,
            secret_access_key,
            session_token,
            lifetime_days: Some(days),
        } = destination
        {
            let s3_client = build_s3_client(
                region,
                access_key_id,
                secret_access_key,
                session_token.as_deref(),
                endpoint.as_deref(),
                *use_path_style,
            );
            match s3_client {
                Ok(client) => {
                    let rule_id = format!("hisui-dash-{}", prefix.replace('/', "-"));
                    let rule = shiguredo_s3::types::LifecycleRule {
                        id: Some(rule_id),
                        status: shiguredo_s3::types::ExpirationStatus::Enabled,
                        filter: Some(shiguredo_s3::types::LifecycleRuleFilter {
                            prefix: Some(prefix.clone()),
                            tag: None,
                            object_size_greater_than: None,
                            object_size_less_than: None,
                            and: None,
                        }),
                        expiration: Some(shiguredo_s3::types::LifecycleExpiration {
                            days: Some(*days as i32),
                            date: None,
                            expired_object_delete_marker: None,
                        }),
                        transitions: None,
                        noncurrent_version_transitions: None,
                        noncurrent_version_expiration: None,
                        abort_incomplete_multipart_upload: None,
                    };
                    let request = client
                        .client()
                        .put_bucket_lifecycle_configuration()
                        .bucket(bucket)
                        .rule(rule)
                        .build_request();
                    match request {
                        Ok(req) => match client.execute(&req).await {
                            Ok(response) if !response.is_success() => {
                                tracing::warn!(
                                    "PutBucketLifecycleConfiguration failed: status={}",
                                    response.status_code
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "failed to set S3 lifecycle configuration: {}",
                                    e.display()
                                );
                            }
                            _ => {}
                        },
                        Err(e) => {
                            tracing::warn!(
                                "failed to build PutBucketLifecycleConfiguration request: {e}"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "failed to build S3 client for lifecycle configuration: {}",
                        e.display()
                    );
                }
            }
        }
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            self.input_registry.deactivate_dash();
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    "Pipeline is not initialized",
                ),
            );
        };
        match start_dash_processors(pipeline_handle, &program_output, &run, &dash_settings).await {
            Ok(combined_mpd_task) => {
                self.input_registry.dash_runtime.combined_mpd_task = combined_mpd_task;
            }
            Err(e) => {
                self.input_registry.deactivate_dash();
                let _ = stop_processors_staged_dash(pipeline_handle, &run).await;
                let error_comment = format!("Failed to start MPEG-DASH: {}", e.display());
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        &error_comment,
                    ),
                );
            }
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_start_output_response(request_id),
            None,
        )
    }

    async fn handle_stop_mpeg_dash(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        let run = match self.input_registry.deactivate_dash() {
            Some(run) => run,
            None => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                        "MPEG-DASH is not active",
                    ),
                );
            }
        };
        if let Some(pipeline_handle) = self.pipeline_handle.as_ref()
            && let Err(e) = stop_processors_staged_dash(pipeline_handle, &run).await
        {
            tracing::warn!("failed to stop MPEG-DASH processors: {}", e.display());
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_output_response(request_id),
            None,
        )
    }

    async fn handle_start_rtmp_outbound(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        use crate::obsws::input_registry::{
            ActivateRtmpOutboundError, ObswsRecordTrackRun, ObswsRtmpOutboundRun,
        };
        let rtmp_outbound_settings = self.input_registry.rtmp_outbound_settings().clone();
        let Some(output_url) = rtmp_outbound_settings.output_url else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Missing outputSettings.outputUrl field",
                ),
            );
        };
        let run_id = match self.input_registry.next_rtmp_outbound_run_id() {
            Ok(run_id) => run_id,
            Err(_) => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        "RTMP outbound run ID overflow",
                    ),
                );
            }
        };
        let video = ObswsRecordTrackRun::new(
            "rtmp_outbound",
            run_id,
            "video",
            &self.program_output.video_track_id,
        );
        let audio = ObswsRecordTrackRun::new(
            "rtmp_outbound",
            run_id,
            "audio",
            &self.program_output.audio_track_id,
        );
        let run = ObswsRtmpOutboundRun {
            video,
            audio,
            endpoint_processor_id: crate::ProcessorId::new(format!(
                "obsws:rtmp_outbound:{run_id}:rtmp_outbound_endpoint"
            )),
        };
        if let Err(ActivateRtmpOutboundError::AlreadyActive) =
            self.input_registry.activate_rtmp_outbound(run.clone())
        {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_RUNNING,
                    "RTMP outbound is already active",
                ),
            );
        }
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            self.input_registry.deactivate_rtmp_outbound();
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    "Pipeline is not initialized",
                ),
            );
        };
        let frame_rate = self.input_registry.frame_rate();
        if let Err(e) = start_rtmp_outbound_processors(
            pipeline_handle,
            &output_url,
            rtmp_outbound_settings.stream_name.as_deref(),
            &run,
            frame_rate,
        )
        .await
        {
            self.input_registry.deactivate_rtmp_outbound();
            let _ = stop_processors_staged_rtmp_outbound(pipeline_handle, &run).await;
            let error_comment = format!("Failed to start rtmp_outbound: {}", e.display());
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &error_comment,
                ),
            );
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_start_output_response(request_id),
            None,
        )
    }

    async fn handle_stop_rtmp_outbound(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        let run = match self.input_registry.rtmp_outbound_run() {
            Some(run) => run.clone(),
            None => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                        "RTMP outbound is not active",
                    ),
                );
            }
        };
        if let Some(pipeline_handle) = self.pipeline_handle.as_ref()
            && let Err(e) = stop_processors_staged_rtmp_outbound(pipeline_handle, &run).await
        {
            tracing::warn!("failed to stop rtmp outbound processors: {}", e.display());
        }
        self.input_registry.deactivate_rtmp_outbound();
        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_output_response(request_id),
            None,
        )
    }

    // --- Sora Publisher 操作 ---
    // `sora` は OBS の `stream` を拡張したものではなく、Program 出力の raw frame を
    // `sora-rust-sdk` に直接渡す専用 Output として扱う。
    // 将来的に `stream` を多プロトコル化する余地はあるが、現時点では OBS 互換の
    // 意味を保つため `stream` と `sora` を分離している。

    async fn handle_start_sora_publisher(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        use crate::obsws::input_registry::{ActivateSoraPublisherError, ObswsSoraPublisherRun};
        let sora_settings = self.input_registry.sora_publisher_settings().clone();
        if sora_settings.signaling_urls.is_empty() {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Missing outputSettings.soraSdkSettings.signalingUrls field",
                ),
            );
        }
        let Some(channel_id) = sora_settings.channel_id.clone() else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Missing outputSettings.soraSdkSettings.channelId field",
                ),
            );
        };
        let run_id = match self.input_registry.next_sora_publisher_run_id() {
            Ok(run_id) => run_id,
            Err(_) => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        "Sora publisher run ID overflow",
                    ),
                );
            }
        };
        let publisher_processor_id =
            crate::ProcessorId::new(format!("obsws:sora_publisher:{run_id}:sora_publisher"));
        let run = ObswsSoraPublisherRun {
            publisher_processor_id: publisher_processor_id.clone(),
        };
        if let Err(ActivateSoraPublisherError::AlreadyActive) =
            self.input_registry.activate_sora_publisher(run.clone())
        {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_RUNNING,
                    "Sora publisher is already active",
                ),
            );
        }
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            self.input_registry.deactivate_sora_publisher();
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    "Pipeline is not initialized",
                ),
            );
        };
        let publisher = crate::sora_publisher::SoraPublisher {
            signaling_urls: sora_settings.signaling_urls.clone(),
            channel_id,
            client_id: sora_settings.client_id.clone(),
            bundle_id: sora_settings.bundle_id.clone(),
            metadata: sora_settings.metadata.clone(),
            input_video_track_id: self.program_output.video_track_id.clone(),
            input_audio_track_id: self.program_output.audio_track_id.clone(),
        };
        if let Err(e) = crate::sora_publisher::create_processor(
            pipeline_handle,
            publisher,
            Some(publisher_processor_id),
        )
        .await
        {
            self.input_registry.deactivate_sora_publisher();
            let error_comment = format!("Failed to start sora publisher: {}", e.display());
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &error_comment,
                ),
            );
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_start_output_response(request_id),
            None,
        )
    }

    async fn handle_stop_sora_publisher(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        let run = match self.input_registry.sora_publisher_run() {
            Some(run) => run.clone(),
            None => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                        "Sora publisher is not active",
                    ),
                );
            }
        };
        if let Some(pipeline_handle) = self.pipeline_handle.as_ref()
            && let Err(e) = terminate_and_wait(
                pipeline_handle,
                std::slice::from_ref(&run.publisher_processor_id),
            )
            .await
        {
            tracing::warn!("failed to stop sora publisher processor: {}", e.display());
        }
        self.input_registry.deactivate_sora_publisher();
        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_output_response(request_id),
            None,
        )
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

    /// 入力ライフサイクルの source processor を起動する
    async fn start_input_source_processor(
        &mut self,
        input_entry: &crate::obsws::input_registry::ObswsInputEntry,
    ) -> crate::Result<()> {
        let Some(pipeline_handle) = &self.pipeline_handle else {
            return Ok(());
        };
        let source_plan = crate::obsws::source::build_record_source_plan(
            input_entry,
            crate::obsws::source::ObswsOutputKind::Program,
            0,
            &input_entry.input_uuid,
            self.input_registry.frame_rate(),
        )
        .map_err(|e| crate::Error::new(format!("failed to build source plan: {}", e.message())))?;

        let state = InputSourceState {
            processor_ids: source_plan.source_processor_ids.clone(),
            video_track_id: source_plan.source_video_track_id.clone(),
            audio_track_id: source_plan.source_audio_track_id.clone(),
        };

        crate::obsws::session::output::start_source_processors(pipeline_handle, &mut [source_plan])
            .await?;

        self.input_source_processors
            .insert(input_entry.input_uuid.clone(), state);
        Ok(())
    }

    /// 入力ライフサイクルの source processor を停止する
    async fn stop_input_source_processor(&mut self, input_uuid: &str) -> crate::Result<()> {
        let Some(state) = self.input_source_processors.remove(input_uuid) else {
            return Ok(());
        };
        let Some(pipeline_handle) = &self.pipeline_handle else {
            return Ok(());
        };
        crate::obsws::session::output::stop_source_processors(pipeline_handle, &state.processor_ids)
            .await
    }

    /// 初期入力に対して source processor を一括起動する
    pub async fn start_initial_input_source_processors(&mut self) -> crate::Result<()> {
        let entries: Vec<_> = self
            .input_registry
            .inputs_by_uuid
            .values()
            .cloned()
            .collect();
        for entry in entries {
            if let Err(e) = self.start_input_source_processor(&entry).await {
                tracing::warn!(
                    "failed to start source processor for initial input {}: {}",
                    entry.input_uuid,
                    e.display()
                );
            }
        }
        Ok(())
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
            crate::obsws::source::ObswsOutputKind::Program,
            0,
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

    /// 全入力のミュート・音量を audio mixer に同期する
    async fn sync_all_input_mute_volume(&self) {
        for (input_uuid, source_state) in &self.input_source_processors {
            let Some(ref audio_track_id) = source_state.audio_track_id else {
                continue;
            };
            let Some(entry) = self.input_registry.find_input(Some(input_uuid), None) else {
                continue;
            };
            // デフォルト値（unmuted, 1.0）の場合は通知を省略する
            if !entry.input.input_muted
                && entry.input.input_volume_mul == crate::types::NonNegFiniteF64::ONE
            {
                continue;
            }
            let Some(pipeline_handle) = &self.pipeline_handle else {
                return;
            };
            if let Err(e) = crate::mixer::audio::set_track_mute_volume(
                pipeline_handle,
                &self.program_output.audio_mixer_processor_id,
                audio_track_id.clone(),
                entry.input.input_muted,
                entry.input.input_volume_mul,
            )
            .await
            {
                tracing::warn!(
                    "failed to sync mute/volume for input {input_uuid}: {}",
                    e.display()
                );
            }
        }
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

/// output 操作の結果（成功/失敗 + レスポンス + 出力パス）
struct OutputOperationOutcome {
    response_text: nojson::RawJsonOwned,
    success: bool,
    output_path: Option<String>,
}

impl OutputOperationOutcome {
    fn success(response_text: nojson::RawJsonOwned, output_path: Option<String>) -> Self {
        Self {
            response_text,
            success: true,
            output_path,
        }
    }

    fn failure(response_text: nojson::RawJsonOwned) -> Self {
        Self {
            response_text,
            success: false,
            output_path: None,
        }
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
        // output 設定
        "SetStreamServiceSettings"
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

// -----------------------------------------------------------------------
// Output プロセッサ起動/停止 自由関数
// -----------------------------------------------------------------------

/// ストリーム用プロセッサを起動する: エンコーダー → パブリッシャー
/// program mixer の出力トラックを直接エンコーダーに入力するため、ミキサーとソースの起動は不要。
async fn start_stream_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    output_url: &str,
    stream_key: Option<&str>,
    run: &crate::obsws::input_registry::ObswsStreamRun,
    frame_rate: crate::video::FrameRate,
) -> crate::Result<()> {
    // ビデオエンコーダーを起動する
    crate::encoder::create_video_processor(
        pipeline_handle,
        run.video.source_track_id.clone(),
        run.video.encoded_track_id.clone(),
        crate::types::CodecName::H264,
        std::num::NonZeroUsize::new(2_000_000).unwrap(),
        frame_rate,
        Some(run.video.encoder_processor_id.clone()),
    )
    .await?;
    // オーディオエンコーダーを起動する
    crate::encoder::create_audio_processor(
        pipeline_handle,
        run.audio.source_track_id.clone(),
        run.audio.encoded_track_id.clone(),
        crate::types::CodecName::Aac,
        std::num::NonZeroUsize::new(128_000).unwrap(),
        Some(run.audio.encoder_processor_id.clone()),
    )
    .await?;
    // RTMP パブリッシャーを起動する
    let publisher = crate::rtmp::publisher::RtmpPublisher {
        output_url: output_url.to_owned(),
        stream_name: stream_key.map(|s| s.to_owned()),
        input_audio_track_id: Some(run.audio.encoded_track_id.clone()),
        input_video_track_id: Some(run.video.encoded_track_id.clone()),
        options: Default::default(),
    };
    crate::rtmp::publisher::create_processor(
        pipeline_handle,
        publisher,
        Some(run.publisher_processor_id.clone()),
    )
    .await?;
    Ok(())
}

/// レコード用プロセッサを起動する: エンコーダー → MP4 ライター
/// program mixer の出力トラックを直接エンコーダーに入力するため、ミキサーとソースの起動は不要。
async fn start_record_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    output_path: &std::path::Path,
    run: &crate::obsws::input_registry::ObswsRecordRun,
    frame_rate: crate::video::FrameRate,
) -> crate::Result<()> {
    crate::encoder::create_video_processor(
        pipeline_handle,
        run.video.source_track_id.clone(),
        run.video.encoded_track_id.clone(),
        crate::types::CodecName::H264,
        std::num::NonZeroUsize::new(2_000_000).unwrap(),
        frame_rate,
        Some(run.video.encoder_processor_id.clone()),
    )
    .await?;
    // レコードは Opus エンコーディングを使用する
    crate::encoder::create_audio_processor(
        pipeline_handle,
        run.audio.source_track_id.clone(),
        run.audio.encoded_track_id.clone(),
        crate::types::CodecName::Opus,
        std::num::NonZeroUsize::new(128_000).unwrap(),
        Some(run.audio.encoder_processor_id.clone()),
    )
    .await?;
    crate::mp4::writer::create_processor(
        pipeline_handle,
        output_path.to_path_buf(),
        Some(run.audio.encoded_track_id.clone()),
        Some(run.video.encoded_track_id.clone()),
        Some(run.writer_processor_id.clone()),
    )
    .await?;
    Ok(())
}

/// RTMP outbound 用プロセッサを起動する: エンコーダー → RTMP エンドポイント
/// program mixer の出力トラックを直接エンコーダーに入力するため、ミキサーとソースの起動は不要。
async fn start_rtmp_outbound_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    output_url: &str,
    stream_name: Option<&str>,
    run: &crate::obsws::input_registry::ObswsRtmpOutboundRun,
    frame_rate: crate::video::FrameRate,
) -> crate::Result<()> {
    crate::encoder::create_video_processor(
        pipeline_handle,
        run.video.source_track_id.clone(),
        run.video.encoded_track_id.clone(),
        crate::types::CodecName::H264,
        std::num::NonZeroUsize::new(2_000_000).unwrap(),
        frame_rate,
        Some(run.video.encoder_processor_id.clone()),
    )
    .await?;
    // RTMP outbound は AAC エンコーディングを使用する（RTMP の制約）
    crate::encoder::create_audio_processor(
        pipeline_handle,
        run.audio.source_track_id.clone(),
        run.audio.encoded_track_id.clone(),
        crate::types::CodecName::Aac,
        std::num::NonZeroUsize::new(128_000).unwrap(),
        Some(run.audio.encoder_processor_id.clone()),
    )
    .await?;
    let endpoint = crate::rtmp::outbound_endpoint::RtmpOutboundEndpoint {
        output_url: output_url.to_owned(),
        stream_name: stream_name.map(|s| s.to_owned()),
        input_audio_track_id: Some(run.audio.encoded_track_id.clone()),
        input_video_track_id: Some(run.video.encoded_track_id.clone()),
        options: Default::default(),
    };
    crate::rtmp::outbound_endpoint::create_processor(
        pipeline_handle,
        endpoint,
        Some(run.endpoint_processor_id.clone()),
    )
    .await?;
    Ok(())
}

/// ストリーム用プロセッサを段階的に停止する: エンコーダー → パブリッシャー
async fn stop_processors_staged_stream(
    pipeline_handle: &crate::MediaPipelineHandle,
    run: &crate::obsws::input_registry::ObswsStreamRun,
) -> crate::Result<()> {
    terminate_and_wait(
        pipeline_handle,
        &[
            run.video.encoder_processor_id.clone(),
            run.audio.encoder_processor_id.clone(),
        ],
    )
    .await?;
    terminate_and_wait(
        pipeline_handle,
        std::slice::from_ref(&run.publisher_processor_id),
    )
    .await?;
    Ok(())
}

/// レコード用プロセッサを段階的に停止する。
/// エンコーダーを terminate し、ライターは EOS 伝播で自然終了させる。
async fn stop_processors_staged_record(
    pipeline_handle: &crate::MediaPipelineHandle,
    run: &crate::obsws::input_registry::ObswsRecordRun,
) -> crate::Result<()> {
    // 1. MP4 writer に Finish RPC を送信して finalize を促す
    finish_mp4_writer_rpc(pipeline_handle, &run.writer_processor_id).await;

    // 2. writer の自然終了を待ち、タイムアウト時は強制停止
    wait_or_terminate(
        pipeline_handle,
        std::slice::from_ref(&run.writer_processor_id),
        Duration::from_secs(5),
    )
    .await?;

    // 3. エンコーダーを停止する
    terminate_and_wait(
        pipeline_handle,
        &[
            run.video.encoder_processor_id.clone(),
            run.audio.encoder_processor_id.clone(),
        ],
    )
    .await?;

    Ok(())
}

/// MP4 writer に Finish RPC を送り、finalize を促す。
async fn finish_mp4_writer_rpc(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_id: &crate::ProcessorId,
) {
    const RETRY_TIMEOUT: Duration = Duration::from_millis(500);
    const RETRY_INTERVAL: Duration = Duration::from_millis(10);
    let deadline = tokio::time::Instant::now() + RETRY_TIMEOUT;

    loop {
        match pipeline_handle
            .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<
                crate::mp4::writer::Mp4WriterRpcMessage,
            >>(processor_id)
            .await
        {
            Ok(sender) => {
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                let _ =
                    sender.send(crate::mp4::writer::Mp4WriterRpcMessage::Finish { reply_tx });
                let _ = reply_rx.await;
                return;
            }
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
            Err(_) => {
                let _ = pipeline_handle
                    .terminate_processor(processor_id.clone())
                    .await;
                return;
            }
        }
    }
}

/// S3 クライアントを構築する
fn build_s3_client(
    region: &str,
    access_key_id: &str,
    secret_access_key: &str,
    session_token: Option<&str>,
    endpoint: Option<&str>,
    use_path_style: bool,
) -> crate::Result<crate::s3::S3HttpClient> {
    let credential = match session_token {
        Some(token) => {
            shiguredo_s3::Credential::with_session_token(access_key_id, secret_access_key, token)
        }
        None => shiguredo_s3::Credential::new(access_key_id, secret_access_key),
    };
    let mut config_builder = shiguredo_s3::S3Config::builder()
        .region(region)
        .credential(credential)
        .use_path_style(use_path_style);
    if let Some(ep) = endpoint {
        config_builder = config_builder.endpoint(ep);
    }
    let s3_config = config_builder
        .build()
        .map_err(|e| crate::Error::new(format!("failed to build S3 config: {e}")))?;
    Ok(crate::s3::S3HttpClient::new(s3_config))
}

/// HLS 用プロセッサを起動する
/// 戻り値は ABR マスタープレイリスト書き出しタスクの JoinHandle（ABR でない場合は None）。
/// 呼び出し元は JoinHandle を保持し、出力停止時に abort() すること。
async fn start_hls_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    program_output: &ObswsProgramOutputContext,
    run: &crate::obsws::input_registry::ObswsHlsRun,
    hls_settings: &crate::obsws::input_registry::ObswsHlsSettings,
) -> crate::Result<Option<tokio::task::JoinHandle<()>>> {
    // HLS 用にキーフレーム間隔を設定する。
    // segment_duration に合わせたフレーム数を計算し、エンコーダーに事前通知する。
    let fps = program_output.frame_rate.numerator.get() as f64
        / program_output.frame_rate.denumerator.get() as f64;
    let keyframe_interval_frames = (hls_settings.segment_duration * fps).ceil() as u32;
    let keyframe_interval_frames = keyframe_interval_frames.max(1);
    let encode_params = crate::encoder::encode_config_with_keyframe_interval(
        keyframe_interval_frames,
        program_output.frame_rate,
    );

    let is_abr = run.is_abr();

    // ABR の場合、各 variant writer が SampleEntry から codec string を確定したら
    // oneshot channel 経由で通知を受け取り、全 variant の値がそろってからマスタープレイリストを書き出す。
    let mut codec_string_receivers = Vec::new();

    // バリアントごとにスケーラー、エンコーダー、ライターを起動する
    for (i, (variant, variant_run)) in hls_settings
        .variants
        .iter()
        .zip(run.variant_runs.iter())
        .enumerate()
    {
        // filesystem かつ ABR の場合はバリアントのサブディレクトリを作成する
        if is_abr
            && let crate::obsws::input_registry::HlsDestination::Filesystem { .. } = run.destination
        {
            std::fs::create_dir_all(&variant_run.variant_path).map_err(|e| {
                crate::Error::new(format!(
                    "failed to create variant directory {}: {e}",
                    variant_run.variant_path
                ))
            })?;
        }

        // 解像度変換が必要な場合はスケーラーを挿入する
        let video_encoder_input_track = if let (Some(scaler_id), Some(scaled_track_id)) = (
            &variant_run.scaler_processor_id,
            &variant_run.scaled_track_id,
        ) {
            let width = variant.width.expect("infallible: scaler requires width");
            let height = variant.height.expect("infallible: scaler requires height");
            crate::scaler::create_processor(
                pipeline_handle,
                crate::scaler::VideoScalerConfig {
                    input_track_id: program_output.video_track_id.clone(),
                    output_track_id: scaled_track_id.clone(),
                    width,
                    height,
                },
                Some(scaler_id.clone()),
            )
            .await?;
            scaled_track_id.clone()
        } else {
            variant_run.video.source_track_id.clone()
        };

        // ビデオエンコーダー
        crate::encoder::create_video_processor_with_params(
            pipeline_handle,
            video_encoder_input_track,
            variant_run.video.encoded_track_id.clone(),
            crate::types::CodecName::H264,
            std::num::NonZeroUsize::new(variant.video_bitrate_bps)
                .unwrap_or(std::num::NonZeroUsize::MIN),
            program_output.frame_rate,
            Some(encode_params.clone()),
            Some(variant_run.video.encoder_processor_id.clone()),
        )
        .await?;

        // オーディオエンコーダー（HLS 仕様で AAC 必須）
        crate::encoder::create_audio_processor(
            pipeline_handle,
            program_output.audio_track_id.clone(),
            variant_run.audio.encoded_track_id.clone(),
            crate::types::CodecName::Aac,
            std::num::NonZeroUsize::new(variant.audio_bitrate_bps)
                .unwrap_or(std::num::NonZeroUsize::MIN),
            Some(variant_run.audio.encoder_processor_id.clone()),
        )
        .await?;

        // HLS ライター
        let storage_config = match &run.destination {
            crate::obsws::input_registry::HlsDestination::Filesystem { .. } => {
                crate::hls::writer::HlsStorageConfig::Filesystem {
                    output_directory: std::path::PathBuf::from(&variant_run.variant_path),
                }
            }
            crate::obsws::input_registry::HlsDestination::S3 {
                bucket,
                region,
                endpoint,
                use_path_style,
                access_key_id,
                secret_access_key,
                session_token,
                ..
            } => {
                let client = build_s3_client(
                    region,
                    access_key_id,
                    secret_access_key,
                    session_token.as_deref(),
                    endpoint.as_deref(),
                    *use_path_style,
                )?;
                crate::hls::writer::HlsStorageConfig::S3 {
                    client,
                    bucket: bucket.clone(),
                    prefix: variant_run.variant_path.clone(),
                }
            }
        };
        // ABR の場合は codec string 通知用の channel を作成する
        let codec_string_sender = if is_abr {
            let (tx, rx) = tokio::sync::oneshot::channel();
            codec_string_receivers.push(rx);
            Some(tx)
        } else {
            None
        };

        crate::hls::writer::create_processor(
            pipeline_handle,
            crate::hls::writer::HlsWriterConfig {
                storage: storage_config,
                input_audio_track_id: variant_run.audio.encoded_track_id.clone(),
                input_video_track_id: variant_run.video.encoded_track_id.clone(),
                segment_duration: hls_settings.segment_duration,
                max_retained_segments: hls_settings.max_retained_segments,
                segment_format: hls_settings.segment_format,
                codec_string_sender,
            },
            Some(variant_run.writer_processor_id.clone()),
        )
        .await?;

        tracing::info!(
            variant = i,
            video_bitrate = variant.video_bitrate_bps,
            audio_bitrate = variant.audio_bitrate_bps,
            directory = %variant_run.variant_path,
            "HLS variant processor started"
        );
    }

    // ABR の場合は各 variant writer が SampleEntry から codec string を確定するのを待ち、
    // 全 variant の codec string が一致することを検証してからマスタープレイリストを書き出す。
    if is_abr {
        let master_variants: Vec<crate::hls::writer::MasterPlaylistVariant> = hls_settings
            .variants
            .iter()
            .enumerate()
            .map(|(i, variant)| {
                let width = variant
                    .width
                    .map(|w| w.get() as u32)
                    .unwrap_or(program_output.canvas_width.get() as u32);
                let height = variant
                    .height
                    .map(|h| h.get() as u32)
                    .unwrap_or(program_output.canvas_height.get() as u32);
                crate::hls::writer::MasterPlaylistVariant {
                    bandwidth: variant.video_bitrate_bps as u64 + variant.audio_bitrate_bps as u64,
                    width,
                    height,
                    playlist_uri: format!("variant_{i}/playlist.m3u8"),
                }
            })
            .collect();

        let destination = run.destination.clone();

        let handle = tokio::spawn(async move {
            // 全 variant の codec string を収集する
            let mut codec_strings = Vec::with_capacity(codec_string_receivers.len());
            for (i, rx) in codec_string_receivers.into_iter().enumerate() {
                match rx.await {
                    Ok(cs) => codec_strings.push(cs),
                    Err(_) => {
                        tracing::warn!(
                            variant = i,
                            "HLS variant writer dropped codec string sender before resolving codecs"
                        );
                        return;
                    }
                }
            }

            // 全 variant の codec string が一致することを検証する
            let Some(first) = codec_strings.first() else {
                return;
            };
            for (i, cs) in codec_strings.iter().enumerate().skip(1) {
                if cs.video != first.video || cs.audio != first.audio {
                    tracing::error!(
                        variant = i,
                        expected_video = %first.video,
                        expected_audio = %first.audio,
                        actual_video = %cs.video,
                        actual_audio = %cs.audio,
                        "HLS ABR variant codec string mismatch: \
                         all variants must produce identical codec strings"
                    );
                    return;
                }
            }

            let master_content =
                crate::hls::writer::build_master_playlist_content(&master_variants, first);
            match &destination {
                crate::obsws::input_registry::HlsDestination::Filesystem { directory } => {
                    if let Err(e) = crate::hls::writer::write_master_playlist(
                        &std::path::PathBuf::from(directory),
                        &master_variants,
                        first,
                    ) {
                        tracing::error!(error = ?e, "failed to write HLS master playlist");
                    }
                }
                crate::obsws::input_registry::HlsDestination::S3 {
                    bucket,
                    prefix,
                    region,
                    endpoint,
                    use_path_style,
                    access_key_id,
                    secret_access_key,
                    session_token,
                    ..
                } => {
                    let s3_client = match build_s3_client(
                        region,
                        access_key_id,
                        secret_access_key,
                        session_token.as_deref(),
                        endpoint.as_deref(),
                        *use_path_style,
                    ) {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::error!(error = ?e, "failed to create S3 client for HLS master playlist");
                            return;
                        }
                    };
                    let key = if prefix.is_empty() {
                        "playlist.m3u8".to_owned()
                    } else {
                        format!("{prefix}/playlist.m3u8")
                    };
                    let request = match s3_client
                        .client()
                        .put_object()
                        .bucket(bucket)
                        .key(&key)
                        .body(master_content.into_bytes())
                        .content_type("application/vnd.apple.mpegurl")
                        .build_request()
                    {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::error!(error = ?e, "failed to build S3 PutObject request for HLS master playlist");
                            return;
                        }
                    };
                    match s3_client.execute(&request).await {
                        Ok(response) if !response.is_success() => {
                            tracing::error!(
                                status = response.status_code,
                                "S3 PutObject failed for HLS master playlist {key}"
                            );
                        }
                        Err(e) => {
                            tracing::error!(error = ?e, "failed to upload HLS master playlist to S3");
                        }
                        _ => {}
                    }
                }
            }
        });
        Ok(Some(handle))
    } else {
        Ok(None)
    }
}

/// HLS 用プロセッサを段階的に停止する。
/// Program 出力は共有なので、variant 後段の processor のみを停止する。
async fn stop_processors_staged_hls(
    pipeline_handle: &crate::MediaPipelineHandle,
    run: &crate::obsws::input_registry::ObswsHlsRun,
) -> crate::Result<()> {
    // NOTE:
    // ライブ用途では StopOutput / ToggleOutput への応答遅延を避けることを優先し、
    // ここでは writer に finalize / cleanup を先行させる。
    // この経路は上流 encoder / scaler の完全 drain を保証しないため、
    // 停止直前の数フレームが最終セグメントに含まれない可能性がある。
    //
    // TODO:
    // 末尾欠損まで解消するには、writer を先に閉じるのではなく、
    // 上流から EOS 相当を伝播させる明示的な finish 経路が必要になる。
    // terminate_processor() は abort ベースで停止するだけなので、
    // encoder / scaler の残フレーム排出には使えない。
    let writer_ids: Vec<crate::ProcessorId> = run
        .variant_runs
        .iter()
        .map(|vr| vr.writer_processor_id.clone())
        .collect();
    for writer_id in &writer_ids {
        finish_hls_writer_rpc(pipeline_handle, writer_id).await;
    }
    wait_or_terminate(pipeline_handle, &writer_ids, Duration::from_secs(5)).await?;

    let encoder_ids: Vec<crate::ProcessorId> = run
        .variant_runs
        .iter()
        .flat_map(|vr| {
            [
                vr.video.encoder_processor_id.clone(),
                vr.audio.encoder_processor_id.clone(),
            ]
        })
        .collect();
    terminate_and_wait(pipeline_handle, &encoder_ids).await?;

    let scaler_ids: Vec<crate::ProcessorId> = run
        .variant_runs
        .iter()
        .filter_map(|vr| vr.scaler_processor_id.clone())
        .collect();
    if !scaler_ids.is_empty() {
        terminate_and_wait(pipeline_handle, &scaler_ids).await?;
    }

    // ABR の場合はマスタープレイリストとバリアントディレクトリを削除する
    if run.is_abr() {
        match &run.destination {
            crate::obsws::input_registry::HlsDestination::Filesystem { directory } => {
                let master_playlist_path =
                    std::path::PathBuf::from(directory).join("playlist.m3u8");
                if let Err(e) = std::fs::remove_file(&master_playlist_path)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!(
                        "failed to remove master playlist {}: {e}",
                        master_playlist_path.display()
                    );
                }
                // バリアントのサブディレクトリも削除する（ライターが中身を削除済みなので空のはず）
                for vr in &run.variant_runs {
                    if let Err(e) = std::fs::remove_dir(&vr.variant_path)
                        && e.kind() != std::io::ErrorKind::NotFound
                    {
                        tracing::warn!(
                            "failed to remove variant directory {}: {e}",
                            vr.variant_path
                        );
                    }
                }
            }
            crate::obsws::input_registry::HlsDestination::S3 {
                bucket,
                prefix,
                region,
                endpoint,
                use_path_style,
                access_key_id,
                secret_access_key,
                session_token,
                ..
            } => {
                // マスタープレイリストを DeleteObject で削除する
                // バリアント「ディレクトリ」の削除は不要（S3 にディレクトリ概念なし）
                if let Ok(s3_client) = build_s3_client(
                    region,
                    access_key_id,
                    secret_access_key,
                    session_token.as_deref(),
                    endpoint.as_deref(),
                    *use_path_style,
                ) {
                    let key = if prefix.is_empty() {
                        "playlist.m3u8".to_owned()
                    } else {
                        format!("{prefix}/playlist.m3u8")
                    };
                    match s3_client
                        .client()
                        .delete_object()
                        .bucket(bucket)
                        .key(&key)
                        .build_request()
                    {
                        Ok(request) => match s3_client.execute(&request).await {
                            Ok(response) if !response.is_success() => {
                                tracing::warn!(
                                    "S3 DeleteObject failed for master playlist {key}: status={}",
                                    response.status_code
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "failed to delete S3 master playlist {key}: {}",
                                    e.display()
                                );
                            }
                            _ => {}
                        },
                        Err(e) => {
                            tracing::warn!(
                                "failed to build DeleteObject for master playlist {key}: {e}"
                            );
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// 戻り値は ABR 結合 MPD 書き出しタスクの JoinHandle（ABR でない場合は None）。
/// 呼び出し元は JoinHandle を保持し、出力停止時に abort() すること。
async fn start_dash_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    program_output: &ObswsProgramOutputContext,
    run: &crate::obsws::input_registry::ObswsDashRun,
    dash_settings: &crate::obsws::input_registry::ObswsDashSettings,
) -> crate::Result<Option<tokio::task::JoinHandle<()>>> {
    // MPEG-DASH 用にキーフレーム間隔を設定する
    let fps = program_output.frame_rate.numerator.get() as f64
        / program_output.frame_rate.denumerator.get() as f64;
    let keyframe_interval_frames = (dash_settings.segment_duration * fps).ceil() as u32;
    let keyframe_interval_frames = keyframe_interval_frames.max(1);
    let encode_params = crate::encoder::encode_config_with_keyframe_interval(
        keyframe_interval_frames,
        program_output.frame_rate,
    );

    let is_abr = run.is_abr();

    // ABR の場合、各 variant writer が SampleEntry から codec string を確定したら
    // oneshot channel 経由で通知を受け取り、全 variant の値がそろってから結合 MPD を書き出す。
    let mut codec_string_receivers = Vec::new();

    // バリアントごとにスケーラー、エンコーダー、ライターを起動する
    for (i, (variant, variant_run)) in dash_settings
        .variants
        .iter()
        .zip(run.variant_runs.iter())
        .enumerate()
    {
        // filesystem かつ ABR の場合はバリアントのサブディレクトリを作成する
        if is_abr
            && let crate::obsws::input_registry::DashDestination::Filesystem { .. } =
                run.destination
        {
            std::fs::create_dir_all(&variant_run.variant_path).map_err(|e| {
                crate::Error::new(format!(
                    "failed to create variant directory {}: {e}",
                    variant_run.variant_path
                ))
            })?;
        }

        // 解像度変換が必要な場合はスケーラーを挿入する
        let video_encoder_input_track = if let (Some(scaler_id), Some(scaled_track_id)) = (
            &variant_run.scaler_processor_id,
            &variant_run.scaled_track_id,
        ) {
            let width = variant.width.expect("infallible: scaler requires width");
            let height = variant.height.expect("infallible: scaler requires height");
            crate::scaler::create_processor(
                pipeline_handle,
                crate::scaler::VideoScalerConfig {
                    input_track_id: program_output.video_track_id.clone(),
                    output_track_id: scaled_track_id.clone(),
                    width,
                    height,
                },
                Some(scaler_id.clone()),
            )
            .await?;
            scaled_track_id.clone()
        } else {
            variant_run.video.source_track_id.clone()
        };

        // ビデオエンコーダー
        crate::encoder::create_video_processor_with_params(
            pipeline_handle,
            video_encoder_input_track,
            variant_run.video.encoded_track_id.clone(),
            dash_settings.video_codec,
            std::num::NonZeroUsize::new(variant.video_bitrate_bps)
                .unwrap_or(std::num::NonZeroUsize::MIN),
            program_output.frame_rate,
            Some(encode_params.clone()),
            Some(variant_run.video.encoder_processor_id.clone()),
        )
        .await?;

        // オーディオエンコーダー
        crate::encoder::create_audio_processor(
            pipeline_handle,
            program_output.audio_track_id.clone(),
            variant_run.audio.encoded_track_id.clone(),
            dash_settings.audio_codec,
            std::num::NonZeroUsize::new(variant.audio_bitrate_bps)
                .unwrap_or(std::num::NonZeroUsize::MIN),
            Some(variant_run.audio.encoder_processor_id.clone()),
        )
        .await?;

        // DASH ライター
        let storage_config = match &run.destination {
            crate::obsws::input_registry::DashDestination::Filesystem { .. } => {
                crate::dash::writer::DashStorageConfig::Filesystem {
                    output_directory: std::path::PathBuf::from(&variant_run.variant_path),
                }
            }
            crate::obsws::input_registry::DashDestination::S3 {
                bucket,
                region,
                endpoint,
                use_path_style,
                access_key_id,
                secret_access_key,
                session_token,
                ..
            } => {
                let client = build_s3_client(
                    region,
                    access_key_id,
                    secret_access_key,
                    session_token.as_deref(),
                    endpoint.as_deref(),
                    *use_path_style,
                )?;
                crate::dash::writer::DashStorageConfig::S3 {
                    client,
                    bucket: bucket.clone(),
                    prefix: variant_run.variant_path.clone(),
                }
            }
        };
        // ABR の場合は codec string 通知用の channel を作成する
        let codec_string_sender = if is_abr {
            let (tx, rx) = tokio::sync::oneshot::channel();
            codec_string_receivers.push(rx);
            Some(tx)
        } else {
            None
        };

        crate::dash::writer::create_processor(
            pipeline_handle,
            crate::dash::writer::DashWriterConfig {
                storage: storage_config,
                input_audio_track_id: variant_run.audio.encoded_track_id.clone(),
                input_video_track_id: variant_run.video.encoded_track_id.clone(),
                segment_duration: dash_settings.segment_duration,
                max_retained_segments: dash_settings.max_retained_segments,
                skip_mpd: is_abr,
                codec_string_sender,
            },
            Some(variant_run.writer_processor_id.clone()),
        )
        .await?;

        tracing::info!(
            variant = i,
            video_bitrate = variant.video_bitrate_bps,
            audio_bitrate = variant.audio_bitrate_bps,
            directory = %variant_run.variant_path,
            "MPEG-DASH variant processor started"
        );
    }

    // ABR の場合は各 variant writer が SampleEntry から codec string を確定するのを待ち、
    // 全 variant の codec string が一致することを検証してから結合 MPD を書き出す。
    if is_abr {
        let mpd_variants: Vec<crate::dash::writer::CombinedMpdVariant> = dash_settings
            .variants
            .iter()
            .enumerate()
            .map(|(i, variant)| {
                let width = variant
                    .width
                    .map(|w| w.get() as u32)
                    .unwrap_or(program_output.canvas_width.get() as u32);
                let height = variant
                    .height
                    .map(|h| h.get() as u32)
                    .unwrap_or(program_output.canvas_height.get() as u32);
                Ok(crate::dash::writer::CombinedMpdVariant {
                    bandwidth: variant.video_bitrate_bps as u64 + variant.audio_bitrate_bps as u64,
                    width,
                    height,
                    media_path: dash_variant_media_path(&run.destination, &run.variant_runs[i])?,
                    init_path: dash_variant_init_path(&run.destination, &run.variant_runs[i])?,
                })
            })
            .collect::<crate::Result<Vec<_>>>()?;
        let root_storage_config = build_dash_root_storage_config(&run.destination)?;
        let segment_duration = dash_settings.segment_duration;
        let max_retained_segments = dash_settings.max_retained_segments;

        // 各 variant の codec string が確定するのを待ってから結合 MPD を書き出すタスクを起動する。
        // JoinHandle を呼び出し元に返し、出力停止時に abort() でキャンセルできるようにする。
        let handle = tokio::spawn(async move {
            // 全 variant の codec string を収集する
            let mut codec_strings = Vec::with_capacity(codec_string_receivers.len());
            for (i, rx) in codec_string_receivers.into_iter().enumerate() {
                match rx.await {
                    Ok(cs) => codec_strings.push(cs),
                    Err(_) => {
                        tracing::warn!(
                            variant = i,
                            "DASH variant writer dropped codec string sender before resolving codecs"
                        );
                        return;
                    }
                }
            }

            // 全 variant の codec string が一致することを検証する
            if let Some(first) = codec_strings.first() {
                for (i, cs) in codec_strings.iter().enumerate().skip(1) {
                    if cs.video != first.video || cs.audio != first.audio {
                        tracing::error!(
                            variant = i,
                            expected_video = %first.video,
                            expected_audio = %first.audio,
                            actual_video = %cs.video,
                            actual_audio = %cs.audio,
                            "DASH ABR variant codec string mismatch: \
                             all variants must produce identical codec strings"
                        );
                        return;
                    }
                }

                if let Err(e) = crate::dash::writer::write_combined_mpd(
                    root_storage_config,
                    &mpd_variants,
                    segment_duration,
                    max_retained_segments,
                    first,
                )
                .await
                {
                    tracing::error!(error = ?e, "failed to write combined DASH MPD");
                }
            }
        });
        Ok(Some(handle))
    } else {
        Ok(None)
    }
}

/// MPEG-DASH 用プロセッサを段階的に停止する。
/// Program 出力は共有なので、variant 後段の processor のみを停止する。
async fn stop_processors_staged_dash(
    pipeline_handle: &crate::MediaPipelineHandle,
    run: &crate::obsws::input_registry::ObswsDashRun,
) -> crate::Result<()> {
    // NOTE:
    // ライブ用途では StopOutput / ToggleOutput への応答遅延を避けることを優先し、
    // ここでは writer に finalize / cleanup を先行させる。
    // この経路は上流 encoder / scaler の完全 drain を保証しないため、
    // 停止直前の数フレームが最終 segment や MPD に反映されない可能性がある。
    //
    // TODO:
    // 末尾欠損まで解消するには、writer を先に閉じるのではなく、
    // 上流から EOS 相当を伝播させる明示的な finish 経路が必要になる。
    // terminate_processor() は abort ベースで停止するだけなので、
    // encoder / scaler の残フレーム排出には使えない。
    // 1. 各 writer に finalize / cleanup を要求し、停止を待つ。
    let writer_ids: Vec<crate::ProcessorId> = run
        .variant_runs
        .iter()
        .map(|vr| vr.writer_processor_id.clone())
        .collect();
    for writer_id in &writer_ids {
        finish_dash_writer_rpc(pipeline_handle, writer_id).await;
    }
    wait_or_terminate(pipeline_handle, &writer_ids, Duration::from_secs(5)).await?;

    // 2. 全バリアントのエンコーダーを停止する。
    let encoder_ids: Vec<crate::ProcessorId> = run
        .variant_runs
        .iter()
        .flat_map(|vr| {
            [
                vr.video.encoder_processor_id.clone(),
                vr.audio.encoder_processor_id.clone(),
            ]
        })
        .collect();
    terminate_and_wait(pipeline_handle, &encoder_ids).await?;

    // 3. 解像度変換があるバリアントのスケーラーを停止する。
    let scaler_ids: Vec<crate::ProcessorId> = run
        .variant_runs
        .iter()
        .filter_map(|vr| vr.scaler_processor_id.clone())
        .collect();
    if !scaler_ids.is_empty() {
        terminate_and_wait(pipeline_handle, &scaler_ids).await?;
    }

    // ABR の場合は結合 MPD とバリアントディレクトリを削除する
    if run.is_abr() {
        if let Ok(root_storage_config) = build_dash_root_storage_config(&run.destination) {
            crate::dash::writer::delete_combined_mpd(root_storage_config).await;
        }
        // filesystem の場合はバリアントのサブディレクトリも削除する（ライターが中身を削除済みなので空のはず）
        if let crate::obsws::input_registry::DashDestination::Filesystem { .. } = &run.destination {
            for vr in &run.variant_runs {
                if let Err(e) = std::fs::remove_dir(&vr.variant_path)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!(
                        "failed to remove variant directory {}: {e}",
                        vr.variant_path
                    );
                }
            }
        }
    }

    Ok(())
}

/// 結合 MPD に書く media path を生成する。
/// writer が実際に使う variant_path と同じ規則から相対パスを導出する。
fn dash_variant_media_path(
    destination: &crate::obsws::input_registry::DashDestination,
    variant_run: &crate::obsws::input_registry::ObswsDashVariantRun,
) -> crate::Result<String> {
    let base_path = dash_variant_relative_path(destination, &variant_run.variant_path)?;
    Ok(format!("{base_path}/segment-$Number%06d$.m4s"))
}

/// 結合 MPD に書く init segment path を生成する。
/// writer が実際に使う variant_path と同じ規則から相対パスを導出する。
fn dash_variant_init_path(
    destination: &crate::obsws::input_registry::DashDestination,
    variant_run: &crate::obsws::input_registry::ObswsDashVariantRun,
) -> crate::Result<String> {
    let base_path = dash_variant_relative_path(destination, &variant_run.variant_path)?;
    Ok(format!("{base_path}/init.mp4"))
}

/// variant_path から結合 MPD 用の相対パス部分を取り出す。
fn dash_variant_relative_path(
    destination: &crate::obsws::input_registry::DashDestination,
    variant_path: &str,
) -> crate::Result<String> {
    match destination {
        crate::obsws::input_registry::DashDestination::Filesystem { directory } => {
            let root = std::path::Path::new(directory);
            let path = std::path::Path::new(variant_path);
            let relative = path.strip_prefix(root).map_err(|_| {
                crate::Error::new(format!(
                    "variant path {variant_path} is not under DASH destination root {directory}"
                ))
            })?;
            Ok(relative.to_string_lossy().replace('\\', "/"))
        }
        crate::obsws::input_registry::DashDestination::S3 { prefix, .. } => {
            if prefix.is_empty() {
                return Ok(variant_path.to_owned());
            }
            let Some(relative) = variant_path.strip_prefix(prefix) else {
                return Err(crate::Error::new(format!(
                    "variant path {variant_path} does not start with DASH destination prefix {prefix}"
                )));
            };
            Ok(relative.trim_start_matches('/').to_owned())
        }
    }
}

/// DASH destination からルートディレクトリ/prefix 用の DashStorageConfig を構築する。
/// 結合 MPD の書き出し・削除に使用する。
fn build_dash_root_storage_config(
    destination: &crate::obsws::input_registry::DashDestination,
) -> crate::Result<crate::dash::writer::DashStorageConfig> {
    match destination {
        crate::obsws::input_registry::DashDestination::Filesystem { directory } => {
            Ok(crate::dash::writer::DashStorageConfig::Filesystem {
                output_directory: std::path::PathBuf::from(directory),
            })
        }
        crate::obsws::input_registry::DashDestination::S3 {
            bucket,
            prefix,
            region,
            endpoint,
            use_path_style,
            access_key_id,
            secret_access_key,
            session_token,
            ..
        } => {
            let client = build_s3_client(
                region,
                access_key_id,
                secret_access_key,
                session_token.as_deref(),
                endpoint.as_deref(),
                *use_path_style,
            )?;
            Ok(crate::dash::writer::DashStorageConfig::S3 {
                client,
                bucket: bucket.clone(),
                prefix: prefix.clone(),
            })
        }
    }
}

/// RTMP outbound 用プロセッサを段階的に停止する: エンコーダー → エンドポイント
async fn stop_processors_staged_rtmp_outbound(
    pipeline_handle: &crate::MediaPipelineHandle,
    run: &crate::obsws::input_registry::ObswsRtmpOutboundRun,
) -> crate::Result<()> {
    terminate_and_wait(
        pipeline_handle,
        &[
            run.video.encoder_processor_id.clone(),
            run.audio.encoder_processor_id.clone(),
        ],
    )
    .await?;
    terminate_and_wait(
        pipeline_handle,
        std::slice::from_ref(&run.endpoint_processor_id),
    )
    .await?;
    Ok(())
}

/// プロセッサを terminate してから停止を待つ
async fn terminate_and_wait(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_ids: &[crate::ProcessorId],
) -> crate::Result<()> {
    for id in processor_ids {
        let _ = pipeline_handle.terminate_processor(id.clone()).await;
    }
    wait_processors_stopped(pipeline_handle, processor_ids, Duration::from_secs(5)).await?;
    Ok(())
}

/// 指定したプロセッサが全て停止するまでポーリングする
async fn wait_processors_stopped(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_ids: &[crate::ProcessorId],
    timeout: Duration,
) -> crate::Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let live = live_processor_ids(pipeline_handle, processor_ids).await;
        if live.is_empty() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(crate::Error::new("timeout waiting for processors to stop"));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// プロセッサの自然終了を待ち、タイムアウト後に強制停止する
async fn wait_or_terminate(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_ids: &[crate::ProcessorId],
    timeout: Duration,
) -> crate::Result<()> {
    if wait_processors_stopped(pipeline_handle, processor_ids, timeout)
        .await
        .is_ok()
    {
        return Ok(());
    }
    let live = live_processor_ids(pipeline_handle, processor_ids).await;
    if live.is_empty() {
        return Ok(());
    }
    terminate_and_wait(pipeline_handle, &live).await
}

/// 指定したプロセッサ ID のうち、まだ生存しているものを返す
async fn live_processor_ids(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_ids: &[crate::ProcessorId],
) -> Vec<crate::ProcessorId> {
    let Ok(live) = pipeline_handle.list_processors().await else {
        return Vec::new();
    };
    processor_ids
        .iter()
        .filter(|id| live.contains(id))
        .cloned()
        .collect()
}

/// HLS writer に Finish RPC を送り、finalize / cleanup を促す。
/// これは writer 側の入力購読を閉じるためのもので、上流の完全 drain は保証しない。
/// 失敗時は terminate にフォールバックする。
async fn finish_hls_writer_rpc(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_id: &crate::ProcessorId,
) {
    const RETRY_TIMEOUT: Duration = Duration::from_millis(500);
    const RETRY_INTERVAL: Duration = Duration::from_millis(10);
    let deadline = tokio::time::Instant::now() + RETRY_TIMEOUT;

    loop {
        match pipeline_handle
            .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<
                crate::hls::writer::HlsWriterRpcMessage,
            >>(processor_id)
            .await
        {
            Ok(sender) => {
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                let _ = sender.send(crate::hls::writer::HlsWriterRpcMessage::Finish { reply_tx });
                let _ = reply_rx.await;
                return;
            }
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
            Err(_) => {
                let _ = pipeline_handle.terminate_processor(processor_id.clone()).await;
                return;
            }
        }
    }
}

/// DASH writer に Finish RPC を送り、finalize / cleanup を促す。
/// これは writer 側の入力購読を閉じるためのもので、上流の完全 drain は保証しない。
/// 失敗時は terminate にフォールバックする。
async fn finish_dash_writer_rpc(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_id: &crate::ProcessorId,
) {
    const RETRY_TIMEOUT: Duration = Duration::from_millis(500);
    const RETRY_INTERVAL: Duration = Duration::from_millis(10);
    let deadline = tokio::time::Instant::now() + RETRY_TIMEOUT;

    loop {
        match pipeline_handle
            .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<
                crate::dash::writer::DashWriterRpcMessage,
            >>(processor_id)
            .await
        {
            Ok(sender) => {
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                let _ = sender.send(crate::dash::writer::DashWriterRpcMessage::Finish { reply_tx });
                let _ = reply_rx.await;
                return;
            }
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
            Err(_) => {
                let _ = pipeline_handle.terminate_processor(processor_id.clone()).await;
                return;
            }
        }
    }
}

// --- SoraSubscriber / sora_source ハンドラ ---

impl ObswsCoordinator {
    fn handle_sora_source_event(&mut self, event: crate::sora_source::SoraSourceEvent) {
        match event {
            crate::sora_source::SoraSourceEvent::TrackReceived {
                subscriber_name,
                transceiver,
            } => {
                let receiver = transceiver.receiver();
                let track = receiver.track();
                let track_id = track.id().unwrap_or_default();
                let track_kind = track.kind().unwrap_or_default();
                tracing::debug!(
                    "TrackReceived: subscriber={}, track_id={}, kind={}",
                    subscriber_name,
                    track_id,
                    track_kind
                );

                // 空の track_id や kind をスキップ
                if track_id.is_empty() || track_kind.is_empty() {
                    tracing::debug!("skipping track with empty id or kind");
                    return;
                }

                // track_id は "{connection_id}-{video|audio}" 形式
                let connection_id = track_id
                    .rsplit_once('-')
                    .map(|(prefix, _suffix)| prefix.to_owned())
                    .unwrap_or_else(|| track_id.clone());

                // on_notify の connection.created で収集済みの接続情報から client_id を取得する
                let client_id = self
                    .sora_subscribers
                    .get(&subscriber_name)
                    .and_then(|state| state.connections.get(&connection_id))
                    .and_then(|info| info.client_id.clone());

                if let Some(state) = self.sora_subscribers.get_mut(&subscriber_name) {
                    // holder タスクを起動して WebRTC 型の所有権を移す
                    let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
                    let holder_task = tokio::spawn(crate::sora_source::sora_track_holder_task(
                        transceiver,
                        track_kind.clone(),
                        command_rx,
                    ));

                    state.remote_tracks.insert(
                        track_id.clone(),
                        SoraSourceRemoteTrack {
                            connection_id: connection_id.clone(),
                            client_id: client_id.clone(),
                            track_kind: track_kind.clone(),
                            attached_input_name: None,
                            attached_pipeline_track_id: None,
                            command_tx,
                            holder_abort: holder_task.abort_handle(),
                        },
                    );
                    let event = crate::obsws::response::build_sora_source_track_published_event(
                        &subscriber_name,
                        &connection_id,
                        client_id.as_deref(),
                        &track_kind,
                        &track_id,
                    );
                    let _ = self.obsws_event_tx.send(TaggedEvent {
                        text: event,
                        subscription_flag: OBSWS_EVENT_SUB_SORA_SOURCE,
                    });
                }
            }
            crate::sora_source::SoraSourceEvent::TrackRemoved {
                subscriber_name,
                track_id,
            } => {
                if let Some(state) = self.sora_subscribers.get_mut(&subscriber_name)
                    && let Some(remote_track) = state.remote_tracks.remove(&track_id)
                {
                    remote_track.holder_abort.abort();
                    if let Some(input_name) = &remote_track.attached_input_name {
                        self.clear_sora_source_track_id(input_name, &remote_track.track_kind);
                    }
                    let event = crate::obsws::response::build_sora_source_track_unpublished_event(
                        &subscriber_name,
                        &remote_track.connection_id,
                        &remote_track.track_kind,
                        &track_id,
                    );
                    let _ = self.obsws_event_tx.send(TaggedEvent {
                        text: event,
                        subscription_flag: OBSWS_EVENT_SUB_SORA_SOURCE,
                    });
                }
            }
            crate::sora_source::SoraSourceEvent::Notify {
                subscriber_name,
                json,
            } => {
                // Sora の notify は必ず JSON であるべき
                let Ok(parsed) = nojson::RawJsonOwned::parse(&json) else {
                    tracing::warn!(
                        "SoraSubscriberNotify: invalid JSON from Sora, dropping: {}",
                        &json[..json.len().min(200)]
                    );
                    return;
                };

                // connection.created / connection.destroyed をパースして接続情報を管理する
                let v = parsed.value();
                let event_type: Option<String> = v
                    .to_member("event_type")
                    .ok()
                    .and_then(|m| m.optional())
                    .and_then(|v| v.try_into().ok());
                match event_type.as_deref() {
                    Some("connection.created") => {
                        let connection_id: Option<String> = v
                            .to_member("connection_id")
                            .ok()
                            .and_then(|m| m.optional())
                            .and_then(|v| v.try_into().ok());
                        let Some(cid) = connection_id else {
                            tracing::warn!("connection.created notify missing connection_id");
                            return;
                        };
                        let client_id: Option<String> = v
                            .to_member("client_id")
                            .ok()
                            .and_then(|m| m.optional())
                            .and_then(|v| v.try_into().ok());
                        if let Some(state) = self.sora_subscribers.get_mut(&subscriber_name) {
                            state
                                .connections
                                .insert(cid, SoraConnectionInfo { client_id });
                        }
                    }
                    Some("connection.destroyed") => {
                        let connection_id: Option<String> = v
                            .to_member("connection_id")
                            .ok()
                            .and_then(|m| m.optional())
                            .and_then(|v| v.try_into().ok());
                        let Some(cid) = connection_id else {
                            tracing::warn!("connection.destroyed notify missing connection_id");
                            return;
                        };
                        if let Some(state) = self.sora_subscribers.get_mut(&subscriber_name) {
                            state.connections.remove(&cid);
                        }
                    }
                    _ => {}
                }

                let event = crate::obsws::response::build_sora_subscriber_notify_event(
                    &subscriber_name,
                    &parsed,
                );
                let _ = self.obsws_event_tx.send(TaggedEvent {
                    text: event,
                    subscription_flag: OBSWS_EVENT_SUB_SORA_SOURCE,
                });
            }
            crate::sora_source::SoraSourceEvent::WebSocketClose {
                subscriber_name,
                code,
                reason,
            } => {
                let event = crate::obsws::response::build_sora_subscriber_disconnected_event(
                    &subscriber_name,
                    code,
                    &reason,
                );
                let _ = self.obsws_event_tx.send(TaggedEvent {
                    text: event,
                    subscription_flag: OBSWS_EVENT_SUB_SORA_SOURCE,
                });
            }
            crate::sora_source::SoraSourceEvent::Disconnected { subscriber_name } => {
                let drained: Vec<_> = self
                    .sora_subscribers
                    .get_mut(&subscriber_name)
                    .map(|state| {
                        state.run = None;
                        state.remote_tracks.drain().collect()
                    })
                    .unwrap_or_default();
                for (track_id, remote_track) in drained {
                    remote_track.holder_abort.abort();
                    if let Some(input_name) = &remote_track.attached_input_name {
                        self.clear_sora_source_track_id(input_name, &remote_track.track_kind);
                    }
                    let event = crate::obsws::response::build_sora_source_track_unpublished_event(
                        &subscriber_name,
                        &remote_track.connection_id,
                        &remote_track.track_kind,
                        &track_id,
                    );
                    let _ = self.obsws_event_tx.send(TaggedEvent {
                        text: event,
                        subscription_flag: OBSWS_EVENT_SUB_SORA_SOURCE,
                    });
                }
            }
        }
    }

    fn clear_sora_source_track_id(&mut self, input_name: &str, track_kind: &str) {
        if let Some(uuid) = self.input_registry.uuids_by_name.get(input_name)
            && let Some(entry) = self.input_registry.inputs_by_uuid.get_mut(uuid)
            && let crate::obsws::input_registry::ObswsInputSettings::SoraSource(ref mut s) =
                entry.input.settings
        {
            match track_kind {
                "video" => s.video_track_id = None,
                "audio" => s.audio_track_id = None,
                _ => {}
            }
        }
    }

    async fn handle_start_sora_subscriber(
        &mut self,
        request_type: &str,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(data) = request_data else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing requestData",
            );
        };
        // requestData から全パラメータをパースする
        let subscriber_name = match Self::parse_subscriber_name(data) {
            Ok(name) => name,
            Err(msg) => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_MISSING_REQUEST_FIELD,
                    &msg,
                );
            }
        };
        // 同名の subscriber が既に存在する場合はエラー
        if self.sora_subscribers.contains_key(&subscriber_name) {
            return self.build_error_result(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_OUTPUT_RUNNING,
                "Subscriber is already active",
            );
        }
        let json = data.value();
        let signaling_urls: Vec<String> = json
            .to_member("signalingUrls")
            .ok()
            .and_then(|v| v.optional())
            .and_then(|v| v.try_into().ok())
            .unwrap_or_default();
        if signaling_urls.is_empty() {
            return self.build_error_result(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                "signalingUrls must not be empty",
            );
        }
        let channel_id: Option<String> = json
            .to_member("channelId")
            .ok()
            .and_then(|v| v.optional())
            .and_then(|v| v.try_into().ok());
        let Some(channel_id) = channel_id else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing channelId field",
            );
        };
        let client_id: Option<String> = json
            .to_member("clientId")
            .ok()
            .and_then(|v| v.optional())
            .and_then(|v| v.try_into().ok());
        let bundle_id: Option<String> = json
            .to_member("bundleId")
            .ok()
            .and_then(|v| v.optional())
            .and_then(|v| v.try_into().ok());
        let metadata: Option<nojson::RawJsonOwned> = json
            .to_member("metadata")
            .ok()
            .and_then(|v| v.optional())
            .filter(|v| v.kind().is_object())
            .map(|v| v.extract().into_owned());
        // SoraSubscriberState を作成して挿入する
        let settings = crate::obsws::input_registry::ObswsSoraSubscriberSettings {
            signaling_urls,
            channel_id: Some(channel_id.clone()),
            client_id,
            bundle_id,
            metadata,
        };
        self.sora_subscribers.insert(
            subscriber_name.clone(),
            SoraSubscriberState {
                settings,
                run: None,
                remote_tracks: std::collections::HashMap::new(),
                connections: std::collections::HashMap::new(),
            },
        );
        let state = self
            .sora_subscribers
            .get_mut(&subscriber_name)
            .expect("subscriber was just inserted");
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            return self.build_error_result(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                "Pipeline is not initialized",
            );
        };
        let processor_id =
            crate::ProcessorId::new(format!("obsws:sora_subscriber:{}", subscriber_name));
        let subscriber = crate::sora_source::SoraSubscriber {
            subscriber_name: subscriber_name.clone(),
            signaling_urls: state.settings.signaling_urls.clone(),
            channel_id,
            client_id: state.settings.client_id.clone(),
            bundle_id: state.settings.bundle_id.clone(),
            metadata: state.settings.metadata.clone(),
            event_tx: self.sora_source_event_tx.clone(),
        };
        if let Err(e) = crate::sora_source::create_processor(
            pipeline_handle,
            subscriber,
            Some(processor_id.clone()),
        )
        .await
        {
            return self.build_error_result(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                &format!("Failed to start sora subscriber: {}", e.display()),
            );
        }
        state.run = Some(SoraSubscriberRun { processor_id });
        self.build_result_from_response(
            crate::obsws::response::build_request_response_success_no_data(
                request_type,
                request_id,
            ),
            Vec::new(),
        )
    }

    async fn handle_stop_sora_subscriber(
        &mut self,
        request_type: &str,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(data) = request_data else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing requestData",
            );
        };
        let subscriber_name = match Self::parse_subscriber_name(data) {
            Ok(name) => name,
            Err(msg) => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_MISSING_REQUEST_FIELD,
                    &msg,
                );
            }
        };
        // subscriber の存在と稼働状態を確認する
        let is_active = self
            .sora_subscribers
            .get(&subscriber_name)
            .map(|s| s.run.is_some());
        match is_active {
            None => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Subscriber not found",
                );
            }
            Some(false) => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                    "Subscriber is not active",
                );
            }
            Some(true) => {}
        }
        // subscriber を削除して所有権を取得する
        let mut removed_state = self
            .sora_subscribers
            .remove(&subscriber_name)
            .expect("subscriber existence was just verified");
        let run = removed_state
            .run
            .take()
            .expect("subscriber was verified as active");
        if let Some(pipeline_handle) = self.pipeline_handle.as_ref()
            && let Err(e) =
                terminate_and_wait(pipeline_handle, std::slice::from_ref(&run.processor_id)).await
        {
            tracing::warn!("failed to stop sora subscriber processor: {}", e.display());
        }
        // remote_tracks をクリーンアップする
        let drained: Vec<(String, SoraSourceRemoteTrack)> =
            removed_state.remote_tracks.into_iter().collect();
        for (_track_id, rt) in drained {
            rt.holder_abort.abort();
            if let Some(input_name) = &rt.attached_input_name {
                self.clear_sora_source_track_id(input_name, &rt.track_kind);
            }
        }
        self.build_result_from_response(
            crate::obsws::response::build_request_response_success_no_data(
                request_type,
                request_id,
            ),
            Vec::new(),
        )
    }

    fn handle_list_sora_subscribers(&self, request_id: &str) -> CommandResult {
        let response_text = crate::obsws::response::build_request_response_success(
            "ListSoraSubscribers",
            request_id,
            |f| {
                f.member(
                    "subscribers",
                    nojson::array(|f| {
                        for (name, state) in &self.sora_subscribers {
                            f.element(nojson::object(|f| {
                                f.member("subscriberName", name.as_str())?;
                                f.member("active", state.run.is_some())?;
                                f.member("settings", &state.settings)
                            }))?;
                        }
                        Ok(())
                    }),
                )
            },
        );
        self.build_result_from_response(response_text, Vec::new())
    }

    fn handle_list_sora_source_tracks(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let filter_name: Option<String> = request_data.and_then(|data| {
            data.value()
                .to_member("subscriberName")
                .ok()?
                .optional()
                .and_then(|v| v.try_into().ok())
        });

        let response_text = crate::obsws::response::build_request_response_success(
            "ListSoraSourceTracks",
            request_id,
            |f| {
                f.member(
                    "tracks",
                    nojson::array(|f| {
                        for (name, state) in &self.sora_subscribers {
                            if let Some(ref filter) = filter_name
                                && name != filter
                            {
                                continue;
                            }
                            for (track_id, rt) in &state.remote_tracks {
                                f.element(nojson::object(|f| {
                                    f.member("subscriberName", name.as_str())?;
                                    f.member("connectionId", rt.connection_id.as_str())?;
                                    f.member("clientId", rt.client_id.as_deref())?;
                                    f.member("trackId", track_id.as_str())?;
                                    f.member("trackKind", rt.track_kind.as_str())?;
                                    f.member("attachedInputName", rt.attached_input_name.as_deref())
                                }))?;
                            }
                        }
                        Ok(())
                    }),
                )
            },
        );
        self.build_result_from_response(response_text, Vec::new())
    }

    async fn handle_attach_sora_source_track(
        &mut self,
        request_type: &str,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(data) = request_data else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing requestData",
            );
        };
        let json = data.value();
        let input_name: String = match json
            .to_member("inputName")
            .and_then(|v| v.required()?.try_into())
        {
            Ok(n) => n,
            Err(_) => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_MISSING_REQUEST_FIELD,
                    "Missing inputName",
                );
            }
        };
        let connection_id: String = match json
            .to_member("connectionId")
            .and_then(|v| v.required()?.try_into())
        {
            Ok(n) => n,
            Err(_) => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_MISSING_REQUEST_FIELD,
                    "Missing connectionId",
                );
            }
        };
        let track_kind: String = match json
            .to_member("trackKind")
            .and_then(|v| v.required()?.try_into())
        {
            Ok(n) => n,
            Err(_) => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_MISSING_REQUEST_FIELD,
                    "Missing trackKind",
                );
            }
        };
        if track_kind != "video" && track_kind != "audio" {
            return self.build_error_result(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                "trackKind must be 'video' or 'audio'",
            );
        }
        let resolved = self.resolve_input_by_name(&input_name);
        let Some(resolved) = resolved else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Input not found",
            );
        };
        if resolved.input_kind != "sora_source" {
            return self.build_error_result(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                "Input is not a sora_source",
            );
        }
        let mut found: Option<(String, String)> = None;
        for (sub_name, state) in &self.sora_subscribers {
            for (tid, rt) in &state.remote_tracks {
                if rt.connection_id == connection_id && rt.track_kind == track_kind {
                    found = Some((sub_name.clone(), tid.clone()));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        let Some((sub_name, found_track_id)) = found else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "No matching remote track found",
            );
        };
        if self.sora_subscribers[&sub_name].remote_tracks[&found_track_id]
            .attached_input_name
            .is_some()
        {
            return self.build_error_result(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED,
                "Track is already attached",
            );
        }
        let pipeline_track_id = match track_kind.as_str() {
            "video" => resolved.video_track_id.clone(),
            "audio" => self
                .input_source_processors
                .get(&resolved.input_uuid)
                .and_then(|s| s.audio_track_id.clone()),
            _ => None,
        };
        let Some(pipeline_track_id) = pipeline_track_id else {
            return self.build_error_result(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                "Pipeline track not found",
            );
        };
        // pipeline から TrackPublisher を取得してフレーム転送を開始する
        if let Some(pipeline_handle) = self.pipeline_handle.as_ref() {
            let state = self
                .sora_subscribers
                .get(&sub_name)
                .expect("subscriber should exist");
            if let Some(run) = &state.run {
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                pipeline_handle.send(crate::media_pipeline::MediaPipelineCommand::PublishTrack {
                    processor_id: run.processor_id.clone(),
                    track_id: pipeline_track_id.clone(),
                    reply_tx,
                });
                match reply_rx.await {
                    Ok(Ok(publisher)) => {
                        tracing::debug!(
                            "AttachSoraSourceTrack: publish_track succeeded, track_id={}, sending Attach command",
                            pipeline_track_id
                        );
                        let rt = &state.remote_tracks[&found_track_id];
                        let _ = rt
                            .command_tx
                            .send(crate::sora_source::SoraTrackCommand::Attach { publisher });
                    }
                    Ok(Err(e)) => {
                        return self.build_error_result(
                            request_type,
                            request_id,
                            crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                            &format!("Failed to publish track: {e:?}"),
                        );
                    }
                    Err(_) => {
                        return self.build_error_result(
                            request_type,
                            request_id,
                            crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                            "Pipeline terminated during publish_track",
                        );
                    }
                }
            }
        }

        let rt = self
            .sora_subscribers
            .get_mut(&sub_name)
            .expect("BUG: subscriber not found after lookup")
            .remote_tracks
            .get_mut(&found_track_id)
            .expect("BUG: track not found after lookup");
        rt.attached_input_name = Some(input_name.clone());
        rt.attached_pipeline_track_id = Some(pipeline_track_id.clone());
        if let Some(uuid) = self.input_registry.uuids_by_name.get(&input_name)
            && let Some(entry) = self.input_registry.inputs_by_uuid.get_mut(uuid)
            && let crate::obsws::input_registry::ObswsInputSettings::SoraSource(ref mut s) =
                entry.input.settings
        {
            match track_kind.as_str() {
                "video" => s.video_track_id = Some(found_track_id.clone()),
                "audio" => s.audio_track_id = Some(found_track_id.clone()),
                _ => {}
            }
        }
        self.build_result_from_response(
            crate::obsws::response::build_request_response_success_no_data(
                request_type,
                request_id,
            ),
            Vec::new(),
        )
    }

    fn handle_detach_sora_source_track(
        &mut self,
        request_type: &str,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(data) = request_data else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing requestData",
            );
        };
        let json = data.value();
        let input_name: String = match json
            .to_member("inputName")
            .and_then(|v| v.required()?.try_into())
        {
            Ok(n) => n,
            Err(_) => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_MISSING_REQUEST_FIELD,
                    "Missing inputName",
                );
            }
        };
        let track_kind: String = match json
            .to_member("trackKind")
            .and_then(|v| v.required()?.try_into())
        {
            Ok(n) => n,
            Err(_) => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_MISSING_REQUEST_FIELD,
                    "Missing trackKind",
                );
            }
        };
        let mut found: Option<(String, String)> = None;
        for (sub_name, state) in &self.sora_subscribers {
            for (tid, rt) in &state.remote_tracks {
                if rt.attached_input_name.as_deref() == Some(&input_name)
                    && rt.track_kind == track_kind
                {
                    found = Some((sub_name.clone(), tid.clone()));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        let Some((sub_name, track_id)) = found else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "No track attached to this input with the specified trackKind",
            );
        };
        let rt = self
            .sora_subscribers
            .get_mut(&sub_name)
            .expect("BUG: subscriber not found after lookup")
            .remote_tracks
            .get_mut(&track_id)
            .expect("BUG: track not found after lookup");
        // holder タスクに Detach コマンドを送信
        let _ = rt
            .command_tx
            .send(crate::sora_source::SoraTrackCommand::Detach);
        rt.attached_input_name = None;
        rt.attached_pipeline_track_id = None;
        self.clear_sora_source_track_id(&input_name, &track_kind);
        self.build_result_from_response(
            crate::obsws::response::build_request_response_success_no_data(
                request_type,
                request_id,
            ),
            Vec::new(),
        )
    }

    fn parse_subscriber_name(data: &nojson::RawJsonOwned) -> Result<String, String> {
        let json = data.value();
        json.to_member("subscriberName")
            .and_then(|v| v.required()?.try_into())
            .map_err(|_| "Missing subscriberName field".to_string())
    }
}
