use std::sync::Arc;

use shiguredo_webrtc::{
    AdaptedVideoTrackSource, CxxString, DataChannel, DataChannelInit, DataChannelObserver,
    DataChannelObserverHandler, PeerConnection, PeerConnectionDependencies, PeerConnectionFactory,
    PeerConnectionObserver, PeerConnectionObserverHandler, PeerConnectionRtcConfiguration,
    PeerConnectionState, StringVector,
};
use tokio::sync::mpsc;

enum PcEvent {
    ConnectionChange(PeerConnectionState),
    DataChannel(DataChannel),
    DcMessage {
        data: Vec<u8>,
    },
    RpcMessage {
        data: Vec<u8>,
        is_binary: bool,
    },
    TrackMessage {
        track_id: crate::TrackId,
        message: crate::Message,
    },
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
}

impl PeerConnectionObserverHandler for P2pPcObserverHandler {
    fn on_connection_change(&mut self, state: PeerConnectionState) {
        let _ = self.event_tx.send(PcEvent::ConnectionChange(state));
    }

    fn on_data_channel(&mut self, dc: DataChannel) {
        let _ = self.event_tx.send(PcEvent::DataChannel(dc));
    }
}

struct DcMessageHandler {
    event_tx: mpsc::UnboundedSender<PcEvent>,
}

impl DataChannelObserverHandler for DcMessageHandler {
    fn on_message(&mut self, data: &[u8], _is_binary: bool) {
        let _ = self.event_tx.send(PcEvent::DcMessage {
            data: data.to_vec(),
        });
    }
}

struct RpcMessageHandler {
    event_tx: mpsc::UnboundedSender<PcEvent>,
}

impl DataChannelObserverHandler for RpcMessageHandler {
    fn on_message(&mut self, data: &[u8], is_binary: bool) {
        let _ = self.event_tx.send(PcEvent::RpcMessage {
            data: data.to_vec(),
            is_binary,
        });
    }
}

struct Session {
    handle: crate::MediaPipelineHandle,
    processor_handle: crate::ProcessorHandle,
    factory: Arc<PeerConnectionFactory>,
    pc: PeerConnection,
    _pc_observer: shiguredo_webrtc::PeerConnectionObserver,
    signaling_dc: Option<DataChannel>,
    _rpc_dc: Option<DataChannel>,
    _dc_observer: Option<shiguredo_webrtc::DataChannelObserver>,
    _rpc_dc_observer: Option<shiguredo_webrtc::DataChannelObserver>,
    in_flight_offer: bool,
    pending_renegotiation: bool,
    subscribed_tracks: std::collections::HashMap<crate::TrackId, SubscribedTrack>,
    event_tx: mpsc::UnboundedSender<PcEvent>,
}

struct SubscribedTrack {
    state: TrackState,
}

enum TrackState {
    Video(VideoTrackState),
    AudioUnsupported,
}

struct VideoTrackState {
    source: AdaptedVideoTrackSource,
    _track: shiguredo_webrtc::VideoTrack,
}

pub enum BootstrapError {
    SessionAlreadyExists,
    Internal(crate::Error),
}

pub struct WebRtcP2pSessionManager {
    factory_bundle: Arc<crate::webrtc_factory::WebRtcFactoryBundle>,
    pipeline_handle: crate::MediaPipelineHandle,
    session: Arc<tokio::sync::Mutex<Option<Session>>>,
    event_tx: mpsc::UnboundedSender<PcEvent>,
}

impl WebRtcP2pSessionManager {
    pub fn new(handle: crate::MediaPipelineHandle) -> crate::Result<Self> {
        #[allow(clippy::arc_with_non_send_sync)]
        let factory_bundle = Arc::new(crate::webrtc_factory::WebRtcFactoryBundle::new()?);
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
                                }));
                            dc.register_observer(&dc_observer);
                            sess.signaling_dc = Some(dc);
                            sess._dc_observer = Some(dc_observer);
                        }
                    }
                    PcEvent::DcMessage { data } => {
                        if handle_dc_message(sess, &data) {
                            tracing::info!("Session closed");
                            *guard = None;
                        }
                    }
                    PcEvent::RpcMessage { data, is_binary } => {
                        handle_rpc_message(sess, &data, is_binary).await;
                    }
                    PcEvent::TrackMessage { track_id, message } => {
                        handle_track_message(sess, &track_id, message);
                    }
                }
            }
        });

        Ok(Self {
            factory_bundle,
            pipeline_handle: handle,
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

        let mut guard = self.session.lock().await;
        if guard.is_some() {
            drop(processor_handle);
            return Err(BootstrapError::SessionAlreadyExists);
        }

        match bootstrap_internal(
            self.factory_bundle.factory(),
            offer_sdp,
            self.event_tx.clone(),
            self.pipeline_handle.clone(),
            processor_handle,
        ) {
            Ok((answer_sdp, sess)) => {
                *guard = Some(sess);
                Ok(answer_sdp)
            }
            Err(e) => Err(BootstrapError::Internal(e)),
        }
    }
}

fn bootstrap_internal(
    factory: Arc<PeerConnectionFactory>,
    offer_sdp: &str,
    event_tx: mpsc::UnboundedSender<PcEvent>,
    handle: crate::MediaPipelineHandle,
    processor_handle: crate::ProcessorHandle,
) -> crate::Result<(String, Session)> {
    // PeerConnectionObserver の作成
    let pc_observer = PeerConnectionObserver::new_with_handler(Box::new(P2pPcObserverHandler {
        event_tx: event_tx.clone(),
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

    let mut rpc_dc_init = DataChannelInit::new();
    rpc_dc_init.set_ordered(true);
    rpc_dc_init.set_protocol("rpc");
    let mut rpc_dc = pc
        .create_data_channel("rpc", &mut rpc_dc_init)
        .map_err(|e| crate::Error::new(format!("Failed to create rpc DataChannel: {e}")))?;

    // DataChannel に observer を設定
    let dc_observer = DataChannelObserver::new_with_handler(Box::new(DcMessageHandler {
        event_tx: event_tx.clone(),
    }));
    signaling_dc.register_observer(&dc_observer);

    // rpc 用 DataChannel に observer を設定
    let rpc_observer = DataChannelObserver::new_with_handler(Box::new(RpcMessageHandler {
        event_tx: event_tx.clone(),
    }));
    rpc_dc.register_observer(&rpc_observer);

    crate::webrtc_sdp::set_remote_offer(&pc, offer_sdp)?;
    let answer_sdp = crate::webrtc_sdp::create_answer_sdp(&pc)?;
    crate::webrtc_sdp::set_local_answer(&pc, &answer_sdp)?;

    let sess = Session {
        handle,
        processor_handle,
        factory,
        pc,
        _pc_observer: pc_observer,
        signaling_dc: Some(signaling_dc),
        _rpc_dc: Some(rpc_dc),
        _dc_observer: Some(dc_observer),
        _rpc_dc_observer: Some(rpc_observer),
        in_flight_offer: false,
        pending_renegotiation: false,
        subscribed_tracks: std::collections::HashMap::new(),
        event_tx,
    };

    Ok((answer_sdp, sess))
}

// DataChannel メッセージ処理

/// DataChannel メッセージを処理する。true を返した場合はセッションを終了する。
fn handle_dc_message(sess: &mut Session, data: &[u8]) -> bool {
    let Some(msg) = parse_signaling_message(data) else {
        tracing::warn!("Failed to parse signaling message");
        return false;
    };

    match msg.msg_type.as_str() {
        "answer" => handle_answer(sess, msg.sdp.as_deref()),
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
fn handle_answer(sess: &mut Session, sdp: Option<&str>) -> bool {
    if !sess.in_flight_offer {
        send_close(sess, "unexpected", "Offer has not been sent");
        return true;
    }

    let Some(sdp) = sdp else {
        send_close(sess, "missing-sdp", "sdp field is required");
        return true;
    };

    if let Err(e) = crate::webrtc_sdp::set_remote_answer(&sess.pc, sdp) {
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
        if let Err(e) = maybe_send_offer(sess) {
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

async fn handle_rpc_message(sess: &mut Session, data: &[u8], is_binary: bool) {
    if let Ok(text) = std::str::from_utf8(data) {
        tracing::debug!("Received rpc message: {text}");
    } else {
        tracing::debug!("Received rpc message: {data:?}");
    }

    let request_json = match crate::jsonrpc::parse_request_bytes(data) {
        Ok(json) => json,
        Err(response) => {
            send_rpc_response(sess, response.to_string().as_bytes(), is_binary);
            return;
        }
    };
    let request = request_json.value();
    let method = match crate::jsonrpc::get_method(request) {
        Ok(method) => method,
        Err(response) => {
            send_rpc_response(sess, response.to_string().as_bytes(), is_binary);
            return;
        }
    };
    let request_id = request.to_member("id").ok().and_then(|v| v.optional());

    if method != "subscribe" {
        if let Some(response) = sess.handle.rpc(data).await {
            send_rpc_response(sess, response.to_string().as_bytes(), is_binary);
        }
        return;
    }

    match (request_id, handle_subscribe_rpc(sess, request).await) {
        (Some(id), Ok(result)) => {
            let response = crate::jsonrpc::ok_response(
                id,
                nojson::json(|f| write!(f.inner_mut(), "{result}")),
            );
            send_rpc_response(sess, response.to_string().as_bytes(), is_binary);
        }
        (Some(id), Err(e)) => {
            let response =
                crate::jsonrpc::error_response(id, crate::jsonrpc::INTERNAL_ERROR, e.reason);
            send_rpc_response(sess, response.to_string().as_bytes(), is_binary);
        }
        (None, Ok(_)) => {}
        (None, Err(e)) => {
            tracing::warn!(
                "rpc notification failed: method=subscribe, code={}, message={}",
                crate::jsonrpc::INTERNAL_ERROR,
                e.reason
            );
        }
    }
}

fn send_rpc_response(sess: &Session, response_bytes: &[u8], is_binary: bool) {
    if let Some(dc) = &sess._rpc_dc {
        dc.send(response_bytes, is_binary);
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
                    if let Err(e) = crate::webrtc_video::push_i420_frame(&mut state.source, &frame)
                    {
                        tracing::warn!(
                            "Failed to send video frame for track {track_id}: {}",
                            e.display()
                        );
                    }
                }
            }
            crate::MediaFrame::Audio(_) => {
                if let Some(subscribed) = sess.subscribed_tracks.get_mut(track_id)
                    && !matches!(subscribed.state, TrackState::AudioUnsupported)
                {
                    tracing::info!("Audio track is not supported yet: {track_id}");
                    subscribed.state = TrackState::AudioUnsupported;
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
) -> crate::Result<VideoTrackState> {
    let source = AdaptedVideoTrackSource::new();
    let video_source = source.cast_to_video_track_source();
    let track = sess
        .factory
        .create_video_track(&video_source, track_id.get())
        .map_err(|e| crate::Error::new(format!("Failed to create video track: {e}")))?;

    let mut stream_ids = StringVector::new(0);
    let stream_id = CxxString::from_str(track_id.get());
    stream_ids.push(&stream_id);
    let _sender = sess
        .pc
        .add_track(&track.cast_to_media_stream_track(), &stream_ids)
        .map_err(|e| crate::Error::new(format!("Failed to add track: {e}")))?;

    Ok(VideoTrackState {
        source,
        _track: track,
    })
}

fn maybe_send_offer(sess: &mut Session) -> crate::Result<()> {
    if sess.in_flight_offer {
        sess.pending_renegotiation = true;
        return Ok(());
    }
    sess.pending_renegotiation = false;

    let offer_sdp = crate::webrtc_sdp::create_offer_sdp(&sess.pc)?;
    crate::webrtc_sdp::set_local_offer(&sess.pc, &offer_sdp)?;

    send_dc(sess, &make_offer_json(&offer_sdp));
    sess.in_flight_offer = true;
    Ok(())
}

async fn handle_subscribe_rpc(
    session: &mut Session,
    req: nojson::RawJsonValue<'_, '_>,
) -> crate::Result<String> {
    enum SubscribeKind {
        Audio,
        Video,
    }

    struct SubscribeItem {
        track_id: crate::TrackId,
        kind: SubscribeKind,
    }

    let params_value = req.to_member("params")?.required()?;
    let mut items: Vec<SubscribeItem> = params_value
        .to_array()?
        .map(|value| {
            let track_id: crate::TrackId = value.to_member("trackId")?.required()?.try_into()?;
            let kind = match value.to_member("kind")?.required()?.as_string_str()? {
                "audio" => SubscribeKind::Audio,
                "video" => SubscribeKind::Video,
                _ => {
                    return Err(value
                        .to_member("kind")?
                        .required()?
                        .invalid("kind must be \"audio\" or \"video\""));
                }
            };
            Ok(SubscribeItem { track_id, kind })
        })
        .collect::<Result<_, nojson::JsonParseError>>()?;

    items.sort_by(|a, b| a.track_id.cmp(&b.track_id));
    items.dedup_by(|a, b| a.track_id == b.track_id);

    let mut needs_offer = false;
    for item in items {
        if session.subscribed_tracks.contains_key(&item.track_id) {
            continue;
        }
        let state = match item.kind {
            SubscribeKind::Video => {
                let state = create_video_track(session, &item.track_id)?;
                needs_offer = true;
                TrackState::Video(state)
            }
            SubscribeKind::Audio => {
                tracing::info!("Audio track is not supported yet: {}", item.track_id);
                TrackState::AudioUnsupported
            }
        };
        session
            .subscribed_tracks
            .insert(item.track_id.clone(), SubscribedTrack { state });

        let mut rx = session
            .processor_handle
            .subscribe_track(item.track_id.clone());
        let event_tx = session.event_tx.clone();
        let track_id_for_task = item.track_id;
        let _task = tokio::spawn(async move {
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
    }

    if needs_offer {
        maybe_send_offer(session)?;
    }

    Ok(nojson::Json("ok").to_string())
}
