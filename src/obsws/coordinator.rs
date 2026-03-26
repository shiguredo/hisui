use crate::obsws::input_registry::ObswsInputRegistry;
use crate::obsws::message::ObswsSessionStats;
use crate::obsws::protocol::{
    OBSWS_EVENT_SUB_GENERAL, OBSWS_EVENT_SUB_INPUTS, OBSWS_EVENT_SUB_OUTPUTS,
    OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED, OBSWS_EVENT_SUB_SCENE_ITEMS,
    OBSWS_EVENT_SUB_SCENES, REQUEST_STATUS_MISSING_REQUEST_DATA,
    REQUEST_STATUS_MISSING_REQUEST_FIELD, REQUEST_STATUS_RESOURCE_NOT_FOUND,
};

use std::time::Duration;

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
    input_source_processors: std::collections::HashMap<String, InputSourceState>,
    /// bootstrap 用の差分イベント送信チャネル
    bootstrap_event_tx: tokio::sync::broadcast::Sender<BootstrapInputEvent>,
}

impl ObswsCoordinator {
    /// actor と handle を生成する。program_output の初期化は呼び出し側で行う。
    pub fn new(
        input_registry: ObswsInputRegistry,
        program_output: crate::obsws::server::ProgramOutputState,
        pipeline_handle: Option<crate::MediaPipelineHandle>,
    ) -> (Self, ObswsCoordinatorHandle) {
        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let (bootstrap_event_tx, _) = tokio::sync::broadcast::channel(64);
        let program_track_ids = ProgramTrackIds {
            video_track_id: program_output.video_track_id.clone(),
            audio_track_id: program_output.audio_track_id.clone(),
        };
        let actor = Self {
            input_registry,
            program_output,
            pipeline_handle,
            command_rx,
            input_source_processors: std::collections::HashMap::new(),
            bootstrap_event_tx: bootstrap_event_tx.clone(),
        };
        let handle = ObswsCoordinatorHandle {
            command_tx,
            program_track_ids,
            bootstrap_event_tx,
        };
        (actor, handle)
    }

    /// actor のイベントループを実行する
    pub async fn run(mut self) {
        while let Some(command) = self.command_rx.recv().await {
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
        for request in requests {
            let request_type = request.request_type.clone().unwrap_or_default();
            let result = self.dispatch_request(request, session_stats).await;
            let request_succeeded = result.batch_result.request_status_result;
            results.push(result.batch_result);
            events.extend(result.events);
            if let Err(e) = self
                .sync_program_output_state(&request_type, request_succeeded)
                .await
            {
                tracing::warn!("failed to rebuild program output: {}", e.display());
            }
            if halt_on_failure && !request_succeeded {
                break;
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
            if let Some(source_state) =
                self.input_source_processors.get(&created.input_entry.input_uuid)
            {
                let _ =
                    self.bootstrap_event_tx
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
                let _ = self.bootstrap_event_tx.send(BootstrapInputEvent::InputRemoved {
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
            events.push(TaggedEvent {
                text: crate::obsws::response::build_input_settings_changed_event(
                    &updated_input.input_name,
                    &updated_input.input_uuid,
                    &updated_input.input.settings,
                ),
                subscription_flag: OBSWS_EVENT_SUB_INPUTS,
            });
        }
        self.build_result_from_response(response_text, events)
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
            events.push(TaggedEvent {
                text: crate::obsws::response::build_input_name_changed_event(
                    &updated_input.input_name,
                    &old_input.input_name,
                    &updated_input.input_uuid,
                ),
                subscription_flag: OBSWS_EVENT_SUB_INPUTS,
            });
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
        let mut output_plan = match build_output_plan_or_error(
            request_type,
            request_id,
            &self.input_registry,
            crate::obsws::source::ObswsOutputKind::Stream,
            run_id,
        ) {
            Ok(plan) => plan,
            Err(outcome) => return outcome,
        };
        let video =
            ObswsRecordTrackRun::new("stream", run_id, "video", &output_plan.video_track_id);
        let audio =
            ObswsRecordTrackRun::new("stream", run_id, "audio", &output_plan.audio_track_id);
        let run = ObswsStreamRun {
            source_processor_ids: output_plan.source_processor_ids.clone(),
            video,
            audio,
            audio_mixer_processor_id: output_plan.audio_mixer_processor_id.clone(),
            video_mixer_processor_id: output_plan.video_mixer_processor_id.clone(),
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
        if let Err(e) = start_stream_processors(
            pipeline_handle,
            &mut output_plan,
            &output_url,
            stream_service_settings.key.as_deref(),
            &run,
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
        let mut output_plan = match build_output_plan_or_error(
            request_type,
            request_id,
            &self.input_registry,
            crate::obsws::source::ObswsOutputKind::Record,
            run_id,
        ) {
            Ok(plan) => plan,
            Err(outcome) => return outcome,
        };
        let video =
            ObswsRecordTrackRun::new("record", run_id, "video", &output_plan.video_track_id);
        let audio =
            ObswsRecordTrackRun::new("record", run_id, "audio", &output_plan.audio_track_id);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis();
        let output_path = self
            .input_registry
            .record_directory()
            .join(format!("obsws-record-{timestamp}.mp4"));
        let run = ObswsRecordRun {
            source_processor_ids: output_plan.source_processor_ids.clone(),
            video,
            audio,
            audio_mixer_processor_id: output_plan.audio_mixer_processor_id.clone(),
            video_mixer_processor_id: output_plan.video_mixer_processor_id.clone(),
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
        if let Err(e) =
            start_record_processors(pipeline_handle, &mut output_plan, &output_path, &run).await
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
        let mut output_plan = match build_output_plan_or_error(
            request_type,
            request_id,
            &self.input_registry,
            crate::obsws::source::ObswsOutputKind::RtmpOutbound,
            run_id,
        ) {
            Ok(plan) => plan,
            Err(outcome) => return outcome,
        };
        let video = ObswsRecordTrackRun::new(
            "rtmp_outbound",
            run_id,
            "video",
            &output_plan.video_track_id,
        );
        let audio = ObswsRecordTrackRun::new(
            "rtmp_outbound",
            run_id,
            "audio",
            &output_plan.audio_track_id,
        );
        let run = ObswsRtmpOutboundRun {
            source_processor_ids: output_plan.source_processor_ids.clone(),
            video,
            audio,
            audio_mixer_processor_id: output_plan.audio_mixer_processor_id.clone(),
            video_mixer_processor_id: output_plan.video_mixer_processor_id.clone(),
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
        if let Err(e) = start_rtmp_outbound_processors(
            pipeline_handle,
            &mut output_plan,
            &output_url,
            rtmp_outbound_settings.stream_name.as_deref(),
            &run,
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

        crate::obsws::session::output::start_source_processors(
            pipeline_handle,
            &mut vec![source_plan],
        )
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
        crate::obsws::session::output::stop_source_processors(
            pipeline_handle,
            &state.processor_ids,
        )
        .await
    }

    /// 初期入力に対して source processor を一括起動する
    pub async fn start_initial_input_source_processors(&mut self) -> crate::Result<()> {
        let entries: Vec<_> = self.input_registry.inputs_by_uuid.values().cloned().collect();
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

fn build_output_plan_or_error(
    request_type: &str,
    request_id: &str,
    input_registry: &ObswsInputRegistry,
    output_kind: crate::obsws::source::ObswsOutputKind,
    run_id: u64,
) -> Result<crate::obsws::output_plan::ObswsComposedOutputPlan, OutputOperationOutcome> {
    let scene_inputs = input_registry.list_current_program_scene_input_entries();
    crate::obsws::output_plan::build_composed_output_plan(
        &scene_inputs,
        output_kind,
        run_id,
        input_registry.canvas_width(),
        input_registry.canvas_height(),
        input_registry.frame_rate(),
    )
    .map_err(|error| OutputOperationOutcome {
        response_text: crate::obsws::response::build_request_response_error(
            request_type,
            request_id,
            crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
            &error.message(),
        ),
        success: false,
        output_path: None,
    })
}

// -----------------------------------------------------------------------
// Output プロセッサ起動/停止 自由関数
// -----------------------------------------------------------------------

/// ストリーム用プロセッサを起動する: ミキサー → エンコーダー → パブリッシャー → ソース
async fn start_stream_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    output_plan: &mut crate::obsws::output_plan::ObswsComposedOutputPlan,
    output_url: &str,
    stream_key: Option<&str>,
    run: &crate::obsws::input_registry::ObswsStreamRun,
) -> crate::Result<()> {
    // ミキサーを起動する（pub 関数を利用）
    crate::obsws::session::output::start_mixer_processors(pipeline_handle, output_plan).await?;
    // ビデオエンコーダーを起動する
    crate::encoder::create_video_processor(
        pipeline_handle,
        run.video.source_track_id.clone(),
        run.video.encoded_track_id.clone(),
        crate::types::CodecName::H264,
        std::num::NonZeroUsize::new(2_000_000).unwrap(),
        output_plan.frame_rate,
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
    // ソースプロセッサを起動する
    crate::obsws::session::output::start_source_processors(
        pipeline_handle,
        &mut output_plan.source_plans,
    )
    .await?;
    Ok(())
}

/// レコード用プロセッサを起動する: ミキサー → エンコーダー → MP4 ライター → ソース
async fn start_record_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    output_plan: &mut crate::obsws::output_plan::ObswsComposedOutputPlan,
    output_path: &std::path::Path,
    run: &crate::obsws::input_registry::ObswsRecordRun,
) -> crate::Result<()> {
    crate::obsws::session::output::start_mixer_processors(pipeline_handle, output_plan).await?;
    crate::encoder::create_video_processor(
        pipeline_handle,
        run.video.source_track_id.clone(),
        run.video.encoded_track_id.clone(),
        crate::types::CodecName::H264,
        std::num::NonZeroUsize::new(2_000_000).unwrap(),
        output_plan.frame_rate,
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
    crate::obsws::session::output::start_source_processors(
        pipeline_handle,
        &mut output_plan.source_plans,
    )
    .await?;
    Ok(())
}

/// RTMP outbound 用プロセッサを起動する
async fn start_rtmp_outbound_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    output_plan: &mut crate::obsws::output_plan::ObswsComposedOutputPlan,
    output_url: &str,
    stream_name: Option<&str>,
    run: &crate::obsws::input_registry::ObswsRtmpOutboundRun,
) -> crate::Result<()> {
    crate::obsws::session::output::start_mixer_processors(pipeline_handle, output_plan).await?;
    crate::encoder::create_video_processor(
        pipeline_handle,
        run.video.source_track_id.clone(),
        run.video.encoded_track_id.clone(),
        crate::types::CodecName::H264,
        std::num::NonZeroUsize::new(2_000_000).unwrap(),
        output_plan.frame_rate,
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
    crate::obsws::session::output::start_source_processors(
        pipeline_handle,
        &mut output_plan.source_plans,
    )
    .await?;
    Ok(())
}

/// ストリーム用プロセッサを段階的に停止する: ソース → ミキサー → エンコーダー → パブリッシャー
async fn stop_processors_staged_stream(
    pipeline_handle: &crate::MediaPipelineHandle,
    run: &crate::obsws::input_registry::ObswsStreamRun,
) -> crate::Result<()> {
    terminate_and_wait(pipeline_handle, &run.source_processor_ids).await?;
    terminate_and_wait(
        pipeline_handle,
        &[
            run.audio_mixer_processor_id.clone(),
            run.video_mixer_processor_id.clone(),
        ],
    )
    .await?;
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
/// ミキサーには Finish RPC を送信して EOS を発行させ、下流は EOS 伝播で自然終了させる。
async fn stop_processors_staged_record(
    pipeline_handle: &crate::MediaPipelineHandle,
    run: &crate::obsws::input_registry::ObswsRecordRun,
) -> crate::Result<()> {
    // 1. ソースを停止
    terminate_and_wait(pipeline_handle, &run.source_processor_ids).await?;

    // 2. ミキサーに Finish RPC を送り、自然終了を試みる
    let mixer_ids = vec![
        run.audio_mixer_processor_id.clone(),
        run.video_mixer_processor_id.clone(),
    ];
    // まずソース停止後の自然終了を短時間待つ
    if wait_processors_stopped(pipeline_handle, &mixer_ids, Duration::from_secs(1))
        .await
        .is_err()
    {
        // Finish RPC を送信して EOS 発行を要求する
        finish_mixer_rpc(pipeline_handle, &run.audio_mixer_processor_id, true).await;
        finish_mixer_rpc(pipeline_handle, &run.video_mixer_processor_id, false).await;
        // Finish 後の終了を待ち、タイムアウトなら強制停止
        if wait_processors_stopped(pipeline_handle, &mixer_ids, Duration::from_secs(5))
            .await
            .is_err()
        {
            let live = live_processor_ids(pipeline_handle, &mixer_ids).await;
            if !live.is_empty() {
                terminate_and_wait(pipeline_handle, &live).await?;
            }
        }
    }

    // 3. エンコーダーは EOS 伝播での自然終了を優先し、残れば強制停止
    wait_or_terminate(
        pipeline_handle,
        &[
            run.video.encoder_processor_id.clone(),
            run.audio.encoder_processor_id.clone(),
        ],
        Duration::from_secs(5),
    )
    .await?;

    // 4. ライターは finalize を優先し、残れば強制停止
    wait_or_terminate(
        pipeline_handle,
        std::slice::from_ref(&run.writer_processor_id),
        Duration::from_secs(5),
    )
    .await?;

    Ok(())
}

/// RTMP outbound 用プロセッサを段階的に停止する
async fn stop_processors_staged_rtmp_outbound(
    pipeline_handle: &crate::MediaPipelineHandle,
    run: &crate::obsws::input_registry::ObswsRtmpOutboundRun,
) -> crate::Result<()> {
    terminate_and_wait(pipeline_handle, &run.source_processor_ids).await?;
    terminate_and_wait(
        pipeline_handle,
        &[
            run.audio_mixer_processor_id.clone(),
            run.video_mixer_processor_id.clone(),
        ],
    )
    .await?;
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

/// ミキサーに Finish RPC を送信する。失敗時は terminate にフォールバックする。
async fn finish_mixer_rpc(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_id: &crate::ProcessorId,
    is_audio: bool,
) {
    const RETRY_TIMEOUT: Duration = Duration::from_millis(500);
    const RETRY_INTERVAL: Duration = Duration::from_millis(10);
    let deadline = tokio::time::Instant::now() + RETRY_TIMEOUT;

    if is_audio {
        loop {
            match pipeline_handle
                .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<
                    crate::mixer::audio::AudioRealtimeMixerRpcMessage,
                >>(processor_id)
                .await
            {
                Ok(sender) => {
                    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                    let _ =
                        sender.send(crate::mixer::audio::AudioRealtimeMixerRpcMessage::Finish {
                            reply_tx,
                        });
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
    } else {
        loop {
            match pipeline_handle
                .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<
                    crate::mixer::video::VideoRealtimeMixerRpcMessage,
                >>(processor_id)
                .await
            {
                Ok(sender) => {
                    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                    let _ =
                        sender.send(crate::mixer::video::VideoRealtimeMixerRpcMessage::Finish {
                            reply_tx,
                        });
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
}
