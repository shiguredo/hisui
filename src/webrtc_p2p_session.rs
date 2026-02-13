use std::sync::Arc;
use std::time::Duration;

use nojson::RawJson;
use shiguredo_webrtc::{
    AudioDecoderFactory, AudioDeviceModule, AudioDeviceModuleAudioLayer, AudioEncoderFactory,
    AudioProcessingBuilder, CreateSessionDescriptionObserver, DataChannel, DataChannelInit,
    DataChannelObserverBuilder, Environment, PeerConnection, PeerConnectionDependencies,
    PeerConnectionFactory, PeerConnectionFactoryDependencies, PeerConnectionObserverBuilder,
    PeerConnectionOfferAnswerOptions, PeerConnectionRtcConfiguration, PeerConnectionState,
    RtcEventLogFactory, SdpType, SessionDescription, SetLocalDescriptionObserver,
    SetRemoteDescriptionObserver, Thread,
};
use tokio::sync::mpsc;

use crate::json::JsonObject;
use shiguredo_webrtc::{AdaptedVideoTrackSource, CxxString, StringVector};

// -------------------------
// FactoryHolder
// -------------------------

struct FactoryHolder {
    factory: Arc<PeerConnectionFactory>,
    _network: Thread,
    _worker: Thread,
    _signaling: Thread,
}

impl FactoryHolder {
    fn new() -> Option<Self> {
        let env = Environment::new();
        let mut network = Thread::new_with_socket_server();
        let mut worker = Thread::new();
        let mut signaling = Thread::new();
        network.start();
        worker.start();
        signaling.start();

        let mut deps = PeerConnectionFactoryDependencies::new();
        deps.set_network_thread(&network);
        deps.set_worker_thread(&worker);
        deps.set_signaling_thread(&signaling);
        let event_log = RtcEventLogFactory::new();
        deps.set_event_log_factory(event_log);
        let adm = AudioDeviceModule::new(&env, AudioDeviceModuleAudioLayer::Dummy).ok()?;
        deps.set_audio_device_module(&adm);
        let audio_enc = AudioEncoderFactory::builtin();
        let audio_dec = AudioDecoderFactory::builtin();
        deps.set_audio_encoder_factory(&audio_enc);
        deps.set_audio_decoder_factory(&audio_dec);
        let video_enc = shiguredo_webrtc::VideoEncoderFactory::builtin();
        let video_dec = shiguredo_webrtc::VideoDecoderFactory::builtin();
        deps.set_video_encoder_factory(video_enc);
        deps.set_video_decoder_factory(video_dec);
        let apb = AudioProcessingBuilder::new_builtin();
        deps.set_audio_processing_builder(apb);
        deps.enable_media();

        let factory = PeerConnectionFactory::create_modular(&mut deps).ok()?;
        Some(Self {
            factory: Arc::new(factory),
            _network: network,
            _worker: worker,
            _signaling: signaling,
        })
    }

    fn factory(&self) -> Arc<PeerConnectionFactory> {
        self.factory.clone()
    }
}

// -------------------------
// コールバックイベント
// -------------------------

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

// -------------------------
// signaling JSON
// -------------------------

struct SignalingMessage {
    msg_type: String,
    sdp: Option<String>,
}

fn parse_signaling_message(data: &[u8]) -> Option<SignalingMessage> {
    let text = std::str::from_utf8(data).ok()?;
    let json = RawJson::parse(text).ok()?;
    let v = json.value();
    let msg_type: String = v.to_member("type").ok()?.required().ok()?.try_into().ok()?;
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

// -------------------------
// Session
// -------------------------

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

// -------------------------
// 公開 API
// -------------------------

pub enum BootstrapError {
    SessionAlreadyExists,
    Internal(crate::Error),
}

pub struct WebRtcP2pSessionManager {
    factory_holder: Arc<FactoryHolder>,
    pipeline_handle: crate::MediaPipelineHandle,
    session: Arc<tokio::sync::Mutex<Option<Session>>>,
    event_tx: mpsc::UnboundedSender<PcEvent>,
}

impl WebRtcP2pSessionManager {
    pub fn new(handle: crate::MediaPipelineHandle) -> crate::Result<Self> {
        #[allow(clippy::arc_with_non_send_sync)]
        let factory_holder = Arc::new(
            FactoryHolder::new()
                .ok_or_else(|| crate::Error::new("Failed to create FactoryHolder"))?,
        );
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
                    PcEvent::DataChannel(dc) => {
                        let label = dc.label().unwrap_or_default();
                        tracing::info!("DataChannel received: label={label}");
                        if label == "signaling" {
                            let tx = sess.event_tx.clone();
                            let dc_observer = DataChannelObserverBuilder::new()
                                .on_message(move |data, _is_binary| {
                                    let _ = tx.send(PcEvent::DcMessage {
                                        data: data.to_vec(),
                                    });
                                })
                                .build();
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
            factory_holder,
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
            .register_processor(crate::ProcessorId::new("webrtc_p2p_session"))
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
            self.factory_holder.factory(),
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
    let event_tx_conn = event_tx.clone();
    let event_tx_dc = event_tx.clone();
    let pc_observer = PeerConnectionObserverBuilder::new()
        .on_connection_change(move |state| {
            let _ = event_tx_conn.send(PcEvent::ConnectionChange(state));
        })
        .on_data_channel(move |dc| {
            let _ = event_tx_dc.send(PcEvent::DataChannel(dc));
        })
        .build();

    let mut deps = PeerConnectionDependencies::new(&pc_observer);
    let mut config = PeerConnectionRtcConfiguration::new();

    let pc = PeerConnection::create(factory.as_ref(), &mut config, &mut deps)
        .map_err(|e| crate::Error::new(format!("Failed to create PeerConnection: {e}")))?;

    // server 側から DataChannel を作成
    let mut dc_init = DataChannelInit::new();
    dc_init.set_ordered(true);
    dc_init.set_protocol("signaling");
    let signaling_dc = pc
        .create_data_channel("signaling", &mut dc_init)
        .map_err(|e| crate::Error::new(format!("Failed to create signaling DataChannel: {e}")))?;

    let mut rpc_dc_init = DataChannelInit::new();
    rpc_dc_init.set_ordered(true);
    rpc_dc_init.set_protocol("rpc");
    let rpc_dc = pc
        .create_data_channel("rpc", &mut rpc_dc_init)
        .map_err(|e| crate::Error::new(format!("Failed to create rpc DataChannel: {e}")))?;

    // DataChannel に observer を設定
    let dc_event_tx = event_tx.clone();
    let dc_observer = DataChannelObserverBuilder::new()
        .on_message(move |data, _is_binary| {
            let _ = dc_event_tx.send(PcEvent::DcMessage {
                data: data.to_vec(),
            });
        })
        .build();
    signaling_dc.register_observer(&dc_observer);

    // rpc 用 DataChannel に observer を設定
    let rpc_event_tx = event_tx.clone();
    let rpc_observer = DataChannelObserverBuilder::new()
        .on_message(move |data, is_binary| {
            let _ = rpc_event_tx.send(PcEvent::RpcMessage {
                data: data.to_vec(),
                is_binary,
            });
        })
        .build();
    rpc_dc.register_observer(&rpc_observer);

    // リモート SDP (offer) を設定
    let offer = SessionDescription::new(SdpType::Offer, offer_sdp)
        .map_err(|e| crate::Error::new(format!("Failed to parse offer SDP: {e}")))?;
    let (srd_tx, srd_rx) = std::sync::mpsc::channel::<Option<String>>();
    let srd_obs = SetRemoteDescriptionObserver::new(move |err| {
        let msg = if err.ok() {
            None
        } else {
            Some(err.message().unwrap_or_else(|_| "unknown".to_string()))
        };
        let _ = srd_tx.send(msg);
    });
    pc.set_remote_description(offer, &srd_obs);
    let srd_result = srd_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| crate::Error::new("set_remote_description timeout"))?;
    if let Some(err) = srd_result {
        return Err(crate::Error::new(format!(
            "set_remote_description error: {err}"
        )));
    }

    // Answer を作成
    let mut opts = PeerConnectionOfferAnswerOptions::new();
    let (answer_tx, answer_rx) = std::sync::mpsc::channel::<crate::Result<String>>();
    let answer_tx_ok = answer_tx.clone();
    let mut answer_obs = CreateSessionDescriptionObserver::new(
        move |desc| {
            let sdp = desc
                .to_string()
                .map_err(|e| crate::Error::new(format!("Failed to convert answer to string: {e}")));
            let _ = answer_tx_ok.send(sdp);
        },
        move |err| {
            let msg = err.message().unwrap_or_else(|_| "unknown".to_string());
            let _ = answer_tx.send(Err(crate::Error::new(msg)));
        },
    );
    pc.create_answer(&mut answer_obs, &mut opts);
    let answer_sdp = answer_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| crate::Error::new("create_answer timeout"))?
        .map_err(|e| crate::Error::new(format!("create_answer error: {e}")))?;

    // ローカル SDP (answer) を設定
    let answer = SessionDescription::new(SdpType::Answer, &answer_sdp)
        .map_err(|e| crate::Error::new(format!("Failed to parse answer SDP: {e}")))?;
    let (sld_tx, sld_rx) = std::sync::mpsc::channel::<Option<String>>();
    let sld_obs = SetLocalDescriptionObserver::new(move |err| {
        let msg = if err.ok() {
            None
        } else {
            Some(err.message().unwrap_or_else(|_| "unknown".to_string()))
        };
        let _ = sld_tx.send(msg);
    });
    pc.set_local_description(answer, &sld_obs);
    let sld_result = sld_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| crate::Error::new("set_local_description timeout"))?;
    if let Some(err) = sld_result {
        return Err(crate::Error::new(format!(
            "set_local_description error: {err}"
        )));
    }

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

// -------------------------
// DataChannel メッセージ処理
// -------------------------

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

    let answer = match SessionDescription::new(SdpType::Answer, sdp) {
        Ok(d) => d,
        Err(e) => {
            send_close(sess, "sdp-error", &format!("{e}"));
            return true;
        }
    };

    let (srd_tx, srd_rx) = std::sync::mpsc::channel::<Option<String>>();
    let srd_obs = SetRemoteDescriptionObserver::new(move |err| {
        let msg = if err.ok() {
            None
        } else {
            Some(err.message().unwrap_or_else(|_| "unknown".to_string()))
        };
        let _ = srd_tx.send(msg);
    });
    sess.pc.set_remote_description(answer, &srd_obs);
    match srd_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(None) => {
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
        Ok(Some(e)) => {
            send_close(sess, "srd-error", &e);
            true
        }
        Err(_) => {
            send_close(sess, "timeout", "set_remote_description timeout");
            true
        }
    }
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
    let method = request
        .to_member("method")
        .expect("bug")
        .required()
        .expect("bug")
        .as_string_str()
        .expect("bug");
    let request_id = request.to_member("id").ok().and_then(|v| v.get());

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
            return;
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
            crate::MediaSample::Video(frame) => {
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
                    if let Err(e) = push_i420_frame(&mut state.source, &frame) {
                        tracing::warn!("Failed to send video frame for track {track_id}: {e}");
                    }
                }
            }
            crate::MediaSample::Audio(_) => {
                if let Some(subscribed) = sess.subscribed_tracks.get_mut(track_id) {
                    if !matches!(subscribed.state, TrackState::AudioUnsupported) {
                        tracing::info!("Audio track is not supported yet: {track_id}");
                        subscribed.state = TrackState::AudioUnsupported;
                    }
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

    let mut opts = PeerConnectionOfferAnswerOptions::new();
    let (offer_tx, offer_rx) = std::sync::mpsc::channel::<crate::Result<String>>();
    let offer_tx_ok = offer_tx.clone();
    let mut offer_obs = CreateSessionDescriptionObserver::new(
        move |desc| {
            let sdp = desc
                .to_string()
                .map_err(|e| crate::Error::new(format!("Failed to convert offer to string: {e}")));
            let _ = offer_tx_ok.send(sdp);
        },
        move |err| {
            let msg = err.message().unwrap_or_else(|_| "unknown".to_string());
            let _ = offer_tx.send(Err(crate::Error::new(msg)));
        },
    );
    sess.pc.create_offer(&mut offer_obs, &mut opts);
    let offer_sdp = offer_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| crate::Error::new("create_offer timeout"))?
        .map_err(|e| crate::Error::new(format!("create_offer error: {e}")))?;

    let offer = SessionDescription::new(SdpType::Offer, &offer_sdp)
        .map_err(|e| crate::Error::new(format!("Failed to parse offer SDP: {e}")))?;
    let (sld_tx, sld_rx) = std::sync::mpsc::channel::<Option<String>>();
    let sld_obs = SetLocalDescriptionObserver::new(move |err| {
        let msg = if err.ok() {
            None
        } else {
            Some(err.message().unwrap_or_else(|_| "unknown".to_string()))
        };
        let _ = sld_tx.send(msg);
    });
    sess.pc.set_local_description(offer, &sld_obs);
    let sld_result = sld_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| crate::Error::new("set_local_description timeout"))?;
    if let Some(err) = sld_result {
        return Err(crate::Error::new(format!(
            "set_local_description error: {err}"
        )));
    }

    send_dc(sess, &make_offer_json(&offer_sdp));
    sess.in_flight_offer = true;
    Ok(())
}

fn push_i420_frame(
    source: &mut AdaptedVideoTrackSource,
    frame: &crate::VideoFrame,
) -> crate::Result<()> {
    let width: usize = frame.width;
    let height: usize = frame.height;
    if width == 0 || height == 0 {
        return Err(crate::Error::new("invalid frame size"));
    }

    let uv_width = (width + 1) / 2;
    let uv_height = (height + 1) / 2;
    let y_size = width * height;
    let uv_size = uv_width * uv_height;
    if frame.data.len() < y_size + uv_size * 2 {
        return Err(crate::Error::new("insufficient I420 data"));
    }

    let (y_plane, rest) = frame.data.split_at(y_size);
    let (u_plane, v_plane) = rest.split_at(uv_size);

    let buffer = shiguredo_webrtc::I420Buffer::new(width as i32, height as i32);

    // I420Buffer は内部的に可変領域を持つが、API は読み取り専用参照を返すため、
    // ここでは安全性を確認した上で raw ポインタに書き込む。
    unsafe {
        copy_plane(
            buffer.y_data().as_ptr() as *mut u8,
            buffer.stride_y() as usize,
            y_plane,
            width,
            height,
        );
        copy_plane(
            buffer.u_data().as_ptr() as *mut u8,
            buffer.stride_u() as usize,
            u_plane,
            uv_width,
            uv_height,
        );
        copy_plane(
            buffer.v_data().as_ptr() as *mut u8,
            buffer.stride_v() as usize,
            v_plane,
            uv_width,
            uv_height,
        );
    }

    let timestamp_us = frame.timestamp.as_micros() as i64;
    let webrtc_frame = shiguredo_webrtc::VideoFrame::from_i420(&buffer, timestamp_us);
    source.on_frame(&webrtc_frame);
    Ok(())
}

unsafe fn copy_plane(dst: *mut u8, dst_stride: usize, src: &[u8], width: usize, height: usize) {
    for row in 0..height {
        let src_offset = row * width;
        let dst_offset = row * dst_stride;
        unsafe {
            let src_ptr = src.as_ptr().add(src_offset);
            let dst_ptr = dst.add(dst_offset);
            std::ptr::copy_nonoverlapping(src_ptr, dst_ptr, width);
        }
    }
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

    let obj = JsonObject::new(req)?;
    let params_value = obj.get_required_with("params", |v| Ok(v))?;
    let mut items: Vec<SubscribeItem> = params_value
        .to_array()?
        .map(|value| {
            let item = JsonObject::new(value)?;
            let track_id = item.get_required("trackId")?;
            let kind = item.get_required_with("kind", |v| {
                let kind = v.as_string_str()?;
                match kind {
                    "audio" => Ok(SubscribeKind::Audio),
                    "video" => Ok(SubscribeKind::Video),
                    _ => Err(v.invalid("kind must be \"audio\" or \"video\"")),
                }
            })?;
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
