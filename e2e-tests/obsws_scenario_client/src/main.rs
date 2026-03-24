use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use shiguredo_http11::{Request, ResponseDecoder};
use shiguredo_webrtc::{
    AudioDecoderFactory, AudioDeviceModule, AudioDeviceModuleAudioLayer, AudioEncoderFactory,
    AudioProcessingBuilder, CreateSessionDescriptionObserver,
    CreateSessionDescriptionObserverHandler, DataChannel, DataChannelInit, DataChannelObserver,
    DataChannelObserverHandler, Environment, IceGatheringState, PeerConnection,
    PeerConnectionDependencies, PeerConnectionFactory, PeerConnectionFactoryDependencies,
    PeerConnectionObserver, PeerConnectionObserverHandler, PeerConnectionOfferAnswerOptions,
    PeerConnectionRtcConfiguration, PeerConnectionState, RtcError, RtcEventLogFactory,
    RtpTransceiver, SdpType, SessionDescription, SetLocalDescriptionObserver,
    SetLocalDescriptionObserverHandler, SetRemoteDescriptionObserver,
    SetRemoteDescriptionObserverHandler, Thread, VideoDecoderFactory, VideoEncoderFactory,
    VideoSink, VideoSinkHandler, VideoSinkWants,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

const SDP_TIMEOUT: Duration = Duration::from_secs(5);

// --- イベント ---

enum ClientEvent {
    ConnectionChange(PeerConnectionState),
    Track(RtpTransceiver),
    DataChannel(DataChannel),
    SignalingMessage { data: Vec<u8> },
}

enum IceObserverEvent {
    Candidate {
        sdp_mid: String,
        sdp_mline_index: i32,
        candidate: String,
    },
    Complete,
}

// --- Observer ハンドラ ---

struct ClientPcObserver {
    event_tx: mpsc::UnboundedSender<ClientEvent>,
    ice_tx: std::sync::mpsc::Sender<IceObserverEvent>,
}

impl PeerConnectionObserverHandler for ClientPcObserver {
    fn on_connection_change(&mut self, state: PeerConnectionState) {
        let _ = self.event_tx.send(ClientEvent::ConnectionChange(state));
    }

    fn on_track(&mut self, transceiver: RtpTransceiver) {
        let _ = self.event_tx.send(ClientEvent::Track(transceiver));
    }

    fn on_data_channel(&mut self, dc: DataChannel) {
        let _ = self.event_tx.send(ClientEvent::DataChannel(dc));
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

struct SignalingDcHandler {
    event_tx: mpsc::UnboundedSender<ClientEvent>,
}

impl DataChannelObserverHandler for SignalingDcHandler {
    fn on_message(&mut self, data: &[u8], _is_binary: bool) {
        let _ = self.event_tx.send(ClientEvent::SignalingMessage {
            data: data.to_vec(),
        });
    }
}

struct FrameCounterHandler {
    frame_count: Arc<AtomicUsize>,
    width: Arc<AtomicUsize>,
    height: Arc<AtomicUsize>,
}

impl VideoSinkHandler for FrameCounterHandler {
    fn on_frame(&mut self, frame: shiguredo_webrtc::VideoFrameRef<'_>) {
        self.frame_count.fetch_add(1, Ordering::Relaxed);
        // 最新のフレーム解像度を記録する
        self.width.store(frame.width() as usize, Ordering::Relaxed);
        self.height
            .store(frame.height() as usize, Ordering::Relaxed);
    }
}

// --- SDP ヘルパー ---

struct CreateSdpHandler {
    tx: std::sync::mpsc::Sender<Result<String, String>>,
    is_offer: bool,
}

impl CreateSessionDescriptionObserverHandler for CreateSdpHandler {
    fn on_success(&mut self, desc: SessionDescription) {
        let sdp = desc
            .to_string()
            .map_err(|e| format!("failed to convert SDP to string: {e}"));
        let _ = self.tx.send(sdp);
    }

    fn on_failure(&mut self, error: RtcError) {
        let message = error.message().unwrap_or_else(|_| "unknown".to_owned());
        let kind = if self.is_offer { "offer" } else { "answer" };
        let _ = self
            .tx
            .send(Err(format!("create_{kind} failed: {message}")));
    }
}

fn create_offer_sdp(pc: &PeerConnection) -> Result<String, String> {
    let mut options = PeerConnectionOfferAnswerOptions::new();
    options.set_offer_to_receive_audio(1);
    options.set_offer_to_receive_video(1);
    let (tx, rx) = std::sync::mpsc::channel();
    let mut observer =
        CreateSessionDescriptionObserver::new_with_handler(Box::new(CreateSdpHandler {
            tx,
            is_offer: true,
        }));
    pc.create_offer(&mut observer, &mut options);
    rx.recv_timeout(SDP_TIMEOUT)
        .map_err(|_| "create_offer timed out".to_owned())?
}

fn create_answer_sdp(pc: &PeerConnection) -> Result<String, String> {
    let mut options = PeerConnectionOfferAnswerOptions::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut observer =
        CreateSessionDescriptionObserver::new_with_handler(Box::new(CreateSdpHandler {
            tx,
            is_offer: false,
        }));
    pc.create_answer(&mut observer, &mut options);
    rx.recv_timeout(SDP_TIMEOUT)
        .map_err(|_| "create_answer timed out".to_owned())?
}

struct SetLocalSdpHandler {
    tx: std::sync::mpsc::Sender<Option<String>>,
}

impl SetLocalDescriptionObserverHandler for SetLocalSdpHandler {
    fn on_set_local_description_complete(&mut self, error: RtcError) {
        let message = if error.ok() {
            None
        } else {
            Some(error.message().unwrap_or_else(|_| "unknown".to_owned()))
        };
        let _ = self.tx.send(message);
    }
}

struct SetRemoteSdpHandler {
    tx: std::sync::mpsc::Sender<Option<String>>,
}

impl SetRemoteDescriptionObserverHandler for SetRemoteSdpHandler {
    fn on_set_remote_description_complete(&mut self, error: RtcError) {
        let message = if error.ok() {
            None
        } else {
            Some(error.message().unwrap_or_else(|_| "unknown".to_owned()))
        };
        let _ = self.tx.send(message);
    }
}

fn set_local_description(pc: &PeerConnection, sdp_type: SdpType, sdp: &str) -> Result<(), String> {
    let description =
        SessionDescription::new(sdp_type, sdp).map_err(|e| format!("failed to parse SDP: {e}"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    let observer =
        SetLocalDescriptionObserver::new_with_handler(Box::new(SetLocalSdpHandler { tx }));
    pc.set_local_description(description, &observer);
    let result = rx
        .recv_timeout(SDP_TIMEOUT)
        .map_err(|_| "set_local_description timed out".to_owned())?;
    if let Some(message) = result {
        return Err(format!("set_local_description failed: {message}"));
    }
    Ok(())
}

fn set_remote_description(pc: &PeerConnection, sdp_type: SdpType, sdp: &str) -> Result<(), String> {
    let description =
        SessionDescription::new(sdp_type, sdp).map_err(|e| format!("failed to parse SDP: {e}"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    let observer =
        SetRemoteDescriptionObserver::new_with_handler(Box::new(SetRemoteSdpHandler { tx }));
    pc.set_remote_description(description, &observer);
    let result = rx
        .recv_timeout(SDP_TIMEOUT)
        .map_err(|_| "set_remote_description timed out".to_owned())?;
    if let Some(message) = result {
        return Err(format!("set_remote_description failed: {message}"));
    }
    Ok(())
}

// --- HTTP bootstrap ---

async fn http_bootstrap(host: &str, port: u16, offer_sdp: &str) -> Result<String, String> {
    let mut stream = tokio::net::TcpStream::connect(format!("{host}:{port}"))
        .await
        .map_err(|e| format!("failed to connect: {e}"))?;

    let mut request = Request::new("POST", "/bootstrap");
    request.add_header("Content-Type", "application/sdp");
    request.add_header("Host", &format!("{host}:{port}"));
    request.add_header("Connection", "close");
    request.body = offer_sdp.as_bytes().to_vec();

    stream
        .write_all(&request.encode())
        .await
        .map_err(|e| format!("failed to send request: {e}"))?;
    stream
        .flush()
        .await
        .map_err(|e| format!("failed to flush: {e}"))?;

    let mut decoder = ResponseDecoder::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| format!("failed to read response: {e}"))?;
        if n == 0 {
            return Err("server closed connection without response".to_owned());
        }
        decoder
            .feed(&buf[..n])
            .map_err(|e| format!("failed to decode response: {e}"))?;
        if let Some(response) = decoder
            .decode()
            .map_err(|e| format!("failed to decode response: {e}"))?
        {
            if response.status_code != 201 {
                return Err(format!(
                    "bootstrap failed: {} {}",
                    response.status_code, response.reason_phrase
                ));
            }
            return String::from_utf8(response.body)
                .map_err(|e| format!("invalid UTF-8 in answer SDP: {e}"));
        }
    }
}

// --- signaling DC メッセージパーサ ---

fn parse_signaling_type(data: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(data).ok()?;
    let json = nojson::RawJson::parse(text).ok()?;
    let msg_type: String = json
        .value()
        .to_member("type")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    Some(msg_type)
}

fn parse_signaling_sdp(data: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(data).ok()?;
    let json = nojson::RawJson::parse(text).ok()?;
    let sdp: String = json
        .value()
        .to_member("sdp")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    Some(sdp)
}

fn make_answer_json(sdp: &str) -> String {
    nojson::object(|f| {
        f.member("type", "answer")?;
        f.member("sdp", sdp)
    })
    .to_string()
}

// --- main ---

fn main() -> noargs::Result<()> {
    let mut args = noargs::raw_args();
    args.metadata_mut().app_name = "obsws_scenario_client";
    noargs::HELP_FLAG.take_help(&mut args);

    let verbose = noargs::flag("verbose")
        .short('v')
        .doc("詳細ログを出力する")
        .take(&mut args)
        .is_present();

    if !noargs::cmd("bootstrap")
        .doc("bootstrap エンドポイントで WebRTC 接続し、トラック受信統計を出力する")
        .take(&mut args)
        .is_present()
    {
        if let Some(help) = args.finish()? {
            print!("{help}");
        }
        return Ok(());
    }

    let host: String = noargs::opt("host")
        .default("127.0.0.1")
        .doc("接続先ホスト")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let port: u16 = noargs::opt("port")
        .doc("接続先ポート")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let duration: u64 = noargs::opt("duration")
        .default("5")
        .doc("トラック受信を待つ秒数")
        .take(&mut args)
        .then(|o| o.value().parse())?;

    if args.metadata().help_mode {
        return Ok(());
    }

    if verbose {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_target(false)
            .with_writer(std::io::stderr)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_writer(std::io::stderr)
            .init();
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let result = runtime.block_on(async {
        let local = tokio::task::LocalSet::new();
        local.run_until(run_client(&host, port, duration)).await
    });

    match result {
        Ok(stats) => {
            // JSON 統計を stdout に出力する
            let json = nojson::object(|f| {
                f.member("video_tracks_received", stats.video_tracks)?;
                f.member("audio_tracks_received", stats.audio_tracks)?;
                f.member("video_frames_received", stats.video_frames)?;
                f.member("video_width", stats.video_width)?;
                f.member("video_height", stats.video_height)?;
                f.member("connection_state", stats.connection_state.as_str())
            });
            println!("{json}");
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }

    Ok(())
}

struct Stats {
    video_tracks: usize,
    audio_tracks: usize,
    video_frames: usize,
    video_width: usize,
    video_height: usize,
    connection_state: String,
}

#[derive(Clone)]
struct GatheredIceCandidate {
    sdp_mid: String,
    sdp_mline_index: i32,
    candidate: String,
}

struct RetainedState {
    event_tx: mpsc::UnboundedSender<ClientEvent>,
    _pc_observer: PeerConnectionObserver,
    _bootstrap_dc: DataChannel,
    signaling_dc: Option<DataChannel>,
    dc_observer: Option<DataChannelObserver>,
    video_sinks: Vec<VideoSink>,
    ice_rx: std::sync::mpsc::Receiver<IceObserverEvent>,
    ice_candidates: Vec<GatheredIceCandidate>,
}

fn finalize_local_sdp(
    initial_sdp: String,
    ice_rx: &std::sync::mpsc::Receiver<IceObserverEvent>,
    cached_candidates: &mut Vec<GatheredIceCandidate>,
) -> Result<String, String> {
    if initial_sdp.contains("\r\na=candidate:") {
        return Ok(initial_sdp);
    }

    let mut candidates = Vec::new();
    let mut complete = false;
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

    let deadline = std::time::Instant::now() + SDP_TIMEOUT;
    while !complete {
        let Some(timeout) = deadline.checked_duration_since(std::time::Instant::now()) else {
            if !cached_candidates.is_empty() {
                return Ok(append_ice_candidates_to_sdp(
                    &initial_sdp,
                    cached_candidates,
                ));
            }
            return Err("ICE gathering timed out".to_owned());
        };
        match ice_rx.recv_timeout(timeout) {
            Ok(IceObserverEvent::Candidate {
                sdp_mid,
                sdp_mline_index,
                candidate,
            }) => {
                candidates.push(GatheredIceCandidate {
                    sdp_mid,
                    sdp_mline_index,
                    candidate,
                });
            }
            Ok(IceObserverEvent::Complete) => {
                complete = true;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if !cached_candidates.is_empty() {
                    return Ok(append_ice_candidates_to_sdp(
                        &initial_sdp,
                        cached_candidates,
                    ));
                }
                return Err("ICE gathering timed out".to_owned());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("ICE gathering channel closed".to_owned());
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

async fn run_client(host: &str, port: u16, duration_secs: u64) -> Result<Stats, String> {
    // WebRTC ファクトリを初期化する
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
    deps.set_event_log_factory(RtcEventLogFactory::new());

    let adm = AudioDeviceModule::new(&env, AudioDeviceModuleAudioLayer::Dummy)
        .map_err(|e| format!("failed to create AudioDeviceModule: {e}"))?;
    deps.set_audio_device_module(&adm);
    deps.set_audio_encoder_factory(&AudioEncoderFactory::builtin());
    deps.set_audio_decoder_factory(&AudioDecoderFactory::builtin());
    deps.set_video_encoder_factory(VideoEncoderFactory::builtin());
    deps.set_video_decoder_factory(VideoDecoderFactory::builtin());
    deps.set_audio_processing_builder(AudioProcessingBuilder::new_builtin());
    deps.enable_media();

    let factory = Arc::new(
        PeerConnectionFactory::create_modular(&mut deps)
            .map_err(|e| format!("failed to create PeerConnectionFactory: {e}"))?,
    );

    // PeerConnection を作成する
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<ClientEvent>();
    let (ice_tx, ice_rx) = std::sync::mpsc::channel::<IceObserverEvent>();
    let pc_observer = PeerConnectionObserver::new_with_handler(Box::new(ClientPcObserver {
        event_tx: event_tx.clone(),
        ice_tx,
    }));
    let mut pc_deps = PeerConnectionDependencies::new(&pc_observer);
    let mut config = PeerConnectionRtcConfiguration::new();

    let pc = PeerConnection::create(factory.as_ref(), &mut config, &mut pc_deps)
        .map_err(|e| format!("failed to create PeerConnection: {e}"))?;

    let mut dc_init = DataChannelInit::new();
    dc_init.set_ordered(true);
    let bootstrap_dc = pc
        .create_data_channel("bootstrap", &mut dc_init)
        .map_err(|e| format!("failed to create bootstrap DataChannel: {e}"))?;

    // offer SDP を生成する
    let offer_sdp = create_offer_sdp(&pc)?;
    set_local_description(&pc, SdpType::Offer, &offer_sdp)?;
    let mut initial_ice_candidates = Vec::new();
    let offer_sdp = finalize_local_sdp(offer_sdp, &ice_rx, &mut initial_ice_candidates)?;
    tracing::debug!("offer SDP created");

    // /bootstrap で answer SDP を取得する
    let answer_sdp = http_bootstrap(host, port, &offer_sdp).await?;
    set_remote_description(&pc, SdpType::Answer, &answer_sdp)?;
    tracing::info!("bootstrap completed");

    // 統計カウンタ
    let video_tracks = Arc::new(AtomicUsize::new(0));
    let audio_tracks = Arc::new(AtomicUsize::new(0));
    let video_frames = Arc::new(AtomicUsize::new(0));
    let video_width = Arc::new(AtomicUsize::new(0));
    let video_height = Arc::new(AtomicUsize::new(0));
    let connection_state = Arc::new(std::sync::Mutex::new("new".to_owned()));

    let mut retained = RetainedState {
        event_tx: event_tx.clone(),
        _pc_observer: pc_observer,
        _bootstrap_dc: bootstrap_dc,
        signaling_dc: None,
        dc_observer: None,
        video_sinks: Vec::new(),
        ice_rx,
        ice_candidates: initial_ice_candidates,
    };

    // イベントループ（duration 秒間）
    let deadline = tokio::time::Instant::now() + Duration::from_secs(duration_secs);
    loop {
        let event = tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Some(e) => e,
                    None => break,
                }
            }
            _ = tokio::time::sleep_until(deadline) => break,
        };

        match event {
            ClientEvent::ConnectionChange(state) => {
                tracing::info!("connection state: {state:?}");
                let state_str = match state {
                    PeerConnectionState::New => "new",
                    PeerConnectionState::Connecting => "connecting",
                    PeerConnectionState::Connected => "connected",
                    PeerConnectionState::Disconnected => "disconnected",
                    PeerConnectionState::Failed => "failed",
                    PeerConnectionState::Closed => "closed",
                    PeerConnectionState::Unknown(_) => "unknown",
                };
                *connection_state.lock().unwrap() = state_str.to_owned();
            }
            ClientEvent::Track(transceiver) => {
                let receiver = transceiver.receiver();
                let track = receiver.track();
                let kind = track.kind().unwrap_or_default();
                tracing::info!("track received: kind={kind}");
                match kind.as_str() {
                    "video" => {
                        video_tracks.fetch_add(1, Ordering::Relaxed);
                        // VideoSink を登録してフレーム受信をカウントする
                        let mut video_track = track.cast_to_video_track();
                        let sink = VideoSink::new_with_handler(Box::new(FrameCounterHandler {
                            frame_count: video_frames.clone(),
                            width: video_width.clone(),
                            height: video_height.clone(),
                        }));
                        let wants = VideoSinkWants::default();
                        video_track.add_or_update_sink(&sink, &wants);
                        retained.video_sinks.push(sink);
                    }
                    "audio" => {
                        audio_tracks.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {
                        tracing::warn!("unknown track kind: {kind}");
                    }
                }
            }
            ClientEvent::DataChannel(mut dc) => {
                let label = dc.label().unwrap_or_default();
                tracing::info!("data channel received: label={label}");
                if label == "signaling" {
                    let observer =
                        DataChannelObserver::new_with_handler(Box::new(SignalingDcHandler {
                            event_tx: retained.event_tx.clone(),
                        }));
                    dc.register_observer(&observer);
                    retained.signaling_dc = Some(dc);
                    retained.dc_observer = Some(observer);
                }
            }
            ClientEvent::SignalingMessage { data } => {
                let msg_type = parse_signaling_type(&data).unwrap_or_default();
                tracing::debug!("signaling message: type={msg_type}");
                if msg_type == "offer" {
                    // renegotiation: サーバーからの offer に answer を返す
                    if let Some(sdp) = parse_signaling_sdp(&data) {
                        if let Err(e) = set_remote_description(&pc, SdpType::Offer, &sdp) {
                            tracing::warn!("failed to set remote offer: {e}");
                            continue;
                        }
                        match create_answer_sdp(&pc) {
                            Ok(answer) => {
                                if let Err(e) = set_local_description(&pc, SdpType::Answer, &answer)
                                {
                                    tracing::warn!("failed to set local answer: {e}");
                                    continue;
                                }
                                let answer = match finalize_local_sdp(
                                    answer,
                                    &retained.ice_rx,
                                    &mut retained.ice_candidates,
                                ) {
                                    Ok(answer) => answer,
                                    Err(e) => {
                                        tracing::warn!("failed to gather ICE candidates: {e}");
                                        continue;
                                    }
                                };
                                let answer_json = make_answer_json(&answer);
                                if let Some(dc) = &retained.signaling_dc {
                                    dc.send(answer_json.as_bytes(), false);
                                    tracing::info!("renegotiation answer sent");
                                }
                            }
                            Err(e) => {
                                tracing::warn!("failed to create answer: {e}");
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(Stats {
        video_tracks: video_tracks.load(Ordering::Relaxed),
        audio_tracks: audio_tracks.load(Ordering::Relaxed),
        video_frames: video_frames.load(Ordering::Relaxed),
        video_width: video_width.load(Ordering::Relaxed),
        video_height: video_height.load(Ordering::Relaxed),
        connection_state: connection_state.lock().unwrap().clone(),
    })
}
