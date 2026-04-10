//! Sora WebRTC の output エンジンおよび subscriber ハンドラ。
//! Publisher は Program 出力を sora-rust-sdk 経由で Sora に SendOnly 配信する。
//! Subscriber は Sora から WebRTC トラックを受信し、input に attach/detach する。
//! WebRTC の型 (RtpTransceiver, VideoSink 等) は !Sync のため coordinator に直接保持できないため、
//! 実際のフレーム転送は coordinator の外のタスクで管理し、coordinator はメタデータのみ保持する。

use super::output::{OutputOperationOutcome, terminate_and_wait};
use super::{CommandResult, ObswsCoordinator};
use crate::obsws::event::TaggedEvent;
use crate::obsws::protocol::{
    OBSWS_EVENT_SUB_SORA_SOURCE, REQUEST_STATUS_MISSING_REQUEST_DATA,
    REQUEST_STATUS_MISSING_REQUEST_FIELD, REQUEST_STATUS_RESOURCE_NOT_FOUND,
};

/// SoraSubscriber の状態
pub(crate) struct SoraSubscriberState {
    pub(crate) settings: crate::obsws::state::ObswsSoraSubscriberSettings,
    pub(crate) run: Option<SoraSubscriberRun>,
    /// 受信中のリモートトラック（trackId → トラック情報）
    pub(crate) remote_tracks: std::collections::HashMap<String, SoraSourceRemoteTrack>,
    /// on_notify から抽出した接続情報（connection_id → info）
    pub(crate) connections: std::collections::HashMap<String, SoraConnectionInfo>,
}

/// 実行中の SoraSubscriber の情報
#[derive(Clone)]
pub(crate) struct SoraSubscriberRun {
    pub(crate) processor_id: crate::ProcessorId,
}

/// SoraSubscriber から受信したリモートトラックのメタデータ。
///
/// WebRTC の型（RtpTransceiver, VideoSink 等）は !Sync のため coordinator に直接保持できない。
/// 実際のフレーム転送は coordinator の外のタスクで管理し、coordinator はメタデータのみ保持する。
pub(crate) struct SoraSourceRemoteTrack {
    pub(crate) connection_id: String,
    pub(crate) client_id: Option<String>,
    pub(crate) track_kind: String,
    /// attach 先の input 名
    pub(crate) attached_input_name: Option<String>,
    /// attach 先の pipeline track ID
    pub(crate) attached_pipeline_track_id: Option<crate::TrackId>,
    /// holder タスクへのコマンド送信チャネル（Send+Sync）
    pub(crate) command_tx: tokio::sync::mpsc::UnboundedSender<crate::sora_source::SoraTrackCommand>,
    /// holder タスクの停止用ハンドル
    pub(crate) holder_abort: tokio::task::AbortHandle,
}

impl Drop for SoraSourceRemoteTrack {
    fn drop(&mut self) {
        self.holder_abort.abort();
    }
}

/// SoraSubscriber の on_notify から抽出した接続情報
pub(crate) struct SoraConnectionInfo {
    pub(crate) client_id: Option<String>,
}

// -----------------------------------------------------------------------
// SoraOutputSettings: sora output の種別固有設定
// -----------------------------------------------------------------------

/// Sora publisher output の設定。
/// `ObswsSoraPublisherSettings` と同一フィールドを持ち、output_registry の enum から委譲される。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct SoraOutputSettings {
    pub(crate) signaling_urls: Vec<String>,
    pub(crate) channel_id: Option<String>,
    pub(crate) client_id: Option<String>,
    pub(crate) bundle_id: Option<String>,
    pub(crate) metadata: Option<nojson::RawJsonOwned>,
}

impl From<crate::obsws::state::ObswsSoraPublisherSettings> for SoraOutputSettings {
    fn from(s: crate::obsws::state::ObswsSoraPublisherSettings) -> Self {
        Self {
            signaling_urls: s.signaling_urls,
            channel_id: s.channel_id,
            client_id: s.client_id,
            bundle_id: s.bundle_id,
            metadata: s.metadata,
        }
    }
}

impl From<SoraOutputSettings> for crate::obsws::state::ObswsSoraPublisherSettings {
    fn from(s: SoraOutputSettings) -> Self {
        Self {
            signaling_urls: s.signaling_urls,
            channel_id: s.channel_id,
            client_id: s.client_id,
            bundle_id: s.bundle_id,
            metadata: s.metadata,
        }
    }
}

impl nojson::DisplayJson for SoraOutputSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member(
                "soraSdkSettings",
                nojson::object(|f| {
                    if !self.signaling_urls.is_empty() {
                        f.member("signalingUrls", &self.signaling_urls)?;
                    }
                    if let Some(channel_id) = &self.channel_id {
                        f.member("channelId", channel_id)?;
                    }
                    if let Some(client_id) = &self.client_id {
                        f.member("clientId", client_id)?;
                    }
                    if let Some(bundle_id) = &self.bundle_id {
                        f.member("bundleId", bundle_id)?;
                    }
                    if let Some(metadata) = &self.metadata {
                        f.member("metadata", metadata)?;
                    }
                    Ok(())
                }),
            )
        })
        .fmt(f)
    }
}

impl SoraOutputSettings {
    /// JSON から設定を更新する（SetOutputSettings 用）。
    /// soraSdkSettings オブジェクトの中の各フィールドを更新する。
    pub(crate) fn update_from_json(
        &mut self,
        output_settings: &nojson::RawJsonValue<'_, '_>,
    ) -> Result<(), String> {
        if let Ok(v) = output_settings.to_member("soraSdkSettings")
            && let Some(sdk) = v.optional()
            && !sdk.kind().is_null()
        {
            if let Ok(v) = sdk.to_member("signalingUrls")
                && let Some(v) = v.optional()
            {
                if v.kind().is_null() {
                    self.signaling_urls = Vec::new();
                } else {
                    match <Vec<String>>::try_from(v) {
                        Ok(urls) => self.signaling_urls = urls,
                        Err(_) => {
                            return Err("signalingUrls must be an array of strings".to_owned());
                        }
                    }
                }
            }
            if let Ok(v) = sdk.to_member("channelId")
                && let Some(v) = v.optional()
            {
                if v.kind().is_null() {
                    self.channel_id = None;
                } else {
                    match <String>::try_from(v) {
                        Ok(ch) => self.channel_id = Some(ch),
                        Err(_) => return Err("channelId must be a string".to_owned()),
                    }
                }
            }
            if let Ok(v) = sdk.to_member("clientId")
                && let Some(v) = v.optional()
            {
                if v.kind().is_null() {
                    self.client_id = None;
                } else {
                    match <String>::try_from(v) {
                        Ok(ci) => self.client_id = Some(ci),
                        Err(_) => return Err("clientId must be a string".to_owned()),
                    }
                }
            }
            if let Ok(v) = sdk.to_member("bundleId")
                && let Some(v) = v.optional()
            {
                if v.kind().is_null() {
                    self.bundle_id = None;
                } else {
                    match <String>::try_from(v) {
                        Ok(bi) => self.bundle_id = Some(bi),
                        Err(_) => return Err("bundleId must be a string".to_owned()),
                    }
                }
            }
            if let Ok(v) = sdk.to_member("metadata")
                && let Some(v) = v.optional()
            {
                if v.kind().is_null() {
                    self.metadata = None;
                } else if !v.kind().is_object() {
                    return Err("metadata must be an object".to_owned());
                } else {
                    self.metadata = Some(v.extract().into_owned());
                }
            }
        }
        Ok(())
    }

    /// JSON から設定をパースする（HisuiCreateOutput / state file 復元用）。
    pub(crate) fn parse_from_json(
        settings_value: Option<&nojson::RawJsonValue<'_, '_>>,
    ) -> Result<Self, String> {
        use super::output_registry::parse_optional_string_strict;

        let mut settings = Self::default();
        if let Some(v) = settings_value {
            let sdk = v
                .to_member("soraSdkSettings")
                .ok()
                .and_then(|v| v.optional())
                .filter(|v| !v.kind().is_null());
            let source = sdk.as_ref().unwrap_or(v);
            // signalingUrls
            if let Ok(member) = source.to_member("signalingUrls")
                && let Some(val) = member.optional()
                && !val.kind().is_null()
            {
                settings.signaling_urls = <Vec<String>>::try_from(val)
                    .map_err(|_| "signalingUrls must be an array of strings".to_owned())?;
            }
            settings.channel_id =
                parse_optional_string_strict(source, "channelId", "channelId must be a string")?;
            settings.client_id =
                parse_optional_string_strict(source, "clientId", "clientId must be a string")?;
            settings.bundle_id =
                parse_optional_string_strict(source, "bundleId", "bundleId must be a string")?;
            // metadata（object のみ）
            if let Ok(member) = source.to_member("metadata")
                && let Some(val) = member.optional()
                && !val.kind().is_null()
            {
                if !val.kind().is_object() {
                    return Err("metadata must be an object".to_owned());
                }
                settings.metadata = Some(val.extract().into_owned());
            }
        }
        Ok(settings)
    }
}

impl ObswsCoordinator {
    // --- Sora Publisher 操作 ---
    // `sora` は OBS の `stream` を拡張したものではなく、Program 出力の raw frame を
    // `sora-rust-sdk` に直接渡す専用 Output として扱う。

    /// 指定された output_name の sora publisher output を開始する。
    pub(crate) async fn handle_start_sora_publisher(
        &mut self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> OutputOperationOutcome {
        use super::output_registry::{OutputRun, OutputSettings};
        use crate::obsws::state::ObswsSoraPublisherRun;

        let Some(output) = self.outputs.get(output_name) else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Output not found",
                ),
            );
        };
        let OutputSettings::Sora(sora_settings) = &output.settings else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Output is not a sora output",
                ),
            );
        };
        let sora_settings = sora_settings.clone();

        if output.runtime.active {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_RUNNING,
                    "Sora publisher is already active",
                ),
            );
        }

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

        let run_id = self.next_output_run_id;
        self.next_output_run_id = self.next_output_run_id.wrapping_add(1);

        let publisher_processor_id =
            crate::ProcessorId::new(format!("output:{output_name}:publisher:{run_id}"));
        let run = ObswsSoraPublisherRun {
            publisher_processor_id: publisher_processor_id.clone(),
        };

        if let Some(output) = self.outputs.get_mut(output_name) {
            output.runtime.active = true;
            output.runtime.started_at = Some(std::time::Instant::now());
            output.runtime.run = Some(OutputRun::Sora(run.clone()));
        }

        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            if let Some(output) = self.outputs.get_mut(output_name) {
                output.runtime.active = false;
                output.runtime.started_at = None;
                output.runtime.run = None;
            }
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
            if let Some(output) = self.outputs.get_mut(output_name) {
                output.runtime.active = false;
                output.runtime.started_at = None;
                output.runtime.run = None;
            }
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

    /// 指定された output_name の sora publisher output を停止する。
    pub(crate) async fn handle_stop_sora_publisher(
        &mut self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> OutputOperationOutcome {
        use super::output_registry::OutputRun;

        let run = self
            .outputs
            .get(output_name)
            .and_then(|o| o.runtime.run.as_ref())
            .and_then(|r| match r {
                OutputRun::Sora(run) => Some(run.clone()),
                _ => None,
            });
        let Some(run) = run else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                    "Sora publisher is not active",
                ),
            );
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
        if let Some(output) = self.outputs.get_mut(output_name) {
            output.runtime.active = false;
            output.runtime.started_at = None;
            output.runtime.run = None;
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_output_response(request_id),
            None,
        )
    }

    // --- SoraSubscriber / sora_source ハンドラ ---

    pub(crate) fn handle_sora_source_event(&mut self, event: crate::sora_source::SoraSourceEvent) {
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

    pub(crate) fn clear_sora_source_track_id(&mut self, input_name: &str, track_kind: &str) {
        if let Some(uuid) = self.state.uuids_by_name.get(input_name)
            && let Some(entry) = self.state.inputs_by_uuid.get_mut(uuid)
            && let crate::obsws::state::ObswsInputSettings::SoraSource(ref mut s) =
                entry.input.settings
        {
            match track_kind {
                "video" => s.video_track_id = None,
                "audio" => s.audio_track_id = None,
                _ => {}
            }
        }
    }

    pub(crate) async fn handle_start_sora_subscriber(
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
        let settings = crate::obsws::state::ObswsSoraSubscriberSettings {
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
            self.sora_subscribers.remove(&subscriber_name);
            return self.build_error_result(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                "Pipeline is not initialized",
            );
        };
        let processor_id =
            crate::ProcessorId::new(format!("output:sora_subscriber:{}", subscriber_name));
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
            self.sora_subscribers.remove(&subscriber_name);
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

    pub(crate) async fn handle_stop_sora_subscriber(
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
        if !self.sora_subscribers.contains_key(&subscriber_name) {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Subscriber not found",
            );
        }
        // subscriber を削除して所有権を取得する
        let mut removed_state = self
            .sora_subscribers
            .remove(&subscriber_name)
            .expect("BUG: subscriber existence was just verified");
        // run が Some の場合のみ processor を停止する
        // （Disconnected で run = None になった場合はスキップ）
        if let Some(run) = removed_state.run.take()
            && let Some(pipeline_handle) = self.pipeline_handle.as_ref()
            && let Err(e) =
                terminate_and_wait(pipeline_handle, std::slice::from_ref(&run.processor_id)).await
        {
            tracing::warn!("failed to stop sora subscriber processor: {}", e.display());
        }
        // remote_tracks をクリーンアップし、各トラックの Unpublished イベントを送信する
        for (track_id, rt) in removed_state.remote_tracks {
            rt.holder_abort.abort();
            if let Some(input_name) = &rt.attached_input_name {
                self.clear_sora_source_track_id(input_name, &rt.track_kind);
            }
            let event = crate::obsws::response::build_sora_source_track_unpublished_event(
                &subscriber_name,
                &rt.connection_id,
                &rt.track_kind,
                &track_id,
            );
            let _ = self.obsws_event_tx.send(TaggedEvent {
                text: event,
                subscription_flag: OBSWS_EVENT_SUB_SORA_SOURCE,
            });
        }
        self.build_result_from_response(
            crate::obsws::response::build_request_response_success_no_data(
                request_type,
                request_id,
            ),
            Vec::new(),
        )
    }

    pub(crate) fn handle_list_sora_subscribers(&self, request_id: &str) -> CommandResult {
        let response_text = crate::obsws::response::build_request_response_success(
            "HisuiListSoraSubscribers",
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

    pub(crate) fn handle_list_sora_source_tracks(
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
            "HisuiListSoraSourceTracks",
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

    pub(crate) async fn handle_attach_sora_source_track(
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
                            "HisuiAttachSoraSourceTrack: publish_track succeeded, track_id={}, sending Attach command",
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
        if let Some(uuid) = self.state.uuids_by_name.get(&input_name)
            && let Some(entry) = self.state.inputs_by_uuid.get_mut(uuid)
            && let crate::obsws::state::ObswsInputSettings::SoraSource(ref mut s) =
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

    pub(crate) fn handle_detach_sora_source_track(
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

    pub(crate) fn parse_subscriber_name(data: &nojson::RawJsonOwned) -> Result<String, String> {
        let json = data.value();
        json.to_member("subscriberName")
            .and_then(|v| v.required()?.try_into())
            .map_err(|_| "Missing subscriberName field".to_string())
    }
}
