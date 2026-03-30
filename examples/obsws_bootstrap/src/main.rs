use std::io::{Seek, SeekFrom, Write};
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use shiguredo_http11::{Request, ResponseDecoder};
use shiguredo_mp4::FixedPointNumber;
use shiguredo_mp4::Uint;
use shiguredo_mp4::boxes::{
    AudioSampleEntryFields, DopsBox, OpusBox, SampleEntry, VisualSampleEntryFields, Vp09Box,
    VpccBox,
};
use shiguredo_mp4::mux::{Mp4FileMuxer, Mp4FileMuxerOptions, Sample};
use shiguredo_webrtc::{
    AudioDecoderFactory, AudioDeviceModule, AudioDeviceModuleHandler, AudioEncoderFactory,
    AudioProcessingBuilder, AudioTrackSink, AudioTrackSinkHandler, AudioTransportRef,
    CreateSessionDescriptionObserver, CreateSessionDescriptionObserverHandler, DataChannel,
    DataChannelInit, DataChannelObserver, DataChannelObserverHandler, DataChannelState,
    IceGatheringState, PeerConnection, PeerConnectionDependencies, PeerConnectionFactory,
    PeerConnectionFactoryDependencies, PeerConnectionObserver, PeerConnectionObserverHandler,
    PeerConnectionOfferAnswerOptions, PeerConnectionRtcConfiguration, PeerConnectionState,
    RtcError, RtcEventLogFactory, RtpTransceiver, SdpType, SessionDescription,
    SetLocalDescriptionObserver, SetLocalDescriptionObserverHandler, SetRemoteDescriptionObserver,
    SetRemoteDescriptionObserverHandler, Thread, VideoDecoderFactory, VideoEncoderFactory,
    VideoSink, VideoSinkHandler, VideoSinkWants,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

const SDP_TIMEOUT: Duration = Duration::from_secs(5);
const CREATE_INPUT_REQUEST_ID: &str = "req-create-input";
const GET_WEBRTC_STATS_REQUEST_ID: &str = "req-get-webrtc-stats";
const SUBSCRIBE_PROGRAM_TRACKS_REQUEST_ID: &str = "req-subscribe-program-tracks";
const MAX_FRAMES_PER_POLL: usize = 8;
const INITIAL_VIDEO_FRAME_GRACE: Duration = Duration::from_secs(2);

// MP4 のタイムスケールはマイクロ秒固定にする
const TIMESCALE: NonZeroU32 = NonZeroU32::MIN.saturating_add(1_000_000 - 1);

// VP9 SampleEntry 用の定数
const CHROMA_SUBSAMPLING_I420: Uint<u8, 3, 1> = Uint::new(1);
const BIT_DEPTH: Uint<u8, 4, 4> = Uint::new(8);
const LEGAL_RANGE: Uint<u8, 1> = Uint::new(0);
const BT_709: u8 = 1;

// --- イベント ---

enum ClientEvent {
    ConnectionChange(PeerConnectionState),
    Track(RtpTransceiver),
    DataChannel(DataChannel, Option<DataChannelObserver>),
    ObswsDataChannelStateChange,
    SignalingMessage { data: Vec<u8> },
    ObswsMessage { data: Vec<u8> },
}

// VideoSinkHandler から送信するフレームデータ
struct VideoFrameData {
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    width: i32,
    height: i32,
    stride_y: usize,
    stride_u: usize,
    stride_v: usize,
    timestamp_us: i64,
}

// AudioTrackSinkHandler から送信する音声データ
struct AudioFrameData {
    pcm: Vec<i16>,
    sample_rate: i32,
    channels: usize,
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
    ice_tx: mpsc::UnboundedSender<IceObserverEvent>,
}

impl PeerConnectionObserverHandler for ClientPcObserver {
    fn on_connection_change(&mut self, state: PeerConnectionState) {
        let _ = self.event_tx.send(ClientEvent::ConnectionChange(state));
    }

    fn on_track(&mut self, transceiver: RtpTransceiver) {
        let _ = self.event_tx.send(ClientEvent::Track(transceiver));
    }

    fn on_data_channel(&mut self, mut dc: DataChannel) {
        let label = dc.label().unwrap_or_default();
        let observer = if label == "signaling" {
            let observer = DataChannelObserver::new_with_handler(Box::new(SignalingDcHandler {
                event_tx: self.event_tx.clone(),
            }));
            dc.register_observer(&observer);
            Some(observer)
        } else if label == "obsws" {
            let observer = DataChannelObserver::new_with_handler(Box::new(ObswsDcHandler {
                event_tx: self.event_tx.clone(),
            }));
            dc.register_observer(&observer);
            Some(observer)
        } else {
            None
        };
        let _ = self.event_tx.send(ClientEvent::DataChannel(dc, observer));
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

struct ObswsDcHandler {
    event_tx: mpsc::UnboundedSender<ClientEvent>,
}

impl DataChannelObserverHandler for ObswsDcHandler {
    fn on_state_change(&mut self) {
        let _ = self.event_tx.send(ClientEvent::ObswsDataChannelStateChange);
    }

    fn on_message(&mut self, data: &[u8], _is_binary: bool) {
        let _ = self.event_tx.send(ClientEvent::ObswsMessage {
            data: data.to_vec(),
        });
    }
}

// フレームデータをチャネルで送信するハンドラ
struct FrameRecordHandler {
    frame_count: Arc<AtomicUsize>,
    first_frame_logged: Arc<AtomicBool>,
    width: Arc<AtomicUsize>,
    height: Arc<AtomicUsize>,
    frame_tx: std::sync::mpsc::SyncSender<VideoFrameData>,
}

impl VideoSinkHandler for FrameRecordHandler {
    fn on_frame(&mut self, frame: shiguredo_webrtc::VideoFrameRef<'_>) {
        let previous = self.frame_count.fetch_add(1, Ordering::Relaxed);
        let w = frame.width();
        let h = frame.height();
        self.width.store(w as usize, Ordering::Relaxed);
        self.height.store(h as usize, Ordering::Relaxed);
        if previous == 0 && !self.first_frame_logged.swap(true, Ordering::Relaxed) {
            tracing::info!(
                "first video frame received: width={w}, height={h}, timestamp_us={}",
                frame.timestamp_us()
            );
        }

        // I420 バッファからプレーンデータをコピーする
        let buffer = frame.buffer();
        let data = VideoFrameData {
            y: buffer.y_data().to_vec(),
            u: buffer.u_data().to_vec(),
            v: buffer.v_data().to_vec(),
            width: w,
            height: h,
            stride_y: buffer.stride_y() as usize,
            stride_u: buffer.stride_u() as usize,
            stride_v: buffer.stride_v() as usize,
            timestamp_us: frame.timestamp_us(),
        };
        // バッファがいっぱいの場合はフレームを捨てる
        let _ = self.frame_tx.try_send(data);
    }
}

// 受信音声データをチャネルで送信するハンドラ
struct AudioRecordHandler {
    audio_frame_count: Arc<AtomicUsize>,
    audio_tx: std::sync::mpsc::SyncSender<AudioFrameData>,
}

impl AudioTrackSinkHandler for AudioRecordHandler {
    fn on_data(
        &mut self,
        audio_data: &[u8],
        bits_per_sample: i32,
        sample_rate: i32,
        number_of_channels: usize,
        _number_of_frames: usize,
    ) {
        self.audio_frame_count.fetch_add(1, Ordering::Relaxed);
        if bits_per_sample != 16 {
            return;
        }
        // u8 スライスをネイティブエンディアン i16 に変換する
        let pcm: Vec<i16> = audio_data
            .chunks_exact(2)
            .map(|chunk| i16::from_ne_bytes([chunk[0], chunk[1]]))
            .collect();
        let _ = self.audio_tx.try_send(AudioFrameData {
            pcm,
            sample_rate,
            channels: number_of_channels,
        });
    }
}

struct BootstrapAudioDeviceModuleState {
    transport: Mutex<Option<AudioTransportRef>>,
    playing: AtomicBool,
}

impl BootstrapAudioDeviceModuleState {
    fn new() -> Self {
        Self {
            transport: Mutex::new(None),
            playing: AtomicBool::new(false),
        }
    }

    fn render_10ms_audio(&self) {
        let guard = self.transport.lock().unwrap();
        let Some(transport) = guard.as_ref() else {
            return;
        };

        let bits_per_sample = 16;
        let sample_rate = 48_000;
        let number_of_channels = 2;
        let number_of_frames = sample_rate as usize / 100;
        let mut audio_data =
            vec![0_u8; number_of_frames * number_of_channels * (bits_per_sample as usize / 8)];
        let mut elapsed_time_ms = 0_i64;
        let mut ntp_time_ms = 0_i64;

        unsafe {
            transport.pull_render_data(
                bits_per_sample,
                sample_rate,
                number_of_channels,
                number_of_frames,
                audio_data.as_mut_ptr(),
                &mut elapsed_time_ms,
                &mut ntp_time_ms,
            );
        }
    }

    fn shutdown(&self) {
        self.playing.store(false, Ordering::Relaxed);
        let mut guard = self.transport.lock().unwrap();
        *guard = None;
    }
}

struct BootstrapAudioDeviceModuleHandler {
    state: Arc<BootstrapAudioDeviceModuleState>,
}

impl BootstrapAudioDeviceModuleHandler {
    fn new(state: Arc<BootstrapAudioDeviceModuleState>) -> Self {
        Self { state }
    }
}

impl AudioDeviceModuleHandler for BootstrapAudioDeviceModuleHandler {
    fn register_audio_callback(&self, audio_transport: Option<AudioTransportRef>) -> i32 {
        let mut guard = self.state.transport.lock().unwrap();
        *guard = audio_transport;
        0
    }

    fn init(&self) -> i32 {
        0
    }

    fn terminate(&self) -> i32 {
        0
    }

    fn initialized(&self) -> bool {
        true
    }

    fn playout_is_available(&self, available: &mut bool) -> i32 {
        *available = true;
        0
    }

    fn init_playout(&self) -> i32 {
        0
    }

    fn playout_is_initialized(&self) -> bool {
        true
    }

    fn start_playout(&self) -> i32 {
        self.state.playing.store(true, Ordering::Relaxed);
        0
    }

    fn stop_playout(&self) -> i32 {
        self.state.playing.store(false, Ordering::Relaxed);
        0
    }

    fn playing(&self) -> bool {
        self.state.playing.load(Ordering::Relaxed)
    }

    fn stereo_playout_is_available(&self, available: &mut bool) -> i32 {
        *available = true;
        0
    }

    fn set_stereo_playout(&self, _enable: bool) -> i32 {
        0
    }

    fn stereo_playout(&self, enabled: &mut bool) -> i32 {
        *enabled = true;
        0
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
    tracing::info!("connecting to bootstrap endpoint: host={host}, port={port}");
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

fn make_create_mp4_input_request(input_path: &str) -> String {
    nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "CreateInput")?;
                f.member("requestId", CREATE_INPUT_REQUEST_ID)?;
                f.member(
                    "requestData",
                    nojson::object(|f| {
                        f.member("sceneName", "Scene")?;
                        f.member("inputName", "obsws-bootstrap-input")?;
                        f.member("inputKind", "mp4_file_source")?;
                        f.member(
                            "inputSettings",
                            nojson::object(|f| {
                                f.member("path", input_path)?;
                                f.member("loopPlayback", true)
                            }),
                        )?;
                        f.member("sceneItemEnabled", true)
                    }),
                )
            }),
        )
    })
    .to_string()
}

fn make_get_webrtc_stats_request() -> String {
    nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetWebRtcStats")?;
                f.member("requestId", GET_WEBRTC_STATS_REQUEST_ID)
            }),
        )
    })
    .to_string()
}

fn make_subscribe_program_tracks_request() -> String {
    nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SubscribeProgramTracks")?;
                f.member("requestId", SUBSCRIBE_PROGRAM_TRACKS_REQUEST_ID)
            }),
        )
    })
    .to_string()
}

fn parse_obsws_request_response(text: &str) -> Option<Result<(), String>> {
    let json = nojson::RawJson::parse(text).ok()?;
    let root = json.value();
    let op: i64 = root
        .to_member("op")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if op != 7 {
        return None;
    }

    let d = root.to_member("d").ok()?.required().ok()?;
    let request_id: String = d
        .to_member("requestId")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if request_id != CREATE_INPUT_REQUEST_ID {
        return None;
    }

    let request_status = d.to_member("requestStatus").ok()?.required().ok()?;
    let result: bool = request_status
        .to_member("result")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if result {
        return Some(Ok(()));
    }

    let comment: Option<String> =
        if let Some(v) = request_status.to_member("comment").ok()?.optional() {
            v.try_into().ok()
        } else {
            None
        };
    Some(Err(
        comment.unwrap_or_else(|| "CreateInput request failed".to_owned())
    ))
}

fn parse_subscribe_program_tracks_response(text: &str) -> Option<Result<(), String>> {
    let json = nojson::RawJson::parse(text).ok()?;
    let root = json.value();
    let op: i64 = root
        .to_member("op")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if op != 7 {
        return None;
    }

    let d = root.to_member("d").ok()?.required().ok()?;
    let request_id: String = d
        .to_member("requestId")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if request_id != SUBSCRIBE_PROGRAM_TRACKS_REQUEST_ID {
        return None;
    }

    let request_status = d.to_member("requestStatus").ok()?.required().ok()?;
    let result: bool = request_status
        .to_member("result")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if result {
        return Some(Ok(()));
    }

    let comment: Option<String> =
        if let Some(v) = request_status.to_member("comment").ok()?.optional() {
            v.try_into().ok()
        } else {
            None
        };
    Some(Err(comment.unwrap_or_else(|| {
        "SubscribeProgramTracks request failed".to_owned()
    })))
}

fn parse_obsws_server_webrtc_stats_response(text: &str) -> Option<Result<String, String>> {
    let json = nojson::RawJson::parse(text).ok()?;
    let root = json.value();
    let op: i64 = root
        .to_member("op")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if op != 7 {
        return None;
    }

    let d = root.to_member("d").ok()?.required().ok()?;
    let request_id: String = d
        .to_member("requestId")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if request_id != GET_WEBRTC_STATS_REQUEST_ID {
        return None;
    }

    let request_status = d.to_member("requestStatus").ok()?.required().ok()?;
    let result: bool = request_status
        .to_member("result")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if !result {
        let comment: Option<String> =
            if let Some(v) = request_status.to_member("comment").ok()?.optional() {
                v.try_into().ok()
            } else {
                None
            };
        return Some(Err(
            comment.unwrap_or_else(|| "GetWebRtcStats request failed".to_owned())
        ));
    }

    let response_data = d.to_member("responseData").ok()?.required().ok()?;
    let stats = response_data.to_member("stats").ok()?.required().ok()?;
    Some(Ok(stats.as_raw_str().to_owned()))
}

// --- VP9 SampleEntry ---

fn vp9_sample_entry(width: usize, height: usize) -> SampleEntry {
    SampleEntry::Vp09(Vp09Box {
        visual: VisualSampleEntryFields {
            data_reference_index: VisualSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
            width: width as u16,
            height: height as u16,
            horizresolution: VisualSampleEntryFields::DEFAULT_HORIZRESOLUTION,
            vertresolution: VisualSampleEntryFields::DEFAULT_VERTRESOLUTION,
            frame_count: VisualSampleEntryFields::DEFAULT_FRAME_COUNT,
            compressorname: VisualSampleEntryFields::NULL_COMPRESSORNAME,
            depth: VisualSampleEntryFields::DEFAULT_DEPTH,
        },
        vpcc_box: VpccBox {
            profile: 0,
            level: 0,
            bit_depth: BIT_DEPTH,
            chroma_subsampling: CHROMA_SUBSAMPLING_I420,
            video_full_range_flag: LEGAL_RANGE,
            colour_primaries: BT_709,
            transfer_characteristics: BT_709,
            matrix_coefficients: BT_709,
            codec_initialization_data: Vec::new(),
        },
        unknown_boxes: Vec::new(),
    })
}

// --- Opus SampleEntry ---

fn opus_sample_entry_value(channels: u8, pre_skip: u16) -> SampleEntry {
    SampleEntry::Opus(OpusBox {
        audio: AudioSampleEntryFields {
            data_reference_index: AudioSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
            channelcount: channels as u16,
            samplesize: AudioSampleEntryFields::DEFAULT_SAMPLESIZE,
            samplerate: FixedPointNumber::new(48000u16, 0u16),
        },
        dops_box: DopsBox {
            output_channel_count: channels,
            pre_skip,
            input_sample_rate: 48000,
            output_gain: 0,
        },
        unknown_boxes: Vec::new(),
    })
}

// --- MP4 ライター ---

struct SimpleMp4Writer {
    file: std::io::BufWriter<std::fs::File>,
    muxer: Mp4FileMuxer,
    next_position: u64,
    video_sample_entry: Option<SampleEntry>,
    video_sample_count: usize,
    last_video_timestamp_us: Option<i64>,
    audio_sample_entry: Option<SampleEntry>,
    audio_sample_count: usize,
}

impl SimpleMp4Writer {
    fn new(path: &str) -> Result<Self, String> {
        let muxer_options = Mp4FileMuxerOptions {
            creation_timestamp: std::time::UNIX_EPOCH
                .elapsed()
                .map_err(|e| format!("failed to get epoch: {e}"))?,
            reserved_moov_box_size: 0,
        };
        let muxer =
            Mp4FileMuxer::with_options(muxer_options).map_err(|e| format!("muxer error: {e}"))?;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .map_err(|e| format!("failed to create MP4 file: {e}"))?;

        let initial_bytes = muxer.initial_boxes_bytes();
        file.write_all(initial_bytes)
            .map_err(|e| format!("failed to write initial boxes: {e}"))?;
        let next_position = initial_bytes.len() as u64;

        Ok(Self {
            file: std::io::BufWriter::new(file),
            muxer,
            next_position,
            video_sample_entry: None,
            video_sample_count: 0,
            last_video_timestamp_us: None,
            audio_sample_entry: None,
            audio_sample_count: 0,
        })
    }

    fn append_video(
        &mut self,
        data: &[u8],
        keyframe: bool,
        sample_entry: Option<SampleEntry>,
        timestamp_us: i64,
    ) -> Result<(), String> {
        // duration は前のフレームとのタイムスタンプ差から計算する
        let duration_us = if let Some(last_ts) = self.last_video_timestamp_us {
            let d = timestamp_us - last_ts;
            if d > 0 { d as u32 } else { 33333 } // デフォルト 30fps 相当
        } else {
            33333
        };
        self.last_video_timestamp_us = Some(timestamp_us);

        self.file
            .write_all(data)
            .map_err(|e| format!("failed to write video data: {e}"))?;

        let sample = Sample {
            track_kind: shiguredo_mp4::TrackKind::Video,
            sample_entry: sample_entry.or_else(|| self.video_sample_entry.clone()),
            keyframe,
            timescale: TIMESCALE,
            duration: duration_us,
            composition_time_offset: None,
            data_offset: self.next_position,
            data_size: data.len(),
        };

        // 最初のサンプルで sample_entry を記録する
        if self.video_sample_entry.is_none() {
            self.video_sample_entry = sample.sample_entry.clone();
        }

        self.muxer
            .append_sample(&sample)
            .map_err(|e| format!("failed to append video sample: {e}"))?;
        self.next_position += data.len() as u64;
        self.video_sample_count += 1;
        Ok(())
    }

    fn append_audio(
        &mut self,
        data: &[u8],
        sample_entry: Option<SampleEntry>,
        duration: u32,
    ) -> Result<(), String> {
        self.file
            .write_all(data)
            .map_err(|e| format!("failed to write audio data: {e}"))?;

        let sample = Sample {
            track_kind: shiguredo_mp4::TrackKind::Audio,
            sample_entry: sample_entry.or_else(|| self.audio_sample_entry.clone()),
            keyframe: true,
            timescale: TIMESCALE,
            duration,
            composition_time_offset: None,
            data_offset: self.next_position,
            data_size: data.len(),
        };

        if self.audio_sample_entry.is_none() {
            self.audio_sample_entry = sample.sample_entry.clone();
        }

        self.muxer
            .append_sample(&sample)
            .map_err(|e| format!("failed to append audio sample: {e}"))?;
        self.next_position += data.len() as u64;
        self.audio_sample_count += 1;
        Ok(())
    }

    fn finalize(&mut self) -> Result<(), String> {
        let finalized = self
            .muxer
            .finalize()
            .map_err(|e| format!("failed to finalize muxer: {e}"))?;
        for (offset, bytes) in finalized.offset_and_bytes_pairs() {
            self.file
                .seek(SeekFrom::Start(offset))
                .map_err(|e| format!("failed to seek: {e}"))?;
            self.file
                .write_all(bytes)
                .map_err(|e| format!("failed to write finalized data: {e}"))?;
        }
        self.file
            .flush()
            .map_err(|e| format!("failed to flush: {e}"))?;
        Ok(())
    }
}

// --- main ---

fn main() -> noargs::Result<()> {
    let mut args = noargs::raw_args();
    args.metadata_mut().app_name = "obsws_bootstrap";
    noargs::HELP_FLAG.take_help(&mut args);

    let verbose = noargs::flag("verbose")
        .short('v')
        .doc("詳細ログを出力する")
        .take(&mut args)
        .is_present();

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
    let output_path: String = noargs::opt("output-path")
        .doc("MP4 出力先パス")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let input_mp4_path: String = noargs::opt("input-mp4-path")
        .doc("obsws 経由で入力として追加する MP4 ファイルパス")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let subscribe_program_tracks = noargs::flag("subscribe-program-tracks")
        .doc("Program 合成結果トラックを購読する")
        .take(&mut args)
        .is_present();

    args.finish()?;

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
        local
            .run_until(run_client(
                &host,
                port,
                duration,
                &output_path,
                &input_mp4_path,
                subscribe_program_tracks,
            ))
            .await
    });

    match result {
        Ok(stats) => {
            let json = nojson::object(|f| {
                f.member("video_tracks_received", stats.video_tracks)?;
                f.member("audio_tracks_received", stats.audio_tracks)?;
                f.member("video_frames_received", stats.video_frames)?;
                f.member("audio_frames_received", stats.audio_frames)?;
                f.member("video_width", stats.video_width)?;
                f.member("video_height", stats.video_height)?;
                f.member("video_codec", stats.video_codec.as_str())?;
                f.member("audio_codec", stats.audio_codec.as_str())?;
                f.member("video_samples_written", stats.video_samples_written)?;
                f.member("audio_samples_written", stats.audio_samples_written)?;
                f.member("connection_state", stats.connection_state.as_str())?;
                f.member("webrtc_stats_error", stats.webrtc_stats_error.as_str())?;
                f.member("program_tracks_subscribed", stats.program_tracks_subscribed)
            });
            println!("{json}");
            Ok(())
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

struct Stats {
    video_tracks: usize,
    audio_tracks: usize,
    video_frames: usize,
    audio_frames: usize,
    video_width: usize,
    video_height: usize,
    video_codec: String,
    audio_codec: String,
    video_samples_written: usize,
    audio_samples_written: usize,
    connection_state: String,
    webrtc_stats_error: String,
    program_tracks_subscribed: bool,
}

async fn collect_webrtc_stats_json(pc: &PeerConnection) -> Result<String, String> {
    let (tx, rx) = oneshot::channel();
    pc.get_stats(move |report| {
        let _ = tx.send(
            report
                .to_json()
                .map_err(|e| format!("failed to serialize WebRTC stats: {e}")),
        );
    });

    tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .map_err(|_| "timed out waiting for WebRTC stats".to_owned())?
        .map_err(|_| "WebRTC stats callback channel closed".to_owned())?
}

async fn request_server_webrtc_stats(
    retained: &RetainedState,
    event_rx: &mut mpsc::UnboundedReceiver<ClientEvent>,
) -> Result<String, String> {
    let Some(dc) = retained.obsws_dc.as_ref() else {
        return Err("obsws DataChannel is not available".to_owned());
    };
    if dc.state() != DataChannelState::Open {
        return Err("obsws DataChannel is not open".to_owned());
    }

    let request = make_get_webrtc_stats_request();
    if !dc.send(request.as_bytes(), false) {
        return Err("failed to send GetWebRtcStats request".to_owned());
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err("timed out waiting for GetWebRtcStats response".to_owned());
        }

        let event = tokio::time::timeout(remaining, event_rx.recv())
            .await
            .map_err(|_| "timed out waiting for GetWebRtcStats response".to_owned())?
            .ok_or_else(|| {
                "event channel closed while waiting for GetWebRtcStats response".to_owned()
            })?;

        if let ClientEvent::ObswsMessage { data } = event {
            let text = std::str::from_utf8(&data)
                .map_err(|e| format!("GetWebRtcStats response was not UTF-8: {e}"))?;
            if let Some(result) = parse_obsws_server_webrtc_stats_response(text) {
                return result;
            }
        }
    }
}

fn summarize_webrtc_stats_json(stats_json: &str) -> String {
    let Ok(json) = nojson::RawJson::parse(stats_json) else {
        return "failed to parse stats json".to_owned();
    };

    let mut inbound_audio = 0usize;
    let mut inbound_video = 0usize;
    let mut outbound_audio = 0usize;
    let mut outbound_video = 0usize;
    let mut remote_inbound_audio = 0usize;
    let mut remote_inbound_video = 0usize;
    let mut remote_outbound_audio = 0usize;
    let mut remote_outbound_video = 0usize;
    let mut audio_codecs = Vec::new();
    let mut video_codecs = Vec::new();

    let root = json.value();
    let Ok(stats_objects) = root.to_object() else {
        return "stats root is not an object".to_owned();
    };

    for (_, stats) in stats_objects {
        let Ok(stats_type) = stats
            .to_member("type")
            .and_then(|v| v.required())
            .and_then(|v| v.to_unquoted_string_str())
        else {
            continue;
        };

        let mut kind = None;
        if let Ok(kind_member) = stats.to_member("kind")
            && let Some(kind_value) = kind_member.optional()
            && let Ok(kind_str) = kind_value.to_unquoted_string_str()
        {
            kind = Some(kind_str.to_string());
        }

        match (stats_type.as_ref(), kind.as_deref()) {
            ("inbound-rtp", Some("audio")) => inbound_audio += 1,
            ("inbound-rtp", Some("video")) => inbound_video += 1,
            ("outbound-rtp", Some("audio")) => outbound_audio += 1,
            ("outbound-rtp", Some("video")) => outbound_video += 1,
            ("remote-inbound-rtp", Some("audio")) => remote_inbound_audio += 1,
            ("remote-inbound-rtp", Some("video")) => remote_inbound_video += 1,
            ("remote-outbound-rtp", Some("audio")) => remote_outbound_audio += 1,
            ("remote-outbound-rtp", Some("video")) => remote_outbound_video += 1,
            ("codec", _) => {
                let mut mime_type = None;
                if let Ok(mime_type_member) = stats.to_member("mimeType")
                    && let Some(mime_type_value) = mime_type_member.optional()
                    && let Ok(mime_type_str) = mime_type_value.to_unquoted_string_str()
                {
                    mime_type = Some(mime_type_str.to_string());
                }
                if let Some(mime_type) = mime_type {
                    if mime_type.starts_with("audio/") {
                        audio_codecs.push(mime_type);
                    } else if mime_type.starts_with("video/") {
                        video_codecs.push(mime_type);
                    }
                }
            }
            _ => {}
        }
    }

    audio_codecs.sort();
    audio_codecs.dedup();
    video_codecs.sort();
    video_codecs.dedup();

    format!(
        "inbound_audio={inbound_audio}, inbound_video={inbound_video}, outbound_audio={outbound_audio}, outbound_video={outbound_video}, remote_inbound_audio={remote_inbound_audio}, remote_inbound_video={remote_inbound_video}, remote_outbound_audio={remote_outbound_audio}, remote_outbound_video={remote_outbound_video}, audio_codecs=[{}], video_codecs=[{}]",
        audio_codecs.join(", "),
        video_codecs.join(", ")
    )
}

fn summarize_sdp_for_log(sdp: &str) -> String {
    let mut sections = Vec::new();
    let mut current = Vec::new();
    for line in sdp.split("\r\n").filter(|line| !line.is_empty()) {
        if line.starts_with("m=") && !current.is_empty() {
            sections.push(current);
            current = Vec::new();
        }
        current.push(line);
    }
    if !current.is_empty() {
        sections.push(current);
    }

    let mut summary = Vec::new();
    for section in sections {
        let Some(first_line) = section.first() else {
            continue;
        };
        if !(first_line.starts_with("m=audio")
            || first_line.starts_with("m=video")
            || first_line.starts_with("m=application"))
        {
            continue;
        }
        summary.push((*first_line).to_owned());
        for line in &section {
            if line.starts_with("a=mid:")
                || line == &"a=sendrecv"
                || line == &"a=sendonly"
                || line == &"a=recvonly"
                || line == &"a=inactive"
                || line.starts_with("a=rtpmap:")
                || line.starts_with("a=fmtp:")
            {
                summary.push(format!("  {line}"));
            }
        }
    }
    summary.join("\n")
}

fn log_sdp_summary(label: &str, sdp: &str) {
    tracing::info!("{label}:\n{}", summarize_sdp_for_log(sdp));
}

fn log_transceiver_receiver_state(label: &str, transceiver: &RtpTransceiver) {
    let receiver = transceiver.receiver();
    let track = receiver.track();
    let kind = track.kind().unwrap_or_default();
    let track_id = track.id().unwrap_or_default();
    tracing::info!("{label}: receiver_track_kind={kind}, receiver_track_id={track_id}");
}

#[derive(Clone)]
struct GatheredIceCandidate {
    sdp_mid: String,
    sdp_mline_index: i32,
    candidate: String,
}

struct RetainedState {
    _pc_observer: PeerConnectionObserver,
    dummy_dc: DataChannel,
    obsws_dc: Option<DataChannel>,
    signaling_dc: Option<DataChannel>,
    signaling_dc_observer: Option<DataChannelObserver>,
    obsws_dc_observer: Option<DataChannelObserver>,
    video_sinks: Vec<RetainedVideoSink>,
    audio_sinks: Vec<RetainedAudioSink>,
    track_transceivers: Vec<RtpTransceiver>,
    ice_rx: mpsc::UnboundedReceiver<IceObserverEvent>,
    ice_candidates: Vec<GatheredIceCandidate>,
}

struct RetainedVideoSink {
    track_id: String,
    track: shiguredo_webrtc::VideoTrack,
    sink: VideoSink,
}

struct RetainedAudioSink {
    track_id: String,
    track: shiguredo_webrtc::AudioTrack,
    sink: AudioTrackSink,
}

struct VideoSinkAttachState<'a> {
    video_frames: &'a Arc<AtomicUsize>,
    first_video_frame_logged: &'a Arc<AtomicBool>,
    video_width: &'a Arc<AtomicUsize>,
    video_height: &'a Arc<AtomicUsize>,
    frame_tx: &'a std::sync::mpsc::SyncSender<VideoFrameData>,
}

fn attach_video_sink(
    retained: &mut RetainedState,
    track_id: &str,
    video_track: shiguredo_webrtc::VideoTrack,
    state: &VideoSinkAttachState<'_>,
) {
    if retained
        .video_sinks
        .iter()
        .any(|retained_sink| retained_sink.track_id == track_id)
    {
        return;
    }
    let mut video_track = video_track;
    let sink = VideoSink::new_with_handler(Box::new(FrameRecordHandler {
        frame_count: state.video_frames.clone(),
        first_frame_logged: state.first_video_frame_logged.clone(),
        width: state.video_width.clone(),
        height: state.video_height.clone(),
        frame_tx: state.frame_tx.clone(),
    }));
    let wants = VideoSinkWants::default();
    video_track.add_or_update_sink(&sink, &wants);
    retained.video_sinks.push(RetainedVideoSink {
        track_id: track_id.to_owned(),
        track: video_track,
        sink,
    });
}

fn attach_audio_sink(
    retained: &mut RetainedState,
    track_id: &str,
    audio_track: shiguredo_webrtc::AudioTrack,
    audio_frames: &Arc<AtomicUsize>,
    audio_tx: &std::sync::mpsc::SyncSender<AudioFrameData>,
) {
    if retained
        .audio_sinks
        .iter()
        .any(|retained_sink| retained_sink.track_id == track_id)
    {
        return;
    }
    let mut audio_track = audio_track;
    let sink = AudioTrackSink::new_with_handler(Box::new(AudioRecordHandler {
        audio_frame_count: audio_frames.clone(),
        audio_tx: audio_tx.clone(),
    }));
    audio_track.add_sink(&sink);
    retained.audio_sinks.push(RetainedAudioSink {
        track_id: track_id.to_owned(),
        track: audio_track,
        sink,
    });
}

async fn teardown_client(
    pc: &PeerConnection,
    retained: &mut RetainedState,
    audio_state: &BootstrapAudioDeviceModuleState,
) {
    for retained_sink in &mut retained.video_sinks {
        retained_sink.track.remove_sink(&retained_sink.sink);
    }
    for retained_sink in &mut retained.audio_sinks {
        retained_sink.track.remove_sink(&retained_sink.sink);
    }

    if let Some(dc) = retained.obsws_dc.as_ref() {
        dc.unregister_observer();
        dc.close();
    }
    if let Some(dc) = retained.signaling_dc.as_ref() {
        dc.unregister_observer();
        dc.close();
    }
    retained.dummy_dc.close();

    pc.close();
    audio_state.shutdown();

    // close 後の非同期コールバックが収束するまで少し待つ。
    tokio::time::sleep(Duration::from_millis(100)).await;
}

async fn finalize_local_sdp(
    initial_sdp: String,
    ice_rx: &mut mpsc::UnboundedReceiver<IceObserverEvent>,
    cached_candidates: &mut Vec<GatheredIceCandidate>,
) -> Result<String, String> {
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
    let deadline = tokio::time::Instant::now() + SDP_TIMEOUT;
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
                return Err("ICE gathering channel closed".to_owned());
            }
            Err(_) => {
                // タイムアウト
                if !cached_candidates.is_empty() {
                    return Ok(append_ice_candidates_to_sdp(
                        &initial_sdp,
                        cached_candidates,
                    ));
                }
                return Err("ICE gathering timed out".to_owned());
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

async fn run_client(
    host: &str,
    port: u16,
    duration_secs: u64,
    output_path: &str,
    input_mp4_path: &str,
    subscribe_program_tracks: bool,
) -> Result<Stats, String> {
    // WebRTC ファクトリを初期化する
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

    let audio_state = Arc::new(BootstrapAudioDeviceModuleState::new());
    let adm = AudioDeviceModule::new_with_handler(Box::new(
        BootstrapAudioDeviceModuleHandler::new(audio_state.clone()),
    ));
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
    let (ice_tx, mut ice_rx) = mpsc::unbounded_channel::<IceObserverEvent>();
    let pc_observer = PeerConnectionObserver::new_with_handler(Box::new(ClientPcObserver {
        event_tx: event_tx.clone(),
        ice_tx,
    }));
    let mut pc_deps = PeerConnectionDependencies::new(&pc_observer);
    let mut config = PeerConnectionRtcConfiguration::new();

    let pc = PeerConnection::create(factory.as_ref(), &mut config, &mut pc_deps)
        .map_err(|e| format!("failed to create PeerConnection: {e}"))?;
    // server 側の signaling / obsws DataChannel を初回 offer に載せるための
    // m=application 用ダミー DataChannel
    let mut dc_init = DataChannelInit::new();
    dc_init.set_ordered(true);
    let dummy_dc = pc
        .create_data_channel("dummy", &mut dc_init)
        .map_err(|e| format!("failed to create dummy DataChannel: {e}"))?;
    // offer SDP を生成する
    let offer_sdp = create_offer_sdp(&pc)?;
    log_sdp_summary("initial local offer SDP summary", &offer_sdp);
    set_local_description(&pc, SdpType::Offer, &offer_sdp)?;
    let mut initial_ice_candidates = Vec::new();
    let offer_sdp = finalize_local_sdp(offer_sdp, &mut ice_rx, &mut initial_ice_candidates).await?;
    log_sdp_summary("initial local offer with ICE SDP summary", &offer_sdp);

    // /bootstrap で answer SDP を取得する
    let answer_sdp = http_bootstrap(host, port, &offer_sdp).await?;
    log_sdp_summary("bootstrap remote answer SDP summary", &answer_sdp);
    set_remote_description(&pc, SdpType::Answer, &answer_sdp)?;

    // 統計カウンタ
    let video_tracks = Arc::new(AtomicUsize::new(0));
    let audio_tracks = Arc::new(AtomicUsize::new(0));
    let video_frames = Arc::new(AtomicUsize::new(0));
    let audio_frames = Arc::new(AtomicUsize::new(0));
    let video_width = Arc::new(AtomicUsize::new(0));
    let video_height = Arc::new(AtomicUsize::new(0));
    let first_video_frame_logged = Arc::new(AtomicBool::new(false));
    let connection_state = Arc::new(std::sync::Mutex::new("new".to_owned()));

    // フレームデータ受信用チャネル
    let (frame_tx, frame_rx) = std::sync::mpsc::sync_channel::<VideoFrameData>(60);
    let (audio_tx, audio_rx) = std::sync::mpsc::sync_channel::<AudioFrameData>(120);

    let mut retained = RetainedState {
        _pc_observer: pc_observer,
        dummy_dc,
        obsws_dc: None,
        signaling_dc: None,
        signaling_dc_observer: None,
        obsws_dc_observer: None,
        video_sinks: Vec::new(),
        audio_sinks: Vec::new(),
        track_transceivers: Vec::new(),
        ice_rx,
        ice_candidates: initial_ice_candidates,
    };
    let video_sink_attach_state = VideoSinkAttachState {
        video_frames: &video_frames,
        first_video_frame_logged: &first_video_frame_logged,
        video_width: &video_width,
        video_height: &video_height,
        frame_tx: &frame_tx,
    };

    // VP9 エンコーダー（遅延初期化）
    let mut vp9_encoder: Option<shiguredo_libvpx::Encoder> = None;
    let mut vp9_sample_entry: Option<SampleEntry> = None;

    // Opus エンコーダー（遅延初期化）
    let mut opus_encoder: Option<shiguredo_opus::Encoder> = None;
    let mut opus_sample_entry: Option<SampleEntry> = None;
    let mut audio_pcm_buffer: Vec<i16> = Vec::new();
    let mut audio_channels: u8 = 0;

    // MP4 ライター
    let mut mp4_writer = SimpleMp4Writer::new(output_path)?;

    // イベントループ（duration 秒間）
    let deadline = tokio::time::Instant::now() + Duration::from_secs(duration_secs);
    let mut obsws_create_input_sent = false;
    let mut obsws_create_input_succeeded = false;
    let mut obsws_ready = false;
    let mut obsws_subscribe_program_sent = false;
    let mut obsws_subscribe_program_succeeded = false;
    let mut server_webrtc_stats_json = None;
    let mut playout_interval = tokio::time::interval(Duration::from_millis(10));
    playout_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    'event_loop: loop {
        audio_state.render_10ms_audio();

        // フレーム受信チャネルから溜まっているフレームを処理する。
        // 1 回のポーリングで処理するフレーム数を制限して、
        // deadline 判定とイベント処理に必ず戻れるようにする。
        let mut processed_frames = 0;
        while processed_frames < MAX_FRAMES_PER_POLL {
            if tokio::time::Instant::now() >= deadline {
                break 'event_loop;
            }
            let Ok(frame_data) = frame_rx.try_recv() else {
                break;
            };
            encode_and_write_frame(
                &frame_data,
                &mut vp9_encoder,
                &mut vp9_sample_entry,
                &mut mp4_writer,
            )?;
            processed_frames += 1;
        }

        // 音声フレームを処理する
        let mut processed_audio = 0;
        while processed_audio < MAX_FRAMES_PER_POLL {
            if tokio::time::Instant::now() >= deadline {
                break 'event_loop;
            }
            let Ok(audio_data) = audio_rx.try_recv() else {
                break;
            };
            audio_channels = audio_data.channels as u8;
            encode_and_write_audio_frame(
                &audio_data,
                &mut opus_encoder,
                &mut opus_sample_entry,
                &mut audio_pcm_buffer,
                &mut mp4_writer,
            )?;
            processed_audio += 1;
        }

        let connection_ready = connection_state.lock().unwrap().as_str() == "connected";
        let signaling_ready = retained
            .signaling_dc
            .as_ref()
            .is_some_and(|dc| dc.state() == DataChannelState::Open);
        if !obsws_create_input_sent
            && let Some(dc) = &retained.obsws_dc
            && connection_ready
            && signaling_ready
            && obsws_ready
            && dc.state() == DataChannelState::Open
        {
            let request = make_create_mp4_input_request(input_mp4_path);
            tracing::info!(
                "sending CreateInput request: input_mp4_path={input_mp4_path}, duration_secs={duration_secs}"
            );
            if !dc.send(request.as_bytes(), false) {
                return Err("failed to send CreateInput request on obsws DataChannel".to_owned());
            }
            obsws_create_input_sent = true;
        }

        let event = tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Some(e) => e,
                    None => break 'event_loop,
                }
            }
            _ = playout_interval.tick() => continue,
            _ = tokio::time::sleep_until(deadline) => break 'event_loop,
        };

        match event {
            ClientEvent::ConnectionChange(state) => {
                tracing::info!("peer connection state changed: {state:?}");
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
                log_transceiver_receiver_state("onTrack transceiver", &transceiver);
                let receiver = transceiver.receiver();
                let track = receiver.track();
                let kind = track.kind().unwrap_or_default();
                let track_id = track.id().unwrap_or_default();
                match kind.as_str() {
                    "video" => {
                        video_tracks.fetch_add(1, Ordering::Relaxed);
                        tracing::info!("video track received: track_id={track_id}");
                        attach_video_sink(
                            &mut retained,
                            &track_id,
                            track.cast_to_video_track(),
                            &video_sink_attach_state,
                        );
                    }
                    "audio" => {
                        audio_tracks.fetch_add(1, Ordering::Relaxed);
                        tracing::info!("audio track received: track_id={track_id}");
                        attach_audio_sink(
                            &mut retained,
                            &track_id,
                            track.cast_to_audio_track(),
                            &audio_frames,
                            &audio_tx,
                        );
                    }
                    _ => {
                        tracing::warn!("unknown track kind: {kind}");
                    }
                }
                // transceiver を保持しないと、ラッパーの寿命次第で受信が不安定になる可能性がある。
                retained.track_transceivers.push(transceiver);
            }
            ClientEvent::DataChannel(dc, observer) => {
                let label = dc.label().unwrap_or_default();
                tracing::info!(
                    "data channel received: label={label}, state={:?}",
                    dc.state()
                );
                if label == "signaling" {
                    retained.signaling_dc = Some(dc);
                    retained.signaling_dc_observer = observer;
                } else if label == "obsws" {
                    obsws_ready = dc.state() == DataChannelState::Open;
                    retained.obsws_dc = Some(dc);
                    retained.obsws_dc_observer = observer;
                }
            }
            ClientEvent::SignalingMessage { data } => {
                let msg_type = parse_signaling_type(&data).unwrap_or_default();
                if msg_type == "offer" {
                    tracing::info!("renegotiation offer received from signaling data channel");
                    // renegotiation: サーバーからの offer に answer を返す
                    if let Some(sdp) = parse_signaling_sdp(&data) {
                        log_sdp_summary("renegotiation remote offer SDP summary", &sdp);
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
                                    &mut retained.ice_rx,
                                    &mut retained.ice_candidates,
                                )
                                .await
                                {
                                    Ok(answer) => answer,
                                    Err(e) => {
                                        tracing::warn!("failed to gather ICE candidates: {e}");
                                        continue;
                                    }
                                };
                                log_sdp_summary("renegotiation local answer SDP summary", &answer);
                                let answer_json = make_answer_json(&answer);
                                if let Some(dc) = &retained.signaling_dc {
                                    tracing::info!(
                                        "sending renegotiation answer on signaling data channel"
                                    );
                                    dc.send(answer_json.as_bytes(), false);
                                }
                            }
                            Err(e) => {
                                tracing::warn!("failed to create answer: {e}");
                            }
                        }
                    }
                }
            }
            ClientEvent::ObswsMessage { data } => {
                if let Ok(text) = std::str::from_utf8(&data) {
                    if let Some(result) = parse_obsws_request_response(text) {
                        match result {
                            Ok(()) => {
                                obsws_create_input_succeeded = true;
                                // CreateInput 成功後に SubscribeProgramTracks を送信する
                                if subscribe_program_tracks
                                    && !obsws_subscribe_program_sent
                                    && let Some(dc) = &retained.obsws_dc
                                    && dc.state() == DataChannelState::Open
                                {
                                    let request = make_subscribe_program_tracks_request();
                                    tracing::info!("sending SubscribeProgramTracks request");
                                    if !dc.send(request.as_bytes(), false) {
                                        return Err(
                                            "failed to send SubscribeProgramTracks request"
                                                .to_owned(),
                                        );
                                    }
                                    obsws_subscribe_program_sent = true;
                                }
                            }
                            Err(reason) => {
                                return Err(format!("CreateInput request failed: {reason}"));
                            }
                        }
                    }
                    if let Some(result) = parse_subscribe_program_tracks_response(text) {
                        match result {
                            Ok(()) => {
                                obsws_subscribe_program_succeeded = true;
                                tracing::info!("SubscribeProgramTracks succeeded");
                            }
                            Err(reason) => {
                                return Err(format!(
                                    "SubscribeProgramTracks request failed: {reason}"
                                ));
                            }
                        }
                    }
                } else {
                    tracing::debug!("obsws message: <binary {} bytes>", data.len());
                }
            }
            ClientEvent::ObswsDataChannelStateChange => {
                if let Some(dc) = &retained.obsws_dc {
                    obsws_ready = dc.state() == DataChannelState::Open;
                }
            }
        }
    }

    if video_tracks.load(Ordering::Relaxed) > 0 && video_frames.load(Ordering::Relaxed) == 0 {
        tracing::warn!(
            "video track was received but no video frames arrived before deadline; waiting additional {:?}",
            INITIAL_VIDEO_FRAME_GRACE
        );
        let grace_deadline = tokio::time::Instant::now() + INITIAL_VIDEO_FRAME_GRACE;
        while tokio::time::Instant::now() < grace_deadline {
            while let Ok(frame_data) = frame_rx.try_recv() {
                encode_and_write_frame(
                    &frame_data,
                    &mut vp9_encoder,
                    &mut vp9_sample_entry,
                    &mut mp4_writer,
                )?;
            }
            if video_frames.load(Ordering::Relaxed) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    if video_tracks.load(Ordering::Relaxed) > 0 && video_frames.load(Ordering::Relaxed) == 0 {
        match request_server_webrtc_stats(&retained, &mut event_rx).await {
            Ok(stats_json) => {
                tracing::warn!(
                    "server-side libwebrtc stats summary: {}",
                    summarize_webrtc_stats_json(&stats_json)
                );
                tracing::warn!("server-side libwebrtc stats raw: {stats_json}");
                server_webrtc_stats_json = Some(stats_json);
            }
            Err(e) => {
                tracing::warn!("failed to fetch server-side libwebrtc stats: {e}");
            }
        }
    }

    let webrtc_stats_json = collect_webrtc_stats_json(&pc).await;
    let webrtc_stats_error = match &webrtc_stats_json {
        Ok(_) => String::new(),
        Err(e) => e.clone(),
    };
    if video_frames.load(Ordering::Relaxed) == 0
        && let Ok(stats_json) = &webrtc_stats_json
    {
        tracing::warn!(
            "libwebrtc stats summary: {}",
            summarize_webrtc_stats_json(stats_json)
        );
        tracing::warn!("libwebrtc stats raw: {stats_json}");
    }
    if video_frames.load(Ordering::Relaxed) == 0
        && let Some(stats_json) = &server_webrtc_stats_json
    {
        tracing::debug!("server-side libwebrtc stats length={}", stats_json.len());
    }
    if !webrtc_stats_error.is_empty() {
        tracing::warn!("failed to collect libwebrtc stats: {}", webrtc_stats_error);
    }

    teardown_client(&pc, &mut retained, &audio_state).await;

    // 残りのフレームを処理する
    let drain_deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while let Ok(frame_data) = frame_rx.try_recv() {
        encode_and_write_frame(
            &frame_data,
            &mut vp9_encoder,
            &mut vp9_sample_entry,
            &mut mp4_writer,
        )?;
        if tokio::time::Instant::now() >= drain_deadline {
            break;
        }
    }
    // 残りの音声フレームを処理する
    while let Ok(audio_data) = audio_rx.try_recv() {
        audio_channels = audio_data.channels as u8;
        encode_and_write_audio_frame(
            &audio_data,
            &mut opus_encoder,
            &mut opus_sample_entry,
            &mut audio_pcm_buffer,
            &mut mp4_writer,
        )?;
        if tokio::time::Instant::now() >= drain_deadline {
            break;
        }
    }

    // エンコーダーの残りフレームをフラッシュする
    if let Some(encoder) = &mut vp9_encoder {
        encoder
            .finish()
            .map_err(|e| format!("failed to finish encoder: {e}"))?;
        while let Some(frame) = encoder.next_frame() {
            let se = vp9_sample_entry.take();
            mp4_writer.append_video(frame.data(), frame.is_keyframe(), se, 0)?;
        }
    }

    // バッファに残った PCM データが 1 フレーム分以上あればエンコードする
    if let Some(encoder) = &mut opus_encoder {
        let frame_samples = encoder.frame_samples();
        let total_per_frame = frame_samples * audio_channels as usize;
        if total_per_frame > 0 {
            while audio_pcm_buffer.len() >= total_per_frame {
                let pcm: Vec<i16> = audio_pcm_buffer.drain(..total_per_frame).collect();
                let opus_data = encoder
                    .encode(&pcm)
                    .map_err(|e| format!("Opus encode failed: {e}"))?;
                let sample_rate = encoder
                    .get_sample_rate()
                    .map_err(|e| format!("failed to get sample rate: {e}"))?;
                let duration_us = (frame_samples as u64 * 1_000_000 / sample_rate as u64) as u32;
                let se = opus_sample_entry.take();
                mp4_writer.append_audio(&opus_data, se, duration_us)?;
            }
        }
    }

    // MP4 ファイルをファイナライズする
    if mp4_writer.video_sample_count > 0 || mp4_writer.audio_sample_count > 0 {
        mp4_writer.finalize()?;
    }

    let video_codec = if mp4_writer.video_sample_count > 0 {
        "vp9".to_owned()
    } else {
        "none".to_owned()
    };
    let audio_codec = if mp4_writer.audio_sample_count > 0 {
        "opus".to_owned()
    } else {
        "none".to_owned()
    };

    if !obsws_create_input_succeeded {
        tracing::warn!("CreateInput request did not complete before deadline");
        return Err("CreateInput request did not complete".to_owned());
    }
    if subscribe_program_tracks && !obsws_subscribe_program_succeeded {
        tracing::warn!("SubscribeProgramTracks request did not complete before deadline");
        return Err("SubscribeProgramTracks request did not complete".to_owned());
    }
    let final_connection_state = connection_state
        .lock()
        .expect("connection_state mutex should not be poisoned")
        .clone();
    tracing::info!(
        "bootstrap finished: video_tracks={}, video_frames={}, audio_tracks={}, audio_frames={}, video_width={}, video_height={}, video_samples_written={}, audio_samples_written={}, connection_state={}, webrtc_stats_error={}, program_tracks_subscribed={}",
        video_tracks.load(Ordering::Relaxed),
        video_frames.load(Ordering::Relaxed),
        audio_tracks.load(Ordering::Relaxed),
        audio_frames.load(Ordering::Relaxed),
        video_width.load(Ordering::Relaxed),
        video_height.load(Ordering::Relaxed),
        mp4_writer.video_sample_count,
        mp4_writer.audio_sample_count,
        final_connection_state,
        webrtc_stats_error.as_str(),
        obsws_subscribe_program_succeeded,
    );
    Ok(Stats {
        video_tracks: video_tracks.load(Ordering::Relaxed),
        audio_tracks: audio_tracks.load(Ordering::Relaxed),
        video_frames: video_frames.load(Ordering::Relaxed),
        audio_frames: audio_frames.load(Ordering::Relaxed),
        video_width: video_width.load(Ordering::Relaxed),
        video_height: video_height.load(Ordering::Relaxed),
        video_codec,
        audio_codec,
        video_samples_written: mp4_writer.video_sample_count,
        audio_samples_written: mp4_writer.audio_sample_count,
        connection_state: connection_state.lock().unwrap().clone(),
        webrtc_stats_error,
        program_tracks_subscribed: obsws_subscribe_program_succeeded,
    })
}

/// フレームを VP9 エンコードして MP4 に書き込む
fn encode_and_write_frame(
    frame_data: &VideoFrameData,
    vp9_encoder: &mut Option<shiguredo_libvpx::Encoder>,
    vp9_sample_entry: &mut Option<SampleEntry>,
    mp4_writer: &mut SimpleMp4Writer,
) -> Result<(), String> {
    let width = frame_data.width as usize;
    let height = frame_data.height as usize;

    // エンコーダーの遅延初期化
    if vp9_encoder.is_none() {
        let config = shiguredo_libvpx::EncoderConfig::new(
            width,
            height,
            shiguredo_libvpx::ImageFormat::I420,
            shiguredo_libvpx::CodecConfig::Vp9(Default::default()),
        );
        let encoder = shiguredo_libvpx::Encoder::new(config)
            .map_err(|e| format!("failed to create VP9 encoder: {e}"))?;
        *vp9_encoder = Some(encoder);
        *vp9_sample_entry = Some(vp9_sample_entry_value(width, height));
    }

    let encoder = vp9_encoder.as_mut().unwrap();
    let compact_y = compact_i420_plane(&frame_data.y, width, height, frame_data.stride_y)?;
    let uv_width = width.div_ceil(2);
    let uv_height = height.div_ceil(2);
    let compact_u = compact_i420_plane(&frame_data.u, uv_width, uv_height, frame_data.stride_u)?;
    let compact_v = compact_i420_plane(&frame_data.v, uv_width, uv_height, frame_data.stride_v)?;

    let encode_options = shiguredo_libvpx::EncodeOptions {
        force_keyframe: false,
    };
    encoder
        .encode(
            &shiguredo_libvpx::ImageData::I420 {
                y: &compact_y,
                u: &compact_u,
                v: &compact_v,
            },
            &encode_options,
        )
        .map_err(|e| format!("VP9 encode failed: {e}"))?;

    while let Some(frame) = encoder.next_frame() {
        let se = vp9_sample_entry.take();
        mp4_writer.append_video(
            frame.data(),
            frame.is_keyframe(),
            se,
            frame_data.timestamp_us,
        )?;
    }
    Ok(())
}

fn compact_i420_plane(
    plane: &[u8],
    width: usize,
    height: usize,
    stride: usize,
) -> Result<Vec<u8>, String> {
    if stride < width {
        return Err(format!(
            "invalid I420 stride: stride={stride}, width={width}, height={height}"
        ));
    }
    let required_len = stride
        .checked_mul(height)
        .ok_or_else(|| format!("I420 plane size overflow: stride={stride}, height={height}"))?;
    if plane.len() < required_len {
        return Err(format!(
            "insufficient I420 plane data: expected at least {required_len} bytes, got {}",
            plane.len()
        ));
    }
    if stride == width {
        return Ok(plane[..width * height].to_vec());
    }

    let mut compact = Vec::with_capacity(width * height);
    for row in 0..height {
        let offset = row * stride;
        compact.extend_from_slice(&plane[offset..offset + width]);
    }
    Ok(compact)
}

/// VP9 SampleEntry の値を返す（エンコーダー初期化時に呼ぶ）
fn vp9_sample_entry_value(width: usize, height: usize) -> SampleEntry {
    vp9_sample_entry(width, height)
}

/// 受信した PCM 音声データを Opus エンコードして MP4 に書き込む
fn encode_and_write_audio_frame(
    frame_data: &AudioFrameData,
    opus_encoder: &mut Option<shiguredo_opus::Encoder>,
    opus_sample_entry: &mut Option<SampleEntry>,
    audio_pcm_buffer: &mut Vec<i16>,
    mp4_writer: &mut SimpleMp4Writer,
) -> Result<(), String> {
    let sample_rate = frame_data.sample_rate as u32;
    let channels = frame_data.channels as u8;

    // エンコーダーの遅延初期化
    if opus_encoder.is_none() {
        let mut config = shiguredo_opus::EncoderConfig::new(sample_rate, channels);
        config.frame_duration = Some(shiguredo_opus::FrameDuration::Ms10);
        let encoder = shiguredo_opus::Encoder::new(config)
            .map_err(|e| format!("failed to create Opus encoder: {e}"))?;
        let pre_skip = encoder
            .get_lookahead()
            .map_err(|e| format!("failed to get Opus lookahead: {e}"))?;
        *opus_sample_entry = Some(opus_sample_entry_value(channels, pre_skip));
        *opus_encoder = Some(encoder);
    }

    let encoder = opus_encoder.as_mut().unwrap();
    let frame_samples = encoder.frame_samples();
    let total_per_frame = frame_samples * channels as usize;

    audio_pcm_buffer.extend_from_slice(&frame_data.pcm);

    while audio_pcm_buffer.len() >= total_per_frame {
        let pcm: Vec<i16> = audio_pcm_buffer.drain(..total_per_frame).collect();
        let opus_data = encoder
            .encode(&pcm)
            .map_err(|e| format!("Opus encode failed: {e}"))?;
        let duration_us = (frame_samples as u64 * 1_000_000 / sample_rate as u64) as u32;
        let se = opus_sample_entry.take();
        mp4_writer.append_audio(&opus_data, se, duration_us)?;
    }
    Ok(())
}
