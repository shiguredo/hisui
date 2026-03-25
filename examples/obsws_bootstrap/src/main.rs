use std::io::{Seek, SeekFrom, Write};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use shiguredo_http11::{Request, ResponseDecoder};
use shiguredo_mp4::Uint;
use shiguredo_mp4::boxes::{SampleEntry, VisualSampleEntryFields, Vp09Box, VpccBox};
use shiguredo_mp4::mux::{Mp4FileMuxer, Mp4FileMuxerOptions, Sample};
use shiguredo_webrtc::{
    AudioDecoderFactory, AudioDeviceModule, AudioDeviceModuleAudioLayer, AudioEncoderFactory,
    AudioProcessingBuilder, CreateSessionDescriptionObserver,
    CreateSessionDescriptionObserverHandler, DataChannel, DataChannelInit, DataChannelObserver,
    DataChannelObserverHandler, DataChannelState, Environment, IceGatheringState, PeerConnection,
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
const CREATE_INPUT_REQUEST_ID: &str = "req-create-input";

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
    stride_y: i32,
    stride_u: i32,
    stride_v: i32,
    width: i32,
    height: i32,
    timestamp_us: i64,
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
    width: Arc<AtomicUsize>,
    height: Arc<AtomicUsize>,
    frame_tx: std::sync::mpsc::SyncSender<VideoFrameData>,
}

impl VideoSinkHandler for FrameRecordHandler {
    fn on_frame(&mut self, frame: shiguredo_webrtc::VideoFrameRef<'_>) {
        self.frame_count.fetch_add(1, Ordering::Relaxed);
        let w = frame.width();
        let h = frame.height();
        self.width.store(w as usize, Ordering::Relaxed);
        self.height.store(h as usize, Ordering::Relaxed);

        // I420 バッファからプレーンデータをコピーする
        let buffer = frame.buffer();
        let data = VideoFrameData {
            y: buffer.y_data().to_vec(),
            u: buffer.u_data().to_vec(),
            v: buffer.v_data().to_vec(),
            stride_y: buffer.stride_y(),
            stride_u: buffer.stride_u(),
            stride_v: buffer.stride_v(),
            width: w,
            height: h,
            timestamp_us: frame.timestamp_us(),
        };
        // バッファがいっぱいの場合はフレームを捨てる
        let _ = self.frame_tx.try_send(data);
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
    tracing::info!("bootstrap offer sent: sdp_bytes={}", offer_sdp.len());

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
            tracing::info!(
                "bootstrap response received: status={} {}",
                response.status_code,
                response.reason_phrase
            );
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

// --- MP4 ライター ---

struct SimpleMp4Writer {
    file: std::io::BufWriter<std::fs::File>,
    muxer: Mp4FileMuxer,
    next_position: u64,
    video_sample_entry: Option<SampleEntry>,
    video_sample_count: usize,
    last_video_timestamp_us: Option<i64>,
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
            ))
            .await
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
                f.member("video_codec", stats.video_codec.as_str())?;
                f.member("video_samples_written", stats.video_samples_written)?;
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
    video_codec: String,
    video_samples_written: usize,
    connection_state: String,
}

#[derive(Clone)]
struct GatheredIceCandidate {
    sdp_mid: String,
    sdp_mline_index: i32,
    candidate: String,
}

struct RetainedState {
    _pc_observer: PeerConnectionObserver,
    _dummy_dc: DataChannel,
    obsws_dc: Option<DataChannel>,
    signaling_dc: Option<DataChannel>,
    signaling_dc_observer: Option<DataChannelObserver>,
    obsws_dc_observer: Option<DataChannelObserver>,
    video_sinks: Vec<VideoSink>,
    ice_rx: mpsc::UnboundedReceiver<IceObserverEvent>,
    ice_candidates: Vec<GatheredIceCandidate>,
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
) -> Result<Stats, String> {
    tracing::info!(
        "run_client started: host={host}, port={port}, duration_secs={duration_secs}, output_path={output_path}, input_mp4_path={input_mp4_path}"
    );
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
    let (ice_tx, mut ice_rx) = mpsc::unbounded_channel::<IceObserverEvent>();
    let pc_observer = PeerConnectionObserver::new_with_handler(Box::new(ClientPcObserver {
        event_tx: event_tx.clone(),
        ice_tx,
    }));
    let mut pc_deps = PeerConnectionDependencies::new(&pc_observer);
    let mut config = PeerConnectionRtcConfiguration::new();

    let pc = PeerConnection::create(factory.as_ref(), &mut config, &mut pc_deps)
        .map_err(|e| format!("failed to create PeerConnection: {e}"))?;
    tracing::info!("peer connection created");

    // server 側の signaling / obsws DataChannel を初回 offer に載せるための
    // m=application 用ダミー DataChannel
    let mut dc_init = DataChannelInit::new();
    dc_init.set_ordered(true);
    let dummy_dc = pc
        .create_data_channel("dummy", &mut dc_init)
        .map_err(|e| format!("failed to create dummy DataChannel: {e}"))?;
    tracing::info!("dummy data channel created");

    // offer SDP を生成する
    let offer_sdp = create_offer_sdp(&pc)?;
    tracing::info!("initial offer created");
    set_local_description(&pc, SdpType::Offer, &offer_sdp)?;
    tracing::info!("initial local description set");
    let mut initial_ice_candidates = Vec::new();
    let offer_sdp = finalize_local_sdp(offer_sdp, &mut ice_rx, &mut initial_ice_candidates).await?;
    tracing::info!(
        "initial offer finalized with ICE candidates: sdp_bytes={}, ice_candidates={}",
        offer_sdp.len(),
        initial_ice_candidates.len()
    );

    // /bootstrap で answer SDP を取得する
    let answer_sdp = http_bootstrap(host, port, &offer_sdp).await?;
    tracing::info!("bootstrap answer received: sdp_bytes={}", answer_sdp.len());
    set_remote_description(&pc, SdpType::Answer, &answer_sdp)?;
    tracing::info!("bootstrap completed");

    // 統計カウンタ
    let video_tracks = Arc::new(AtomicUsize::new(0));
    let audio_tracks = Arc::new(AtomicUsize::new(0));
    let video_frames = Arc::new(AtomicUsize::new(0));
    let video_width = Arc::new(AtomicUsize::new(0));
    let video_height = Arc::new(AtomicUsize::new(0));
    let connection_state = Arc::new(std::sync::Mutex::new("new".to_owned()));

    // フレームデータ受信用チャネル
    let (frame_tx, frame_rx) = std::sync::mpsc::sync_channel::<VideoFrameData>(60);

    let mut retained = RetainedState {
        _pc_observer: pc_observer,
        _dummy_dc: dummy_dc,
        obsws_dc: None,
        signaling_dc: None,
        signaling_dc_observer: None,
        obsws_dc_observer: None,
        video_sinks: Vec::new(),
        ice_rx,
        ice_candidates: initial_ice_candidates,
    };

    // VP9 エンコーダー（遅延初期化）
    let mut vp9_encoder: Option<shiguredo_libvpx::Encoder> = None;
    let mut vp9_sample_entry: Option<SampleEntry> = None;

    // MP4 ライター
    let mut mp4_writer = SimpleMp4Writer::new(output_path)?;

    // イベントループ（duration 秒間）
    let deadline = tokio::time::Instant::now() + Duration::from_secs(duration_secs);
    let mut obsws_create_input_sent = false;
    let mut obsws_create_input_succeeded = false;
    let mut obsws_ready = false;
    let mut first_video_frame_logged = false;
    loop {
        // フレーム受信チャネルから溜まっているフレームを処理する
        while let Ok(frame_data) = frame_rx.try_recv() {
            if !first_video_frame_logged {
                tracing::info!(
                    "first video frame received: width={}, height={}, timestamp_us={}",
                    frame_data.width,
                    frame_data.height,
                    frame_data.timestamp_us
                );
                first_video_frame_logged = true;
            }
            encode_and_write_frame(
                &frame_data,
                &mut vp9_encoder,
                &mut vp9_sample_entry,
                &mut mp4_writer,
            )?;
        }

        if !obsws_create_input_sent
            && let Some(dc) = &retained.obsws_dc
            && obsws_ready
            && dc.state() == DataChannelState::Open
        {
            tracing::info!("sending CreateInput request on obsws DataChannel");
            let request = make_create_mp4_input_request(input_mp4_path);
            if !dc.send(request.as_bytes(), false) {
                return Err("failed to send CreateInput request on obsws DataChannel".to_owned());
            }
            obsws_create_input_sent = true;
            tracing::info!("CreateInput request sent on obsws DataChannel");
        }

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
                        let mut video_track = track.cast_to_video_track();
                        let sink = VideoSink::new_with_handler(Box::new(FrameRecordHandler {
                            frame_count: video_frames.clone(),
                            width: video_width.clone(),
                            height: video_height.clone(),
                            frame_tx: frame_tx.clone(),
                        }));
                        let wants = VideoSinkWants::default();
                        video_track.add_or_update_sink(&sink, &wants);
                        retained.video_sinks.push(sink);
                    }
                    "audio" => {
                        // 現在の shiguredo_webrtc 0.146.0 には remote audio track
                        // から PCM を取り出すための公開 API が見当たらないため、
                        // 音声は受信数の記録のみに留める。音声を MP4 に書くには、
                        // 別途 audio sink 相当の API が必要になる。
                        audio_tracks.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {
                        tracing::warn!("unknown track kind: {kind}");
                    }
                }
            }
            ClientEvent::DataChannel(dc, observer) => {
                let label = dc.label().unwrap_or_default();
                tracing::info!("data channel received: label={label}");
                if label == "signaling" {
                    retained.signaling_dc = Some(dc);
                    retained.signaling_dc_observer = observer;
                } else if label == "obsws" {
                    obsws_ready = dc.state() == DataChannelState::Open;
                    tracing::info!("obsws data channel ready={obsws_ready}");
                    retained.obsws_dc = Some(dc);
                    retained.obsws_dc_observer = observer;
                }
            }
            ClientEvent::SignalingMessage { data } => {
                let msg_type = parse_signaling_type(&data).unwrap_or_default();
                tracing::debug!("signaling message: type={msg_type}");
                if msg_type == "offer" {
                    tracing::info!("handling renegotiation offer");
                    // renegotiation: サーバーからの offer に answer を返す
                    if let Some(sdp) = parse_signaling_sdp(&data) {
                        tracing::info!("renegotiation offer parsed: sdp_bytes={}", sdp.len());
                        if let Err(e) = set_remote_description(&pc, SdpType::Offer, &sdp) {
                            tracing::warn!("failed to set remote offer: {e}");
                            continue;
                        }
                        tracing::info!("remote renegotiation offer applied");
                        match create_answer_sdp(&pc) {
                            Ok(answer) => {
                                tracing::info!("renegotiation answer created");
                                if let Err(e) = set_local_description(&pc, SdpType::Answer, &answer)
                                {
                                    tracing::warn!("failed to set local answer: {e}");
                                    continue;
                                }
                                tracing::info!("local renegotiation answer applied");
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
            ClientEvent::ObswsMessage { data } => {
                if let Ok(text) = std::str::from_utf8(&data) {
                    tracing::debug!("obsws message: {text}");
                    if let Some(result) = parse_obsws_request_response(text) {
                        match result {
                            Ok(()) => {
                                obsws_create_input_succeeded = true;
                                tracing::info!("CreateInput request succeeded");
                            }
                            Err(reason) => {
                                return Err(format!("CreateInput request failed: {reason}"));
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
                    tracing::info!("obsws data channel ready={obsws_ready}");
                }
            }
        }
    }

    // 残りのフレームを処理する
    while let Ok(frame_data) = frame_rx.try_recv() {
        if !first_video_frame_logged {
            tracing::info!(
                "first video frame received: width={}, height={}, timestamp_us={}",
                frame_data.width,
                frame_data.height,
                frame_data.timestamp_us
            );
            first_video_frame_logged = true;
        }
        encode_and_write_frame(
            &frame_data,
            &mut vp9_encoder,
            &mut vp9_sample_entry,
            &mut mp4_writer,
        )?;
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

    // MP4 ファイルをファイナライズする
    if mp4_writer.video_sample_count > 0 {
        mp4_writer.finalize()?;
    }

    let video_codec = if mp4_writer.video_sample_count > 0 {
        "vp9".to_owned()
    } else {
        "none".to_owned()
    };

    if !obsws_create_input_succeeded {
        tracing::warn!("CreateInput request did not complete before deadline");
        return Err("CreateInput request did not complete".to_owned());
    }

    Ok(Stats {
        video_tracks: video_tracks.load(Ordering::Relaxed),
        audio_tracks: audio_tracks.load(Ordering::Relaxed),
        video_frames: video_frames.load(Ordering::Relaxed),
        video_width: video_width.load(Ordering::Relaxed),
        video_height: video_height.load(Ordering::Relaxed),
        video_codec,
        video_samples_written: mp4_writer.video_sample_count,
        connection_state: connection_state.lock().unwrap().clone(),
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

    // I420 プレーンデータからストライドを考慮して正しいサイズを構築する
    let y_plane = build_plane_data(&frame_data.y, frame_data.stride_y, width, height);
    let u_plane = build_plane_data(
        &frame_data.u,
        frame_data.stride_u,
        width.div_ceil(2),
        height.div_ceil(2),
    );
    let v_plane = build_plane_data(
        &frame_data.v,
        frame_data.stride_v,
        width.div_ceil(2),
        height.div_ceil(2),
    );

    let encode_options = shiguredo_libvpx::EncodeOptions {
        force_keyframe: false,
    };
    encoder
        .encode(
            &shiguredo_libvpx::ImageData::I420 {
                y: &y_plane,
                u: &u_plane,
                v: &v_plane,
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

/// ストライドを考慮して plane データを width * height のバイト列に変換する
fn build_plane_data(data: &[u8], stride: i32, width: usize, height: usize) -> Vec<u8> {
    let stride = stride as usize;
    if stride == width {
        // ストライドと幅が一致する場合はそのまま返す
        data[..width * height].to_vec()
    } else {
        // 行ごとにコピーする
        let mut result = Vec::with_capacity(width * height);
        for row in 0..height {
            let start = row * stride;
            let end = start + width;
            if end <= data.len() {
                result.extend_from_slice(&data[start..end]);
            }
        }
        result
    }
}

/// VP9 SampleEntry の値を返す（エンコーダー初期化時に呼ぶ）
fn vp9_sample_entry_value(width: usize, height: usize) -> SampleEntry {
    vp9_sample_entry(width, height)
}
