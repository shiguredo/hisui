use std::sync::Arc;

use shiguredo_webrtc::{
    AdaptedVideoTrackSource, CxxString, DataChannel, DataChannelInit, DataChannelObserver,
    DataChannelObserverHandler, DataChannelState, IceGatheringState, PeerConnection,
    PeerConnectionDependencies, PeerConnectionFactory, PeerConnectionObserver,
    PeerConnectionObserverHandler, PeerConnectionRtcConfiguration, PeerConnectionState, RtpSender,
    StringVector,
};
use tokio::sync::mpsc;

use crate::obsws::session::{ObswsSession, SessionAction};

enum PcEvent {
    ConnectionChange(PeerConnectionState),
    DataChannel(DataChannel),
    DataChannelStateChange {
        label: String,
    },
    DcMessage {
        data: Vec<u8>,
    },
    ObswsMessage {
        data: Vec<u8>,
    },
    TrackMessage {
        track_id: crate::TrackId,
        message: crate::Message,
    },
    BootstrapInputCreated(crate::obsws::coordinator::BootstrapInputSnapshot),
    BootstrapInputRemoved {
        input_uuid: String,
    },
}

enum IceObserverEvent {
    Candidate {
        sdp_mid: String,
        sdp_mline_index: i32,
        candidate: String,
    },
    Complete,
}

#[derive(Clone)]
struct GatheredIceCandidate {
    sdp_mid: String,
    sdp_mline_index: i32,
    candidate: String,
}

struct SignalingMessage {
    msg_type: String,
    sdp: Option<String>,
}

fn parse_signaling_message(data: &[u8]) -> Option<SignalingMessage> {
    let text = std::str::from_utf8(data).ok()?;
    let json = nojson::RawJson::parse(text).ok()?;
    let v = json.value();
    let msg_type: String = v
        .to_member("type")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    let sdp: Option<String> = v.to_member("sdp").ok()?.try_into().ok()?;
    Some(SignalingMessage { msg_type, sdp })
}

fn make_close_json(code: &str, reason: &str) -> String {
    nojson::object(|f| {
        f.member("type", "close")?;
        f.member("code", code)?;
        f.member("reason", reason)
    })
    .to_string()
}

fn make_offer_json(sdp: &str) -> String {
    nojson::object(|f| {
        f.member("type", "offer")?;
        f.member("sdp", sdp)
    })
    .to_string()
}

struct P2pPcObserverHandler {
    event_tx: mpsc::UnboundedSender<PcEvent>,
    ice_tx: tokio::sync::mpsc::UnboundedSender<IceObserverEvent>,
}

impl PeerConnectionObserverHandler for P2pPcObserverHandler {
    fn on_connection_change(&mut self, state: PeerConnectionState) {
        let _ = self.event_tx.send(PcEvent::ConnectionChange(state));
    }

    fn on_data_channel(&mut self, dc: DataChannel) {
        let _ = self.event_tx.send(PcEvent::DataChannel(dc));
    }

    fn on_ice_gathering_change(&mut self, state: IceGatheringState) {
        if state == IceGatheringState::Complete {
            let _ = self.ice_tx.send(IceObserverEvent::Complete);
        }
    }

    fn on_ice_candidate(&mut self, candidate: shiguredo_webrtc::IceCandidateRef<'_>) {
        let Ok(sdp_mid) = candidate.sdp_mid() else {
            return;
        };
        let sdp_mline_index = candidate.sdp_mline_index();
        let Ok(candidate) = candidate.to_string() else {
            return;
        };
        let _ = self.ice_tx.send(IceObserverEvent::Candidate {
            sdp_mid,
            sdp_mline_index,
            candidate,
        });
    }
}

struct DcMessageHandler {
    event_tx: mpsc::UnboundedSender<PcEvent>,
    label: &'static str,
}

impl DataChannelObserverHandler for DcMessageHandler {
    fn on_state_change(&mut self) {
        let _ = self.event_tx.send(PcEvent::DataChannelStateChange {
            label: self.label.to_owned(),
        });
    }

    fn on_message(&mut self, data: &[u8], _is_binary: bool) {
        let _ = self.event_tx.send(PcEvent::DcMessage {
            data: data.to_vec(),
        });
    }
}

struct ObswsMessageHandler {
    event_tx: mpsc::UnboundedSender<PcEvent>,
    label: &'static str,
}

impl DataChannelObserverHandler for ObswsMessageHandler {
    fn on_state_change(&mut self) {
        let _ = self.event_tx.send(PcEvent::DataChannelStateChange {
            label: self.label.to_owned(),
        });
    }

    fn on_message(&mut self, data: &[u8], _is_binary: bool) {
        let _ = self.event_tx.send(PcEvent::ObswsMessage {
            data: data.to_vec(),
        });
    }
}

struct Session {
    _handle: crate::MediaPipelineHandle,
    processor_handle: crate::ProcessorHandle,
    factory: Arc<PeerConnectionFactory>,
    audio_state: Arc<super::audio::SharedAudioState>,
    pc: PeerConnection,
    _pc_observer: shiguredo_webrtc::PeerConnectionObserver,
    signaling_dc: Option<DataChannel>,
    obsws_dc: Option<DataChannel>,
    _dc_observer: Option<shiguredo_webrtc::DataChannelObserver>,
    _obsws_dc_observer: Option<shiguredo_webrtc::DataChannelObserver>,
    connection_state: PeerConnectionState,
    in_flight_offer: bool,
    pending_renegotiation: bool,
    subscribed_tracks: std::collections::HashMap<crate::TrackId, SubscribedTrack>,
    event_tx: mpsc::UnboundedSender<PcEvent>,
    ice_rx: tokio::sync::mpsc::UnboundedReceiver<IceObserverEvent>,
    ice_candidates: Vec<GatheredIceCandidate>,
    obsws_session: ObswsSession,
    /// bootstrap の input_uuid → snapshot マッピング
    bootstrap_tracks:
        std::collections::HashMap<String, crate::obsws::coordinator::BootstrapInputSnapshot>,
    /// bootstrap 差分イベント購読タスクの停止用ハンドル
    bootstrap_event_abort_handle: Option<tokio::task::AbortHandle>,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(handle) = &self.bootstrap_event_abort_handle {
            handle.abort();
        }
        tracing::info!("Closing PeerConnection");
        self.pc.close();
    }
}

struct SubscribedTrack {
    state: TrackState,
    sender: RtpSender,
    abort_handle: tokio::task::AbortHandle,
}

enum TrackState {
    Video(VideoTrackState),
    Audio(AudioTrackState),
}

struct VideoTrackState {
    source: AdaptedVideoTrackSource,
    _track: shiguredo_webrtc::VideoTrack,
}

struct AudioTrackState {
    audio_state: Arc<super::audio::SharedAudioState>,
    _source: shiguredo_webrtc::AudioTrackSource,
    _track: shiguredo_webrtc::AudioTrack,
}

pub enum BootstrapError {
    SessionAlreadyExists,
    Internal(crate::Error),
}

pub struct WebRtcP2pSessionManager {
    factory_bundle: Arc<super::factory::WebRtcFactoryBundle>,
    pipeline_handle: crate::MediaPipelineHandle,
    coordinator_handle: crate::obsws::coordinator::ObswsCoordinatorHandle,
    session: Arc<tokio::sync::Mutex<Option<Session>>>,
    event_tx: mpsc::UnboundedSender<PcEvent>,
}

impl WebRtcP2pSessionManager {
    pub fn new(
        handle: crate::MediaPipelineHandle,
        coordinator_handle: crate::obsws::coordinator::ObswsCoordinatorHandle,
    ) -> crate::Result<Self> {
        #[allow(clippy::arc_with_non_send_sync)]
        let factory_bundle = Arc::new(super::factory::WebRtcFactoryBundle::new()?);
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<PcEvent>();
        let session: Arc<tokio::sync::Mutex<Option<Session>>> =
            Arc::new(tokio::sync::Mutex::new(None));

        let session_for_events = session.clone();
        tokio::task::spawn_local(async move {
            while let Some(event) = event_rx.recv().await {
                let mut guard = session_for_events.lock().await;
                let Some(sess) = guard.as_mut() else {
                    continue;
                };
                match event {
                    PcEvent::ConnectionChange(state) => {
                        tracing::info!("PeerConnection state changed: {state:?}");
                        sess.connection_state = state;
                        if state == PeerConnectionState::Connected {
                            // 接続確立時に bootstrap-tracks snapshot を送信する
                            let snapshot_entries: Vec<_> =
                                sess.bootstrap_tracks.values().cloned().collect();
                            if !snapshot_entries.is_empty() {
                                let msg = build_bootstrap_tracks_json(&snapshot_entries);
                                send_dc(sess, &msg);
                            }

                            if sess.pending_renegotiation {
                                // 接続確立後に保留中の renegotiation offer を送信する
                                if let Err(e) = maybe_send_offer(sess).await {
                                    tracing::warn!(
                                        "failed to send renegotiation offer: {}",
                                        e.display()
                                    );
                                }
                            }
                        }
                        if matches!(
                            state,
                            PeerConnectionState::Failed | PeerConnectionState::Closed
                        ) {
                            tracing::info!("Session closed");
                            *guard = None;
                        }
                    }
                    PcEvent::DataChannel(mut dc) => {
                        let label = dc.label().unwrap_or_default();
                        tracing::info!("DataChannel received: label={label}");
                        if label == "signaling" {
                            let dc_observer =
                                DataChannelObserver::new_with_handler(Box::new(DcMessageHandler {
                                    event_tx: sess.event_tx.clone(),
                                    label: "signaling",
                                }));
                            dc.register_observer(&dc_observer);
                            sess.signaling_dc = Some(dc);
                            sess._dc_observer = Some(dc_observer);
                        }
                    }
                    PcEvent::DataChannelStateChange { label } => {
                        tracing::info!("DataChannel state changed: label={label}");
                        if label == "signaling"
                            && sess.pending_renegotiation
                            && sess.connection_state == PeerConnectionState::Connected
                            && let Err(e) = maybe_send_offer(sess).await
                        {
                            tracing::warn!("failed to send renegotiation offer: {}", e.display());
                        }
                    }
                    PcEvent::DcMessage { data } => {
                        if handle_dc_message(sess, &data).await {
                            tracing::info!("Session closed");
                            *guard = None;
                        }
                    }
                    PcEvent::ObswsMessage { data } => {
                        if handle_obsws_message(sess, &data).await {
                            tracing::info!("Session closed");
                            *guard = None;
                        }
                    }
                    PcEvent::TrackMessage { track_id, message } => {
                        handle_track_message(sess, &track_id, message);
                    }
                    PcEvent::BootstrapInputCreated(snapshot) => {
                        handle_bootstrap_input_created(sess, snapshot).await;
                    }
                    PcEvent::BootstrapInputRemoved { input_uuid } => {
                        handle_bootstrap_input_removed(sess, &input_uuid).await;
                    }
                }
            }
        });

        Ok(Self {
            factory_bundle,
            pipeline_handle: handle,
            coordinator_handle,
            session,
            event_tx,
        })
    }

    pub async fn bootstrap(&self, offer_sdp: &str) -> Result<String, BootstrapError> {
        {
            let guard = self.session.lock().await;
            if guard.is_some() {
                return Err(BootstrapError::SessionAlreadyExists);
            }
        }

        let processor_handle = self
            .pipeline_handle
            .register_processor(
                crate::ProcessorId::new("webrtc_p2p_session"),
                crate::ProcessorMetadata::new("webrtc_p2p_session"),
            )
            .await
            .map_err(|e| match e {
                crate::RegisterProcessorError::DuplicateProcessorId => {
                    BootstrapError::SessionAlreadyExists
                }
                crate::RegisterProcessorError::PipelineTerminated => {
                    BootstrapError::Internal(crate::Error::new(
                        "Failed to register webrtc processor: pipeline has terminated",
                    ))
                }
            })?;

        let obsws_session = ObswsSession::new_identified(self.coordinator_handle.clone());

        let mut guard = self.session.lock().await;
        if guard.is_some() {
            drop(processor_handle);
            return Err(BootstrapError::SessionAlreadyExists);
        }

        match bootstrap_internal(
            self.factory_bundle.factory(),
            self.factory_bundle.audio_state(),
            offer_sdp,
            self.event_tx.clone(),
            self.pipeline_handle.clone(),
            processor_handle,
            obsws_session,
        )
        .await
        {
            Ok((answer_sdp, mut sess)) => {
                // 入力ソース単位のトラックを購読する
                let snapshot = self
                    .coordinator_handle
                    .get_bootstrap_snapshot()
                    .await
                    .map_err(BootstrapError::Internal)?;
                for input in &snapshot {
                    subscribe_bootstrap_input(&mut sess, input);
                }

                // bootstrap 差分イベントの購読タスクを起動する
                let mut bootstrap_rx = self.coordinator_handle.subscribe_bootstrap_events();
                let event_tx = sess.event_tx.clone();
                let bootstrap_task = tokio::spawn(async move {
                    while let Ok(event) = bootstrap_rx.recv().await {
                        let pc_event = match event {
                            crate::obsws::coordinator::BootstrapInputEvent::InputCreated(
                                snapshot,
                            ) => PcEvent::BootstrapInputCreated(snapshot),
                            crate::obsws::coordinator::BootstrapInputEvent::InputRemoved {
                                input_uuid,
                            } => PcEvent::BootstrapInputRemoved { input_uuid },
                        };
                        if event_tx.send(pc_event).is_err() {
                            break;
                        }
                    }
                });
                sess.bootstrap_event_abort_handle = Some(bootstrap_task.abort_handle());

                // トラック追加があるので pending_renegotiation を設定する。
                // 実際の offer 送信は ConnectionChange(Connected) で行う。
                if !sess.subscribed_tracks.is_empty() {
                    sess.pending_renegotiation = true;
                }

                *guard = Some(sess);
                Ok(answer_sdp)
            }
            Err(e) => Err(BootstrapError::Internal(e)),
        }
    }
}

async fn bootstrap_internal(
    factory: Arc<PeerConnectionFactory>,
    audio_state: Arc<super::audio::SharedAudioState>,
    offer_sdp: &str,
    event_tx: mpsc::UnboundedSender<PcEvent>,
    handle: crate::MediaPipelineHandle,
    processor_handle: crate::ProcessorHandle,
    obsws_session: ObswsSession,
) -> crate::Result<(String, Session)> {
    let (ice_tx, ice_rx) = tokio::sync::mpsc::unbounded_channel::<IceObserverEvent>();

    // PeerConnectionObserver の作成
    let pc_observer = PeerConnectionObserver::new_with_handler(Box::new(P2pPcObserverHandler {
        event_tx: event_tx.clone(),
        ice_tx,
    }));

    let mut deps = PeerConnectionDependencies::new(&pc_observer);
    let mut config = PeerConnectionRtcConfiguration::new();

    let pc = PeerConnection::create(factory.as_ref(), &mut config, &mut deps)
        .map_err(|e| crate::Error::new(format!("Failed to create PeerConnection: {e}")))?;

    // server 側から DataChannel を作成
    let mut dc_init = DataChannelInit::new();
    dc_init.set_ordered(true);
    dc_init.set_protocol("signaling");
    let mut signaling_dc = pc
        .create_data_channel("signaling", &mut dc_init)
        .map_err(|e| crate::Error::new(format!("Failed to create signaling DataChannel: {e}")))?;

    let mut obsws_dc_init = DataChannelInit::new();
    obsws_dc_init.set_ordered(true);
    obsws_dc_init.set_protocol("obsws");
    let mut obsws_dc = pc
        .create_data_channel("obsws", &mut obsws_dc_init)
        .map_err(|e| crate::Error::new(format!("Failed to create obsws DataChannel: {e}")))?;

    // DataChannel に observer を設定
    let dc_observer = DataChannelObserver::new_with_handler(Box::new(DcMessageHandler {
        event_tx: event_tx.clone(),
        label: "signaling",
    }));
    signaling_dc.register_observer(&dc_observer);

    // obsws 用 DataChannel に observer を設定
    let obsws_observer = DataChannelObserver::new_with_handler(Box::new(ObswsMessageHandler {
        event_tx: event_tx.clone(),
        label: "obsws",
    }));
    obsws_dc.register_observer(&obsws_observer);

    super::sdp::set_remote_offer(&pc, offer_sdp)?;
    let answer_sdp = super::sdp::create_answer_sdp(&pc)?;
    super::sdp::set_local_answer(&pc, &answer_sdp)?;
    let mut ice_candidates = Vec::new();
    let mut ice_rx = ice_rx;
    let answer_sdp = finalize_local_sdp(answer_sdp, &mut ice_rx, &mut ice_candidates).await?;

    let sess = Session {
        _handle: handle,
        processor_handle,
        factory,
        audio_state,
        pc,
        _pc_observer: pc_observer,
        signaling_dc: Some(signaling_dc),
        obsws_dc: Some(obsws_dc),
        _dc_observer: Some(dc_observer),
        _obsws_dc_observer: Some(obsws_observer),
        connection_state: PeerConnectionState::New,
        in_flight_offer: false,
        pending_renegotiation: false,
        subscribed_tracks: std::collections::HashMap::new(),
        event_tx,
        ice_rx,
        ice_candidates,
        obsws_session,
        bootstrap_tracks: std::collections::HashMap::new(),
        bootstrap_event_abort_handle: None,
    };

    Ok((answer_sdp, sess))
}

async fn finalize_local_sdp(
    initial_sdp: String,
    ice_rx: &mut tokio::sync::mpsc::UnboundedReceiver<IceObserverEvent>,
    cached_candidates: &mut Vec<GatheredIceCandidate>,
) -> crate::Result<String> {
    if initial_sdp.contains("\r\na=candidate:") {
        return Ok(initial_sdp);
    }

    let mut candidates = Vec::new();
    let mut complete = false;
    // まずノンブロッキングで既に到着しているイベントを処理する
    while let Ok(event) = ice_rx.try_recv() {
        match event {
            IceObserverEvent::Candidate {
                sdp_mid,
                sdp_mline_index,
                candidate,
            } => {
                candidates.push(GatheredIceCandidate {
                    sdp_mid,
                    sdp_mline_index,
                    candidate,
                });
            }
            IceObserverEvent::Complete => {
                complete = true;
            }
        }
    }

    if !complete && candidates.is_empty() && !cached_candidates.is_empty() {
        return Ok(append_ice_candidates_to_sdp(
            &initial_sdp,
            cached_candidates,
        ));
    }

    // タイムアウト付きで ICE gathering 完了を待機する
    let timeout_duration = std::time::Duration::from_secs(5);
    let deadline = tokio::time::Instant::now() + timeout_duration;
    while !complete {
        match tokio::time::timeout_at(deadline, ice_rx.recv()).await {
            Ok(Some(IceObserverEvent::Candidate {
                sdp_mid,
                sdp_mline_index,
                candidate,
            })) => {
                candidates.push(GatheredIceCandidate {
                    sdp_mid,
                    sdp_mline_index,
                    candidate,
                });
            }
            Ok(Some(IceObserverEvent::Complete)) => {
                complete = true;
            }
            Ok(None) => {
                // チャネルが切断された
                return Err(crate::Error::new("ICE gathering channel closed"));
            }
            Err(_) => {
                // タイムアウト
                if !cached_candidates.is_empty() {
                    return Ok(append_ice_candidates_to_sdp(
                        &initial_sdp,
                        cached_candidates,
                    ));
                }
                return Err(crate::Error::new("ICE gathering timed out"));
            }
        }
    }

    if !candidates.is_empty() {
        *cached_candidates = candidates.clone();
    }

    Ok(append_ice_candidates_to_sdp(
        &initial_sdp,
        if candidates.is_empty() {
            cached_candidates
        } else {
            &candidates
        },
    ))
}

fn append_ice_candidates_to_sdp(sdp: &str, candidates: &[GatheredIceCandidate]) -> String {
    let mut sections: Vec<Vec<String>> = Vec::new();
    let mut current_section = Vec::new();

    for line in sdp.split("\r\n").filter(|line| !line.is_empty()) {
        if line.starts_with("m=") && !current_section.is_empty() {
            sections.push(current_section);
            current_section = Vec::new();
        }
        current_section.push(line.to_owned());
    }
    if !current_section.is_empty() {
        sections.push(current_section);
    }

    let mut output = Vec::new();
    for (index, section) in sections.into_iter().enumerate() {
        let is_media_section = section.first().is_some_and(|line| line.starts_with("m="));
        let sdp_mid = section
            .iter()
            .find_map(|line| line.strip_prefix("a=mid:"))
            .unwrap_or_default();

        for line in &section {
            output.push(line.clone());
        }

        if is_media_section {
            let section_candidates: Vec<&GatheredIceCandidate> = candidates
                .iter()
                .filter(|candidate| {
                    candidate.sdp_mid == sdp_mid || candidate.sdp_mline_index == index as i32 - 1
                })
                .collect();
            if !section_candidates.is_empty() {
                for candidate in section_candidates {
                    output.push(format!("a={}", candidate.candidate));
                }
                output.push("a=end-of-candidates".to_owned());
            }
        }
    }

    output.join("\r\n") + "\r\n"
}

// DataChannel メッセージ処理

/// DataChannel メッセージを処理する。true を返した場合はセッションを終了する。
async fn handle_dc_message(sess: &mut Session, data: &[u8]) -> bool {
    let Some(msg) = parse_signaling_message(data) else {
        tracing::warn!("Failed to parse signaling message");
        return false;
    };

    match msg.msg_type.as_str() {
        "answer" => handle_answer(sess, msg.sdp.as_deref()).await,
        "offer" => {
            // client からの offer は無視する
            tracing::info!("Ignoring offer from client");
            false
        }
        "disconnect" => {
            // client からの切断要求 (close は送信しない)
            tracing::info!("Received disconnect from client");
            true
        }
        _ => {
            send_close(
                sess,
                "unknown-type",
                &format!("Unknown type: {}", msg.msg_type),
            );
            true
        }
    }
}

/// answer を処理する。true を返した場合はセッションを終了する。
async fn handle_answer(sess: &mut Session, sdp: Option<&str>) -> bool {
    if !sess.in_flight_offer {
        send_close(sess, "unexpected", "Offer has not been sent");
        return true;
    }

    let Some(sdp) = sdp else {
        send_close(sess, "missing-sdp", "sdp field is required");
        return true;
    };

    if let Err(e) = super::sdp::set_remote_answer(&sess.pc, sdp) {
        if e.reason.contains("timed out") {
            send_close(sess, "timeout", &e.reason);
        } else {
            send_close(sess, "srd-error", &e.reason);
        }
        return true;
    }

    sess.in_flight_offer = false;
    if sess.pending_renegotiation {
        sess.pending_renegotiation = false;
        if let Err(e) = maybe_send_offer(sess).await {
            send_close(sess, "sdp-error", &e.reason);
            return true;
        }
    }
    false
}

fn send_close(sess: &Session, code: &str, reason: &str) {
    send_dc(sess, &make_close_json(code, reason));
}

fn send_dc(sess: &Session, msg: &str) {
    if let Some(dc) = &sess.signaling_dc {
        dc.send(msg.as_bytes(), false);
    }
}

/// obsws DataChannel で OBS WebSocket プロトコルメッセージを処理する
async fn handle_obsws_message(sess: &mut Session, data: &[u8]) -> bool {
    let text = match std::str::from_utf8(data) {
        Ok(text) => text,
        Err(_) => {
            tracing::warn!("Received non-UTF-8 obsws message on DataChannel");
            return false;
        }
    };
    tracing::debug!("Received obsws message: {text}");

    // OBS WS メッセージとして処理する
    let action = match sess.obsws_session.on_text_message(text).await {
        Ok(action) => action,
        Err(e) => {
            tracing::warn!("Invalid OBS WS message on DataChannel: {}", e.display());
            return false;
        }
    };

    // SessionAction を obsws DataChannel 送信に変換する
    apply_obsws_action_to_dc(sess, action)
}

/// OBS WS SessionAction を obsws DataChannel 経由で送信する
fn apply_obsws_action_to_dc(sess: &Session, action: SessionAction) -> bool {
    match action {
        SessionAction::SendText { text, .. } => {
            send_obsws_dc(sess, text.text());
            false
        }
        SessionAction::SendTexts { messages } => {
            for (text, _) in messages {
                send_obsws_dc(sess, text.text());
            }
            false
        }
        SessionAction::Close { reason, .. } => {
            tracing::warn!("OBS WS session close on DataChannel: {reason}");
            send_close(sess, "obsws-error", reason);
            true
        }
        SessionAction::Terminate => {
            tracing::warn!("OBS WS session terminate on DataChannel");
            send_close(sess, "obsws-terminated", "OBS WS session terminated");
            true
        }
    }
}

fn send_obsws_dc(sess: &Session, msg: &str) {
    if let Some(dc) = &sess.obsws_dc {
        dc.send(msg.as_bytes(), false);
    }
}

fn handle_track_message(sess: &mut Session, track_id: &crate::TrackId, message: crate::Message) {
    match message {
        crate::Message::Media(sample) => match sample {
            crate::MediaFrame::Video(frame) => {
                if frame.format != crate::video::VideoFormat::I420 {
                    tracing::info!(
                        "Unsupported video format for track {track_id}: {}",
                        frame.format
                    );
                    return;
                }

                if let Some(subscribed) = sess.subscribed_tracks.get_mut(track_id) {
                    let TrackState::Video(state) = &mut subscribed.state else {
                        return;
                    };
                    if let Err(e) = super::video::push_i420_frame(&mut state.source, &frame) {
                        tracing::warn!(
                            "Failed to send video frame for track {track_id}: {}",
                            e.display()
                        );
                    }
                }
            }
            crate::MediaFrame::Audio(frame) => {
                if let Some(subscribed) = sess.subscribed_tracks.get_mut(track_id)
                    && let TrackState::Audio(state) = &subscribed.state
                    && frame.format == crate::audio::AudioFormat::I16Be
                    && let Err(e) = state.audio_state.push_audio_frame(&frame)
                {
                    tracing::warn!(
                        "Failed to send audio frame for track {track_id}: {}",
                        e.display()
                    );
                }
            }
        },
        crate::Message::Eos => {
            tracing::info!("Track EOS received: {track_id}");
        }
        crate::Message::Syn(_) => {}
    }
}

fn create_video_track(
    sess: &mut Session,
    track_id: &crate::TrackId,
) -> crate::Result<(VideoTrackState, RtpSender)> {
    let source = AdaptedVideoTrackSource::new();
    let video_source = source.cast_to_video_track_source();
    let track = sess
        .factory
        .create_video_track(&video_source, track_id.get())
        .map_err(|e| crate::Error::new(format!("Failed to create video track: {e}")))?;

    let mut stream_ids = StringVector::new(0);
    let stream_id = CxxString::from_str(track_id.get());
    stream_ids.push(&stream_id);
    let sender = sess
        .pc
        .add_track(&track.cast_to_media_stream_track(), &stream_ids)
        .map_err(|e| crate::Error::new(format!("Failed to add track: {e}")))?;

    Ok((
        VideoTrackState {
            source,
            _track: track,
        },
        sender,
    ))
}

fn create_audio_track(
    sess: &mut Session,
    track_id: &crate::TrackId,
) -> crate::Result<(AudioTrackState, RtpSender)> {
    let source = sess
        .factory
        .create_audio_source()
        .map_err(|e| crate::Error::new(format!("Failed to create audio source: {e}")))?;
    let track = sess
        .factory
        .create_audio_track(&source, track_id.get())
        .map_err(|e| crate::Error::new(format!("Failed to create audio track: {e}")))?;

    let mut stream_ids = StringVector::new(0);
    let stream_id = CxxString::from_str(track_id.get());
    stream_ids.push(&stream_id);
    let sender = sess
        .pc
        .add_track(&track.cast_to_media_stream_track(), &stream_ids)
        .map_err(|e| crate::Error::new(format!("Failed to add audio track: {e}")))?;

    Ok((
        AudioTrackState {
            audio_state: sess.audio_state.clone(),
            _source: source,
            _track: track,
        },
        sender,
    ))
}

async fn maybe_send_offer(sess: &mut Session) -> crate::Result<()> {
    if sess.in_flight_offer {
        sess.pending_renegotiation = true;
        return Ok(());
    }
    let Some(dc) = &sess.signaling_dc else {
        sess.pending_renegotiation = true;
        return Ok(());
    };
    if dc.state() != DataChannelState::Open {
        sess.pending_renegotiation = true;
        return Ok(());
    }
    sess.pending_renegotiation = false;

    let offer_sdp = super::sdp::create_offer_sdp(&sess.pc)?;
    super::sdp::set_local_offer(&sess.pc, &offer_sdp)?;
    let offer_sdp =
        finalize_local_sdp(offer_sdp, &mut sess.ice_rx, &mut sess.ice_candidates).await?;

    send_dc(sess, &make_offer_json(&offer_sdp));
    sess.in_flight_offer = true;
    Ok(())
}

/// トラックを購読して WebRTC で配信する
fn subscribe_track(
    session: &mut Session,
    track_id: crate::TrackId,
    kind: TrackKind,
) -> crate::Result<bool> {
    if session.subscribed_tracks.contains_key(&track_id) {
        return Ok(false);
    }

    let (state, sender) = match kind {
        TrackKind::Video => {
            let (state, sender) = create_video_track(session, &track_id)?;
            (TrackState::Video(state), sender)
        }
        TrackKind::Audio => {
            let (state, sender) = create_audio_track(session, &track_id)?;
            (TrackState::Audio(state), sender)
        }
    };

    let mut rx = session.processor_handle.subscribe_track(track_id.clone());
    let event_tx = session.event_tx.clone();
    let track_id_for_task = track_id.clone();
    let task = tokio::spawn(async move {
        loop {
            let message = rx.recv().await;
            if event_tx
                .send(PcEvent::TrackMessage {
                    track_id: track_id_for_task.clone(),
                    message,
                })
                .is_err()
            {
                break;
            }
        }
    });

    session.subscribed_tracks.insert(
        track_id,
        SubscribedTrack {
            state,
            sender,
            abort_handle: task.abort_handle(),
        },
    );

    Ok(true)
}

/// bootstrap snapshot の入力に対してトラックを購読する
fn subscribe_bootstrap_input(
    session: &mut Session,
    snapshot: &crate::obsws::coordinator::BootstrapInputSnapshot,
) {
    if let Some(video_track_id) = &snapshot.video_track_id
        && let Err(e) = subscribe_track(session, video_track_id.clone(), TrackKind::Video)
    {
        tracing::warn!(
            "failed to subscribe bootstrap video track for {}: {}",
            snapshot.input_uuid,
            e.display()
        );
    }
    if let Some(audio_track_id) = &snapshot.audio_track_id
        && let Err(e) = subscribe_track(session, audio_track_id.clone(), TrackKind::Audio)
    {
        tracing::warn!(
            "failed to subscribe bootstrap audio track for {}: {}",
            snapshot.input_uuid,
            e.display()
        );
    }
    session
        .bootstrap_tracks
        .insert(snapshot.input_uuid.clone(), snapshot.clone());
}

/// bootstrap 入力作成時のハンドラ
async fn handle_bootstrap_input_created(
    sess: &mut Session,
    snapshot: crate::obsws::coordinator::BootstrapInputSnapshot,
) {
    subscribe_bootstrap_input(sess, &snapshot);

    // メタデータを signaling DataChannel で送信する
    let msg = build_bootstrap_track_added_json(&snapshot);
    send_dc(sess, &msg);

    if let Err(e) = maybe_send_offer(sess).await {
        tracing::warn!(
            "failed to send renegotiation offer after input created: {}",
            e.display()
        );
    }
}

/// bootstrap 入力削除時のハンドラ
async fn handle_bootstrap_input_removed(sess: &mut Session, input_uuid: &str) {
    if let Some(entry) = sess.bootstrap_tracks.remove(input_uuid) {
        if let Some(video_track_id) = &entry.video_track_id {
            unsubscribe_track(sess, video_track_id);
        }
        if let Some(audio_track_id) = &entry.audio_track_id {
            unsubscribe_track(sess, audio_track_id);
        }
    }

    // メタデータを signaling DataChannel で送信する
    let msg = build_bootstrap_track_removed_json(input_uuid);
    send_dc(sess, &msg);

    if let Err(e) = maybe_send_offer(sess).await {
        tracing::warn!(
            "failed to send renegotiation offer after input removed: {}",
            e.display()
        );
    }
}

/// bootstrap-tracks snapshot メッセージを構築する
fn build_bootstrap_tracks_json(
    snapshots: &[crate::obsws::coordinator::BootstrapInputSnapshot],
) -> String {
    nojson::object(|f| {
        f.member("type", "bootstrap-tracks")?;
        f.member(
            "tracks",
            nojson::array(|a| {
                for s in snapshots {
                    a.element(nojson::object(|f| {
                        f.member("inputUuid", s.input_uuid.as_str())?;
                        f.member("inputName", s.input_name.as_str())?;
                        f.member("inputKind", s.input_kind.as_str())?;
                        if let Some(id) = &s.video_track_id {
                            f.member("videoTrackId", id.get())?;
                        }
                        if let Some(id) = &s.audio_track_id {
                            f.member("audioTrackId", id.get())?;
                        }
                        Ok(())
                    }))?;
                }
                Ok(())
            }),
        )
    })
    .to_string()
}

/// bootstrap-track-added メッセージを構築する
fn build_bootstrap_track_added_json(
    snapshot: &crate::obsws::coordinator::BootstrapInputSnapshot,
) -> String {
    nojson::object(|f| {
        f.member("type", "bootstrap-track-added")?;
        f.member("inputUuid", snapshot.input_uuid.as_str())?;
        f.member("inputName", snapshot.input_name.as_str())?;
        f.member("inputKind", snapshot.input_kind.as_str())?;
        if let Some(id) = &snapshot.video_track_id {
            f.member("videoTrackId", id.get())?;
        }
        if let Some(id) = &snapshot.audio_track_id {
            f.member("audioTrackId", id.get())?;
        }
        Ok(())
    })
    .to_string()
}

/// bootstrap-track-removed メッセージを構築する
fn build_bootstrap_track_removed_json(input_uuid: &str) -> String {
    nojson::object(|f| {
        f.member("type", "bootstrap-track-removed")?;
        f.member("inputUuid", input_uuid)
    })
    .to_string()
}

/// トラックの購読を解除する
fn unsubscribe_track(session: &mut Session, track_id: &crate::TrackId) {
    if let Some(mut subscribed) = session.subscribed_tracks.remove(track_id) {
        subscribed.abort_handle.abort();
        if !subscribed.sender.set_track(None) {
            tracing::warn!("set_track(None) failed for track {track_id}");
        }
    }
}

enum TrackKind {
    Video,
    Audio,
}
