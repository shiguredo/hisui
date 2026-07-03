use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use base64ct::{Base64, Encoding as _};
use shiguredo_http11::{
    auth::{BasicAuth, DigestChallenge},
    uri::Uri,
};
use shiguredo_mp4::boxes::SampleEntry;
use shiguredo_rtsp::{
    DigestCredentials, RtspClientConnection, RtspConnectionEvent, RtspMethod, RtspRequest,
    RtspResponse, RtspTransport, Sdp, sdp::SdpAttribute,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{
    Error, ProcessorHandle, TrackId, TrackPublisher,
    audio::{
        AudioFormat, AudioFrame, Channels, SampleRate,
        aac::{AacRtpDepacketizer, validate_aac_fmtp_lengths},
    },
    sample_entry::SharedSampleEntry,
    timestamp::mapper::TimestampMapper,
    video::{VideoFormat, VideoFrame},
};

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(20);
const RECONNECT_DELAY_INITIAL: Duration = Duration::from_millis(500);
const RECONNECT_DELAY_MAX: Duration = Duration::from_secs(5);
const DEFAULT_RTSP_PORT: u16 = 554;

/// RTSP Subscriber
///
/// フィールドの不変条件は `Self::new()` で eager 検証される。
#[derive(Debug, Clone)]
pub struct RtspSubscriber {
    pub(crate) input_url: String,
    pub(crate) output_audio_track_id: Option<TrackId>,
    pub(crate) output_video_track_id: Option<TrackId>,
}

/// `RtspSubscriber::new()` が返す検証エラー。
#[derive(Debug)]
pub enum RtspSubscriberBuildError {
    EmptyInputUrl,
    NoTrackId,
}

impl std::fmt::Display for RtspSubscriberBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyInputUrl => write!(f, "input_url must not be empty"),
            Self::NoTrackId => write!(
                f,
                "at least one of output_audio_track_id / output_video_track_id must be set"
            ),
        }
    }
}

impl RtspSubscriber {
    /// `RtspSubscriber` を構築する。
    pub fn new(
        input_url: String,
        output_audio_track_id: Option<TrackId>,
        output_video_track_id: Option<TrackId>,
    ) -> Result<Self, RtspSubscriberBuildError> {
        if input_url.is_empty() {
            return Err(RtspSubscriberBuildError::EmptyInputUrl);
        }
        if output_audio_track_id.is_none() && output_video_track_id.is_none() {
            return Err(RtspSubscriberBuildError::NoTrackId);
        }
        Ok(Self {
            input_url,
            output_audio_track_id,
            output_video_track_id,
        })
    }
}

impl RtspSubscriber {
    pub async fn run(self, handle: ProcessorHandle) -> crate::Result<()> {
        let parsed_url = parse_rtsp_input_url(&self.input_url)
            .map_err(|e| Error::new(format!("invalid input_url: {e}")))?;
        let want_audio = self.output_audio_track_id.is_some();
        let want_video = self.output_video_track_id.is_some();

        let mut audio_track_tx = if let Some(track_id) = &self.output_audio_track_id {
            Some(handle.publish_track(track_id.clone()).await?)
        } else {
            None
        };

        let stats = RtspSubscriberStats::new(handle.stats());
        stats.set_connected(false);

        // video decoder task を endpoint 寿命で保持する。 publish_track で得た TrackPublisher を
        // task 内に move し、 session ループには input_tx.clone() を借用経由で渡す。
        // Ok / Retryable 経路では task を継続保持し、 Fatal 経路のみ shutdown().await する。
        let video_decoder_task = if let Some(track_id) = &self.output_video_track_id {
            let output_tx = handle.publish_track(track_id.clone()).await?;
            let options = crate::decoder::VideoDecoderOptions {
                openh264_lib: handle.config().openh264_lib.clone(),
                ..Default::default()
            };
            Some(spawn_video_decoder_task(options, handle.stats(), output_tx))
        } else {
            None
        };
        let mut video_decoder_input_tx = video_decoder_task
            .as_ref()
            .map(|task| task.input_tx.clone());

        let mut audio_decoder = if want_audio {
            let mut decoder_stats = handle.stats();
            decoder_stats.set_default_label("component", "audio_decoder");
            Some(crate::decoder::AudioDecoder::new(
                #[cfg(feature = "fdk-aac")]
                handle.config().fdk_aac_lib.clone(),
                decoder_stats,
            )?)
        } else {
            None
        };

        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        let started_at = Instant::now();
        let mut reconnect_backoff = ReconnectBackoff::new();

        loop {
            let connection_offset = started_at.elapsed();
            let mut output = RtspOutputContext {
                audio_track_tx: &mut audio_track_tx,
                audio_decoder: &mut audio_decoder,
                video_decoder_input_tx: &mut video_decoder_input_tx,
            };
            let session_result = run_rtsp_session(
                &parsed_url,
                want_audio,
                want_video,
                connection_offset,
                &stats,
                &mut output,
            )
            .await;

            match session_result {
                Ok(()) => {
                    stats.set_connected(false);
                    reconnect_backoff.reset();
                    tracing::warn!("RTSP session closed; reconnecting");
                }
                Err(SessionError::Fatal(e)) => {
                    // Fatal は endpoint 停止 = pipeline 停止のため、 decoder task を shutdown して
                    // 下流の永久 hang を防ぐ。 shutdown の Err は warn で握り潰して Fatal を優先する。
                    if let Some(task) = video_decoder_task
                        && let Err(shutdown_err) = task.shutdown().await
                    {
                        tracing::warn!(
                            "video decoder task shutdown failed: {}",
                            shutdown_err.display()
                        );
                    }
                    return Err(e);
                }
                Err(SessionError::Retryable(e)) => {
                    stats.set_connected(false);
                    tracing::warn!("RTSP session disconnected: {}", e.display());
                }
            }

            let delay = reconnect_backoff.next_delay();
            tokio::time::sleep(delay).await;
        }
    }
}

#[derive(Debug, Clone)]
struct RtspSubscriberStats {
    is_connected_metric: crate::stats::StatsFlag,
    audio_codec_metric: crate::stats::StatsString,
    total_input_audio_data_count_metric: crate::stats::StatsCounter,
    last_input_audio_timestamp_metric: crate::stats::StatsDuration,
    video_codec_metric: crate::stats::StatsString,
    total_input_video_frame_count_metric: crate::stats::StatsCounter,
    last_input_video_timestamp_metric: crate::stats::StatsDuration,
}

impl RtspSubscriberStats {
    fn new(mut stats: crate::stats::Stats) -> Self {
        Self {
            is_connected_metric: stats.flag("is_connected"),
            audio_codec_metric: stats.string("audio_codec"),
            total_input_audio_data_count_metric: stats.counter("total_input_audio_data_count"),
            last_input_audio_timestamp_metric: stats.duration("last_input_audio_timestamp"),
            video_codec_metric: stats.string("video_codec"),
            total_input_video_frame_count_metric: stats.counter("total_input_video_frame_count"),
            last_input_video_timestamp_metric: stats.duration("last_input_video_timestamp"),
        }
    }

    fn set_connected(&self, value: bool) {
        self.is_connected_metric.set(value);
    }

    fn set_audio_codec(&self, codec: crate::types::CodecName) {
        self.audio_codec_metric.set(codec.as_str());
    }

    fn add_input_audio_data_count(&self) {
        self.total_input_audio_data_count_metric.inc();
    }

    fn set_last_input_audio_timestamp(&self, timestamp: Duration) {
        self.last_input_audio_timestamp_metric.set(timestamp);
    }

    fn set_video_codec(&self, codec: crate::types::CodecName) {
        self.video_codec_metric.set(codec.as_str());
    }

    fn add_input_video_frame_count(&self) {
        self.total_input_video_frame_count_metric.inc();
    }

    fn set_last_input_video_timestamp(&self, timestamp: Duration) {
        self.last_input_video_timestamp_metric.set(timestamp);
    }
}

#[derive(Debug, Clone)]
struct ParsedRtspUrl {
    host: String,
    port: u16,
    tls: bool,
    request_url: String,
    credentials: Option<RtspCredentials>,
}

#[derive(Debug, Clone)]
struct RtspCredentials {
    username: String,
    password: String,
}

#[derive(Debug)]
enum SessionError {
    Fatal(Error),
    Retryable(Error),
}

#[derive(Debug, Clone)]
struct VideoTrackConfig {
    control_url: String,
    payload_type: u8,
    clock_rate: u32,
    /// SDP `sprop-parameter-sets` から構築したサンプルエントリー（不在時は `None`）。
    sample_entry: Option<SampleEntry>,
}

#[derive(Debug, Clone)]
struct AudioTrackConfig {
    control_url: String,
    payload_type: u8,
    clock_rate: u32,
    sample_rate: SampleRate,
    channels: Channels,
    sample_entry: SampleEntry,
    size_length: u8,
    index_length: u8,
    index_delta_length: u8,
}

#[derive(Debug, Clone)]
struct SelectedTracks {
    video: Option<VideoTrackConfig>,
    audio: Option<AudioTrackConfig>,
    play_url: String,
}

#[derive(Debug)]
/// subscriber の audio 側出力先とデコーダー、 および video 側 decoder task への入力チャネル
/// (`input_tx.clone()`) をまとめた構造体。 video 側の `TrackPublisher` は endpoint 寿命の
/// decoder task に move されているため、 context には保持しない (issue 0072)。
struct RtspOutputContext<'a> {
    audio_track_tx: &'a mut Option<TrackPublisher>,
    audio_decoder: &'a mut Option<crate::decoder::AudioDecoder>,
    video_decoder_input_tx: &'a mut Option<tokio::sync::mpsc::UnboundedSender<DecoderInput>>,
}

struct RtspSessionRunner {
    stream: crate::tcp::TcpOrTlsStream,
    connection: RtspClientConnection,
    recv_buf: Vec<u8>,
    pending_responses: VecDeque<RtspResponse>,
    parsed_url: ParsedRtspUrl,
    auth: Option<RtspAuthorization>,
    session_id: Option<String>,
    video_receiver: Option<VideoRtpReceiver>,
    audio_receiver: Option<AudioRtpReceiver>,
    keepalive_uri: String,
}

#[derive(Debug)]
enum RtspAuthorization {
    Basic(String),
    Digest(DigestChallenge),
}

#[derive(Debug)]
struct VideoRtpReceiver {
    rtp_channel: u8,
    payload_type: u8,
    timestamp_mapper: TimestampMapper,
    depacketizer: H264RtpDepacketizer,
    /// 最後に確定したサンプルエントリー。`None` の間はフレームを下流に流さない（ゲート）。
    last_sample_entry: Option<SharedSampleEntry>,
}

impl VideoRtpReceiver {
    /// depacketizer 出力 frame の NAL を走査し、IDR + SPS + PPS の 3 条件が揃った場合のみ
    /// サンプルエントリーを構築して `self.last_sample_entry` を上書きする。
    ///
    /// 3 条件不揃いの IDR（SDP `sprop-parameter-sets` で初期確定済みの RTSP カメラが
    /// mid-stream で素の VCL NAL のみを送る場合を想定）は更新スキップで `Ok(())` を返す。
    /// NAL 走査自身の Err はそのまま `Err` で伝播し、呼び出し側で `SessionError::Fatal` に
    /// 変換されて RTSP 接続が打ち切られる。
    fn apply_sample_entry(&mut self, frame: &DepacketizedVideoFrame) -> crate::Result<()> {
        // IDR 判定と SPS / PPS NAL 本体収集を同じループで実施する。
        // 3 条件 (IDR + SPS + PPS) が揃ったときのみサンプルエントリーを構築する。
        let mut has_idr = false;
        let mut sps_list: Vec<Vec<u8>> = Vec::new();
        let mut pps_list: Vec<Vec<u8>> = Vec::new();
        for nalu in crate::video::h264::H264AnnexBNalUnits::new(&frame.data) {
            let nalu = nalu?;
            match nalu.ty {
                crate::video::h264::H264_NALU_TYPE_IDR => has_idr = true,
                crate::video::h264::H264_NALU_TYPE_SPS => sps_list.push(nalu.data.to_vec()),
                crate::video::h264::H264_NALU_TYPE_PPS => pps_list.push(nalu.data.to_vec()),
                _ => {}
            }
        }

        if has_idr && !sps_list.is_empty() && !pps_list.is_empty() {
            let (entry, _frame_size) =
                crate::video::h264::h264_sample_entry_from_sps_pps_lists(sps_list, pps_list)?;
            self.last_sample_entry = Some(SharedSampleEntry::new(entry));
        }

        Ok(())
    }
}

#[derive(Debug)]
struct AudioRtpReceiver {
    rtp_channel: u8,
    payload_type: u8,
    timestamp_mapper: TimestampMapper,
    depacketizer: AacRtpDepacketizer,
    sample_rate: SampleRate,
    channels: Channels,
    /// `AudioFrame.sample_entry` の不変条件（issue 0030）に従い、
    /// SDP 由来のサンプルエントリーを共有型で保持して全 AAC AU に clone して付与する。
    sample_entry: SharedSampleEntry,
}

#[derive(Debug, Default)]
struct ReconnectBackoff {
    current: Option<Duration>,
}

impl ReconnectBackoff {
    fn new() -> Self {
        Self::default()
    }

    fn reset(&mut self) {
        self.current = None;
    }

    fn next_delay(&mut self) -> Duration {
        let next = match self.current {
            Some(current) => (current * 2).min(RECONNECT_DELAY_MAX),
            None => RECONNECT_DELAY_INITIAL,
        };
        self.current = Some(next);
        next
    }
}

async fn run_rtsp_session(
    parsed_url: &ParsedRtspUrl,
    want_audio: bool,
    want_video: bool,
    connection_offset: Duration,
    stats: &RtspSubscriberStats,
    output: &mut RtspOutputContext<'_>,
) -> Result<(), SessionError> {
    let stream =
        crate::tcp::TcpOrTlsStream::connect(&parsed_url.host, parsed_url.port, parsed_url.tls)
            .await
            .map_err(|e| {
                SessionError::Retryable(Error::new(format!("failed to connect RTSP endpoint: {e}")))
            })?;

    let mut runner = RtspSessionRunner {
        stream,
        connection: RtspClientConnection::new(),
        recv_buf: vec![0u8; 64 * 1024],
        pending_responses: VecDeque::new(),
        parsed_url: parsed_url.clone(),
        auth: None,
        session_id: None,
        video_receiver: None,
        audio_receiver: None,
        // setup_session 完了後に selected.play_url で上書きされる。
        // ここでは初期化要件を満たすために request_url を入れておく。
        keepalive_uri: parsed_url.request_url.clone(),
    };

    runner
        .setup_session(want_audio, want_video, connection_offset)
        .await?;
    stats.set_connected(true);
    if runner.audio_receiver.is_some() {
        stats.set_audio_codec(crate::types::CodecName::Aac);
    }
    if runner.video_receiver.is_some() {
        stats.set_video_codec(crate::types::CodecName::H264);
    }

    runner.play_loop(output, stats).await.inspect_err(|_| {
        stats.set_connected(false);
    })
}

impl RtspSessionRunner {
    async fn setup_session(
        &mut self,
        want_audio: bool,
        want_video: bool,
        connection_offset: Duration,
    ) -> Result<(), SessionError> {
        let request_url = self.parsed_url.request_url.clone();
        self.send_request_expect_success(RtspMethod::Options, &request_url, |req| req)
            .await?;

        let describe_response = self
            .send_request_expect_success(RtspMethod::Describe, &request_url, |req| {
                req.accept("application/sdp")
            })
            .await?;

        let sdp_base_url = describe_response
            .get_header("Content-Base")
            .map(str::to_owned)
            .unwrap_or_else(|| self.parsed_url.request_url.clone());
        let sdp_text = String::from_utf8(describe_response.body).map_err(|e| {
            SessionError::Fatal(Error::new(format!(
                "failed to parse SDP body as UTF-8: {e}"
            )))
        })?;
        let sdp = Sdp::parse(&sdp_text)
            .map_err(|e| SessionError::Fatal(Error::new(format!("failed to parse SDP: {e}"))))?;
        let selected = select_tracks(&sdp, &sdp_base_url, want_audio, want_video)
            .map_err(SessionError::Fatal)?;
        // keepalive は PLAY 対象 URI に対して送る。
        self.keepalive_uri = selected.play_url.clone();

        let mut next_channel = 0u8;

        if let Some(video) = selected.video {
            let rtp_channel = next_channel;
            let rtcp_channel = next_channel
                .checked_add(1)
                .expect("BUG: RTSP interleaved channel overflow");
            next_channel = next_channel
                .checked_add(2)
                .expect("BUG: RTSP interleaved channel overflow");

            let transport = format!(
                "RTP/AVP/TCP;unicast;interleaved={}-{}",
                rtp_channel, rtcp_channel
            );
            let setup_response = self
                .send_request_expect_success(RtspMethod::Setup, &video.control_url, |req| {
                    req.transport(&transport)
                })
                .await?;
            self.update_session_id(&setup_response)?;
            let accepted_channel = setup_response
                .get_header("Transport")
                .and_then(|value| parse_interleaved_channel(value).ok())
                .unwrap_or(rtp_channel);

            let last_sample_entry = video.sample_entry.map(SharedSampleEntry::new);

            self.video_receiver = Some(VideoRtpReceiver {
                rtp_channel: accepted_channel,
                payload_type: video.payload_type,
                timestamp_mapper: TimestampMapper::new(
                    32,
                    u64::from(video.clock_rate),
                    connection_offset,
                )
                .map_err(SessionError::Fatal)?,
                depacketizer: H264RtpDepacketizer::new(),
                last_sample_entry,
            });
        }

        if let Some(audio) = selected.audio {
            let rtp_channel = next_channel;
            let rtcp_channel = next_channel
                .checked_add(1)
                .expect("BUG: RTSP interleaved channel overflow");

            let transport = format!(
                "RTP/AVP/TCP;unicast;interleaved={}-{}",
                rtp_channel, rtcp_channel
            );
            let setup_response = self
                .send_request_expect_success(RtspMethod::Setup, &audio.control_url, |req| {
                    req.transport(&transport)
                })
                .await?;
            self.update_session_id(&setup_response)?;
            let accepted_channel = setup_response
                .get_header("Transport")
                .and_then(|value| parse_interleaved_channel(value).ok())
                .unwrap_or(rtp_channel);

            self.audio_receiver = Some(AudioRtpReceiver {
                rtp_channel: accepted_channel,
                payload_type: audio.payload_type,
                timestamp_mapper: TimestampMapper::new(
                    32,
                    u64::from(audio.clock_rate),
                    connection_offset,
                )
                .map_err(SessionError::Fatal)?,
                depacketizer: AacRtpDepacketizer::new(
                    audio.size_length,
                    audio.index_length,
                    audio.index_delta_length,
                ),
                sample_rate: audio.sample_rate,
                channels: audio.channels,
                sample_entry: SharedSampleEntry::new(audio.sample_entry),
            });
        }

        let keepalive_uri = self.keepalive_uri.clone();
        self.send_request_expect_success(RtspMethod::Play, &keepalive_uri, |req| req)
            .await?;

        Ok(())
    }

    async fn play_loop(
        &mut self,
        output: &mut RtspOutputContext<'_>,
        stats: &RtspSubscriberStats,
    ) -> Result<(), SessionError> {
        let mut keepalive_interval = tokio::time::interval(KEEPALIVE_INTERVAL);
        keepalive_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                read_result = self.stream.read(&mut self.recv_buf) => {
                    let n = read_result.map_err(|e| SessionError::Retryable(Error::new(format!("failed to read RTSP stream: {e}"))))?;
                    if n == 0 {
                        return Err(SessionError::Retryable(Error::new("RTSP connection closed by peer")));
                    }
                    self.connection
                        .feed_recv_buf(&self.recv_buf[..n])
                        .map_err(|e| SessionError::Retryable(Error::new(format!("failed to parse RTSP stream: {e}"))))?;
                    self.process_events(output, stats)?;
                }
                _ = keepalive_interval.tick() => {
                    self.send_keepalive().await?;
                }
            }
        }
    }

    async fn send_keepalive(&mut self) -> Result<(), SessionError> {
        let method = RtspMethod::GetParameter;
        let request = self.apply_common_headers(
            RtspRequest::new(method.clone(), &self.keepalive_uri),
            &method,
            &self.keepalive_uri,
        )?;
        self.connection.send_request(request).map_err(|e| {
            SessionError::Retryable(Error::new(format!("failed to send keepalive request: {e}")))
        })?;
        self.flush_send_buffer().await
    }

    fn process_events(
        &mut self,
        output: &mut RtspOutputContext<'_>,
        stats: &RtspSubscriberStats,
    ) -> Result<(), SessionError> {
        while let Some(event) = self.connection.next_event() {
            match event {
                RtspConnectionEvent::ResponseReceived(response) => {
                    if !response.is_success() {
                        return Err(SessionError::Retryable(Error::new(format!(
                            "RTSP keepalive failed: status={} reason={}",
                            response.status_code, response.reason_phrase
                        ))));
                    }
                }
                RtspConnectionEvent::RtpReceived { channel, packet } => {
                    self.handle_rtp_packet(channel, packet, output, stats)?
                }
                RtspConnectionEvent::RtcpReceived { .. } => {}
                RtspConnectionEvent::InterleavedData { .. } => {}
                RtspConnectionEvent::RequestReceived(_) => {}
                RtspConnectionEvent::Redirect { location } => {
                    return Err(SessionError::Retryable(Error::new(format!(
                        "RTSP server requested redirect: {location}",
                    ))));
                }
                RtspConnectionEvent::Error(reason) => {
                    return Err(SessionError::Retryable(Error::new(format!(
                        "RTSP connection event error: {reason}",
                    ))));
                }
                RtspConnectionEvent::StateChanged(_) => {}
            }
        }

        Ok(())
    }

    fn process_events_for_response(&mut self) -> Result<(), SessionError> {
        while let Some(event) = self.connection.next_event() {
            match event {
                RtspConnectionEvent::ResponseReceived(response) => {
                    self.pending_responses.push_back(response);
                }
                RtspConnectionEvent::RtpReceived { .. } => {}
                RtspConnectionEvent::RtcpReceived { .. } => {}
                RtspConnectionEvent::InterleavedData { .. } => {}
                RtspConnectionEvent::RequestReceived(_) => {}
                RtspConnectionEvent::Redirect { location } => {
                    return Err(SessionError::Retryable(Error::new(format!(
                        "RTSP server requested redirect: {location}",
                    ))));
                }
                RtspConnectionEvent::Error(reason) => {
                    return Err(SessionError::Retryable(Error::new(format!(
                        "RTSP connection event error: {reason}",
                    ))));
                }
                RtspConnectionEvent::StateChanged(_) => {}
            }
        }
        Ok(())
    }

    fn handle_rtp_packet(
        &mut self,
        channel: u8,
        packet: shiguredo_rtsp::RtpPacket,
        output: &mut RtspOutputContext<'_>,
        stats: &RtspSubscriberStats,
    ) -> Result<(), SessionError> {
        if let Some(video_receiver) = self.video_receiver.as_mut()
            && channel == video_receiver.rtp_channel
            && packet.header.payload_type == video_receiver.payload_type
        {
            let frames = video_receiver
                .depacketizer
                .push_packet(packet)
                .map_err(SessionError::Fatal)?;
            for frame in frames {
                let timestamp = video_receiver
                    .timestamp_mapper
                    .map(u64::from(frame.rtp_timestamp));

                video_receiver
                    .apply_sample_entry(&frame)
                    .map_err(SessionError::Fatal)?;

                // sample_entry 未確定の間は破棄する（stats にもカウントしない）。
                let Some(sample_entry) = video_receiver.last_sample_entry.clone() else {
                    continue;
                };

                let video_frame = VideoFrame {
                    data: frame.data,
                    format: VideoFormat::H264AnnexB,
                    keyframe: frame.keyframe,
                    size: None,
                    timestamp,
                    sample_entry: Some(sample_entry),
                };
                stats.add_input_video_frame_count();
                stats.set_last_input_video_timestamp(timestamp);
                if let Some(tx) = output.video_decoder_input_tx.as_ref() {
                    // decoder task の同期 unbounded_channel::send を経由して投入する。
                    // Err (task 死亡) は Fatal 相当 (task の再 spawn 経路を持たない設計)。
                    tx.send(DecoderInput::Media(crate::MediaFrame::new_video(
                        video_frame,
                    )))
                    .map_err(|_| {
                        SessionError::Fatal(Error::new(
                            "video decoder task terminated unexpectedly",
                        ))
                    })?;
                }
            }
            return Ok(());
        }

        if let Some(audio_receiver) = self.audio_receiver.as_mut()
            && channel == audio_receiver.rtp_channel
            && packet.header.payload_type == audio_receiver.payload_type
        {
            let access_units = audio_receiver
                .depacketizer
                .depacketize(&packet)
                .map_err(SessionError::Fatal)?;
            for access_unit in access_units {
                let timestamp = audio_receiver
                    .timestamp_mapper
                    .map(u64::from(access_unit.rtp_timestamp));
                // `AudioFrame.sample_entry` の不変条件（issue 0030）に従い全フレームに付与する。
                let audio_frame = AudioFrame {
                    data: access_unit.data,
                    format: AudioFormat::Aac,
                    channels: audio_receiver.channels,
                    sample_rate: audio_receiver.sample_rate,
                    timestamp,
                    sample_entry: Some(audio_receiver.sample_entry.clone()),
                };
                stats.add_input_audio_data_count();
                stats.set_last_input_audio_timestamp(timestamp);
                if let Some(decoder) = output.audio_decoder.as_mut()
                    && let Some(tx) = output.audio_track_tx.as_mut()
                {
                    decoder
                        .handle_input_sample(Some(crate::MediaFrame::Audio(std::sync::Arc::new(
                            audio_frame,
                        ))))
                        .map_err(SessionError::Fatal)?;
                    // Finished は EOS 入力時にしか発生しないため、通常フレーム処理中は Pending のみ返る
                    if crate::decoder::drain_audio_decoder_output(decoder, tx)
                        .map_err(SessionError::Fatal)?
                        == crate::decoder::DrainResult::PipelineClosed
                    {
                        return Err(SessionError::Fatal(Error::new(
                            "audio track pipeline closed",
                        )));
                    }
                }
            }
        }

        Ok(())
    }

    async fn send_request_expect_success<F>(
        &mut self,
        method: RtspMethod,
        uri: &str,
        build_request: F,
    ) -> Result<RtspResponse, SessionError>
    where
        F: Fn(RtspRequest) -> RtspRequest,
    {
        for attempt in 0..2 {
            let request = self.apply_common_headers(
                build_request(RtspRequest::new(method.clone(), uri)),
                &method,
                uri,
            )?;
            self.connection.send_request(request).map_err(|e| {
                SessionError::Retryable(Error::new(format!("failed to send RTSP request: {e}")))
            })?;
            self.flush_send_buffer().await?;

            let response = self.wait_for_response().await?;
            if response.status_code == 401
                && attempt == 0
                && self.try_update_auth_header(&response)?
            {
                continue;
            }

            if response.is_success() {
                return Ok(response);
            }

            let error = Error::new(format!(
                "RTSP {} failed: status={} reason={}",
                method.as_str(),
                response.status_code,
                response.reason_phrase
            ));
            if response.is_server_error() {
                return Err(SessionError::Retryable(error));
            }
            return Err(SessionError::Fatal(error));
        }

        Err(SessionError::Fatal(Error::new(format!(
            "RTSP {} failed with unauthorized response",
            method.as_str()
        ))))
    }

    fn try_update_auth_header(&mut self, response: &RtspResponse) -> Result<bool, SessionError> {
        let Some(credentials) = self.parsed_url.credentials.as_ref() else {
            return Ok(false);
        };

        let Some(challenge_value) = response.get_header("WWW-Authenticate") else {
            return Ok(false);
        };

        if challenge_value.to_ascii_lowercase().starts_with("basic") {
            let basic =
                BasicAuth::new(&credentials.username, &credentials.password).map_err(|e| {
                    SessionError::Fatal(Error::new(format!(
                        "failed to build Basic auth header: {e}"
                    )))
                })?;
            self.auth = Some(RtspAuthorization::Basic(basic.to_header_value()));
            return Ok(true);
        }

        if challenge_value.to_ascii_lowercase().starts_with("digest") {
            let challenge = DigestChallenge::parse(challenge_value).map_err(|e| {
                SessionError::Fatal(Error::new(format!("failed to parse Digest challenge: {e}")))
            })?;
            self.auth = Some(RtspAuthorization::Digest(challenge));
            return Ok(true);
        }

        Ok(false)
    }

    fn apply_common_headers(
        &self,
        mut request: RtspRequest,
        method: &RtspMethod,
        uri: &str,
    ) -> Result<RtspRequest, SessionError> {
        if let Some(auth) = self.auth.as_ref() {
            match auth {
                RtspAuthorization::Basic(value) => {
                    request = request.header("Authorization", value);
                }
                RtspAuthorization::Digest(challenge) => {
                    let credentials = self.parsed_url.credentials.as_ref().ok_or_else(|| {
                        SessionError::Fatal(Error::new(
                            "Digest auth requires credentials in input_url",
                        ))
                    })?;
                    let value = shiguredo_rtsp::auth::build_authorization(
                        &DigestCredentials {
                            username: credentials.username.clone(),
                            password: credentials.password.clone(),
                        },
                        challenge,
                        method.as_str(),
                        uri,
                    );
                    request = request.header("Authorization", value.as_str());
                }
            }
        }
        if let Some(value) = self.session_id.as_deref() {
            request = request.header("Session", value);
        }
        Ok(request)
    }

    fn update_session_id(&mut self, response: &RtspResponse) -> Result<(), SessionError> {
        let Some(raw_value) = response.get_header("Session") else {
            return Ok(());
        };
        let Some(parsed_value) = parse_rtsp_session_id(raw_value) else {
            return Err(SessionError::Fatal(Error::new(format!(
                "invalid RTSP Session header: {raw_value}",
            ))));
        };

        match self.session_id.as_deref() {
            Some(current) if current != parsed_value => Err(SessionError::Fatal(Error::new(
                format!("conflicting RTSP Session header: current={current} new={parsed_value}",),
            ))),
            Some(_) => Ok(()),
            None => {
                self.session_id = Some(parsed_value.to_owned());
                Ok(())
            }
        }
    }

    async fn wait_for_response(&mut self) -> Result<RtspResponse, SessionError> {
        loop {
            if let Some(response) = self.pending_responses.pop_front() {
                return Ok(response);
            }

            let n = self.stream.read(&mut self.recv_buf).await.map_err(|e| {
                SessionError::Retryable(Error::new(format!("failed to read RTSP response: {e}")))
            })?;
            if n == 0 {
                return Err(SessionError::Retryable(Error::new(
                    "RTSP connection closed while waiting for response",
                )));
            }

            self.connection
                .feed_recv_buf(&self.recv_buf[..n])
                .map_err(|e| {
                    SessionError::Retryable(Error::new(format!(
                        "failed to parse RTSP response: {e}"
                    )))
                })?;
            self.process_events_for_response()?;
        }
    }

    async fn flush_send_buffer(&mut self) -> Result<(), SessionError> {
        while !self.connection.send_buf().is_empty() {
            let written = self
                .stream
                .write(self.connection.send_buf())
                .await
                .map_err(|e| {
                    SessionError::Retryable(Error::new(format!("failed to send RTSP bytes: {e}")))
                })?;
            if written == 0 {
                return Err(SessionError::Retryable(Error::new(
                    "failed to send RTSP bytes: write returned 0",
                )));
            }
            self.connection.advance_send_buf(written);
        }
        self.stream.flush().await.map_err(|e| {
            SessionError::Retryable(Error::new(format!("failed to flush RTSP stream: {e}")))
        })
    }
}

#[derive(Debug, Clone)]
struct DepacketizedVideoFrame {
    rtp_timestamp: u32,
    keyframe: bool,
    data: Vec<u8>,
}

#[derive(Debug, Default)]
struct H264RtpDepacketizer {
    current_timestamp: Option<u32>,
    current_data: Vec<u8>,
    current_has_keyframe: bool,
}

impl H264RtpDepacketizer {
    fn new() -> Self {
        Self::default()
    }

    fn push_packet(
        &mut self,
        packet: shiguredo_rtsp::RtpPacket,
    ) -> crate::Result<Vec<DepacketizedVideoFrame>> {
        if packet.payload.is_empty() {
            return Ok(Vec::new());
        }

        let mut out = Vec::new();
        if let Some(ts) = self.current_timestamp
            && ts != packet.header.timestamp
            && !self.current_data.is_empty()
        {
            out.push(self.take_frame(ts));
        }
        if self.current_timestamp.is_none() {
            self.current_timestamp = Some(packet.header.timestamp);
        }

        let nal_unit_type = packet.payload[0] & 0x1f;
        match nal_unit_type {
            1..=23 => {
                self.append_annexb_nalu(&packet.payload);
                if nal_unit_type == crate::video::h264::H264_NALU_TYPE_IDR {
                    self.current_has_keyframe = true;
                }
            }
            24 => {
                let mut pos = 1usize;
                while pos + 2 <= packet.payload.len() {
                    let nalu_size =
                        u16::from_be_bytes([packet.payload[pos], packet.payload[pos + 1]]) as usize;
                    pos += 2;
                    if pos + nalu_size > packet.payload.len() {
                        return Err(Error::new(
                            "invalid STAP-A payload: NAL unit size is truncated",
                        ));
                    }
                    let nalu = &packet.payload[pos..pos + nalu_size];
                    if let Some(header) = nalu.first()
                        && header & 0x1f == crate::video::h264::H264_NALU_TYPE_IDR
                    {
                        self.current_has_keyframe = true;
                    }
                    self.append_annexb_nalu(nalu);
                    pos += nalu_size;
                }
            }
            28 => {
                if packet.payload.len() < 2 {
                    return Err(Error::new("invalid FU-A payload: too short"));
                }
                let fu_indicator = packet.payload[0];
                let fu_header = packet.payload[1];
                let start = fu_header & 0x80 != 0;
                let reconstructed_nal = (fu_indicator & 0b1110_0000) | (fu_header & 0b0001_1111);
                if start {
                    self.current_data
                        .extend_from_slice(&[0, 0, 0, 1, reconstructed_nal]);
                    if reconstructed_nal & 0x1f == crate::video::h264::H264_NALU_TYPE_IDR {
                        self.current_has_keyframe = true;
                    }
                }
                self.current_data.extend_from_slice(&packet.payload[2..]);
            }
            _ => {
                return Err(Error::new(format!(
                    "unsupported H264 RTP packetization type: {nal_unit_type}"
                )));
            }
        }

        if packet.header.marker && !self.current_data.is_empty() {
            let ts = self.current_timestamp.unwrap_or(packet.header.timestamp);
            out.push(self.take_frame(ts));
        }

        Ok(out)
    }

    fn append_annexb_nalu(&mut self, nalu: &[u8]) {
        self.current_data.extend_from_slice(&[0, 0, 0, 1]);
        self.current_data.extend_from_slice(nalu);
    }

    fn take_frame(&mut self, timestamp: u32) -> DepacketizedVideoFrame {
        let data = std::mem::take(&mut self.current_data);
        let keyframe = self.current_has_keyframe;
        self.current_has_keyframe = false;
        self.current_timestamp = None;
        DepacketizedVideoFrame {
            rtp_timestamp: timestamp,
            keyframe,
            data,
        }
    }
}

fn parse_rtsp_input_url(input_url: &str) -> Result<ParsedRtspUrl, String> {
    let uri = Uri::parse(input_url).map_err(|e| format!("failed to parse URL: {e}"))?;
    let scheme = uri
        .scheme()
        .ok_or_else(|| "input_url must contain URL scheme".to_owned())?;
    let tls = match scheme {
        "rtsp" => false,
        "rtsps" => true,
        _ => return Err("input_url scheme must be rtsp or rtsps".to_owned()),
    };
    let host = uri
        .host()
        .ok_or_else(|| "input_url must contain host".to_owned())?
        .to_owned();
    let port = uri.port().unwrap_or(DEFAULT_RTSP_PORT);
    let authority = uri
        .authority()
        .ok_or_else(|| "input_url must contain authority".to_owned())?;
    let (credentials, authority_without_userinfo) = parse_authority(authority)?;

    let mut path_and_query = uri.path().to_owned();
    if path_and_query.is_empty() {
        path_and_query = "/".to_owned();
    }
    if let Some(query) = uri.query() {
        path_and_query.push('?');
        path_and_query.push_str(query);
    }

    let request_url = format!("{scheme}://{authority_without_userinfo}{path_and_query}");
    Uri::parse(&request_url).map_err(|e| format!("failed to build request URL: {e}"))?;

    Ok(ParsedRtspUrl {
        host,
        port,
        tls,
        request_url,
        credentials,
    })
}

fn parse_authority(authority: &str) -> Result<(Option<RtspCredentials>, String), String> {
    let Some((userinfo, host_port)) = authority.rsplit_once('@') else {
        return Ok((None, authority.to_owned()));
    };
    if userinfo.is_empty() {
        return Err("input_url must not contain empty username".to_owned());
    }

    let (username, password) = match userinfo.split_once(':') {
        Some((username, password)) => (username, password),
        None => (userinfo, ""),
    };
    if username.is_empty() {
        return Err("input_url username must not be empty".to_owned());
    }

    Ok((
        Some(RtspCredentials {
            username: username.to_owned(),
            password: password.to_owned(),
        }),
        host_port.to_owned(),
    ))
}

fn select_tracks(
    sdp: &Sdp,
    base_url: &str,
    want_audio: bool,
    want_video: bool,
) -> crate::Result<SelectedTracks> {
    let session_control = extract_control(&sdp.attributes);
    let play_url = match session_control {
        Some("*") | None => base_url.to_owned(),
        Some(control) => resolve_rtsp_url(base_url, control)?,
    };

    let mut selected_video = None;
    let mut selected_audio = None;

    for media in &sdp.media {
        if media.port == 0 {
            continue;
        }

        if want_video && selected_video.is_none() && media.media_type.eq_ignore_ascii_case("video")
        {
            selected_video = select_video_track(media, base_url)?;
        }
        if want_audio && selected_audio.is_none() && media.media_type.eq_ignore_ascii_case("audio")
        {
            selected_audio = select_audio_track(media, base_url)?;
        }
    }

    if want_video && selected_video.is_none() {
        return Err(Error::new(
            "failed to find supported H264 video track in SDP",
        ));
    }
    if want_audio && selected_audio.is_none() {
        return Err(Error::new(
            "failed to find supported MPEG4-GENERIC audio track in SDP",
        ));
    }

    Ok(SelectedTracks {
        video: selected_video,
        audio: selected_audio,
        play_url,
    })
}

fn select_video_track(
    media: &shiguredo_rtsp::sdp::SdpMedia,
    base_url: &str,
) -> crate::Result<Option<VideoTrackConfig>> {
    let control = extract_control(&media.attributes)
        .ok_or_else(|| Error::new("video media is missing control attribute"))?;
    let control_url = resolve_rtsp_url(base_url, control)?;

    for payload in &media.formats {
        let Ok(payload_type) = payload.parse::<u8>() else {
            continue;
        };
        if let Some((encoding, clock_rate)) = find_rtpmap(&media.attributes, payload_type)
            && encoding.eq_ignore_ascii_case("H264")
        {
            let sample_entry = extract_sample_entry_from_sprop(&media.attributes, payload_type)?;
            return Ok(Some(VideoTrackConfig {
                control_url,
                payload_type,
                clock_rate,
                sample_entry,
            }));
        }
    }

    Ok(None)
}

/// SDP fmtp 行から `sprop-parameter-sets` (RFC 6184 §8.2.1) を抽出して、SPS / PPS NAL リストから
/// `h264_sample_entry_from_sps_pps_lists` でサンプルエントリーを構築する。
///
/// パース方針: 不完全な補助メタデータは fail-fast にせず Ok(None) で代替経路に委ねる一方、
/// 構造的に壊れた SDP は Err で接続を打ち切る。両者の境界は以下のとおり:
///
/// 戻り値:
/// - 以下のいずれかは `Ok(None)`（`sprop-parameter-sets` は MAY 扱いの補助情報のため、
///   不完全な構成は許容し IDR 内 inline SPS / PPS による代替経路
///   (`VideoRtpReceiver::apply_sample_entry`) に委ねる）:
///   - fmtp 不在 / `sprop-parameter-sets` 不在 / 値が空文字列 / 空要素のみ
///   - 空 NAL (Base64 デコード結果が 0 バイト) — 該当要素のみ continue で読み飛ばす
///   - SPS または PPS の片方が欠ける場合
/// - 以下は `crate::Error` で伝播（壊れた SDP として接続を打ち切る）:
///   - Base64 デコード失敗 (要素自体が不正)
///   - forbidden_zero_bit が立った NAL ヘッダ (ITU-T H.264 7.4.1 違反)
///   - `h264_sample_entry_from_sps_pps_lists` 内の Err (SPS パース失敗 等)
fn extract_sample_entry_from_sprop(
    attributes: &[SdpAttribute],
    payload_type: u8,
) -> crate::Result<Option<SampleEntry>> {
    let Some(fmtp) = find_fmtp(attributes, payload_type) else {
        return Ok(None);
    };
    let params = parse_fmtp_parameters(&fmtp);
    let Some(sprop_value) = params.get("sprop-parameter-sets").map(String::as_str) else {
        return Ok(None);
    };
    if sprop_value.is_empty() {
        return Ok(None);
    }

    // sprop-parameter-sets の各要素は Base64 エンコードされた SPS / PPS NAL (start code なし)。
    // Base64 デコード後の先頭バイトの NAL タイプで SPS / PPS を判別して直接リストへ詰める。
    let mut sps_list: Vec<Vec<u8>> = Vec::new();
    let mut pps_list: Vec<Vec<u8>> = Vec::new();
    for raw_entry in sprop_value.split(',') {
        let trimmed = raw_entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        let nal = Base64::decode_vec(trimmed)
            .map_err(|e| Error::new(format!("invalid sprop-parameter-sets base64: {e}")))?;
        if nal.is_empty() {
            continue;
        }
        // NAL ヘッダ 1 バイトの最上位 bit は forbidden_zero_bit で ITU-T H.264 7.4.1 上 0 (MUST)。
        // Annex-B 走査経路 (H264AnnexBNalUnits) と対称に Err で接続を打ち切る。
        if (nal[0] >> 7) != 0 {
            return Err(Error::new(
                "invalid H.264 NAL header in sprop-parameter-sets: forbidden_zero_bit is set",
            ));
        }
        // NAL ヘッダ 1 バイトの下位 5 bit が NAL ユニットタイプ
        let nal_unit_type = nal[0] & 0x1F;
        match nal_unit_type {
            crate::video::h264::H264_NALU_TYPE_SPS => sps_list.push(nal),
            crate::video::h264::H264_NALU_TYPE_PPS => pps_list.push(nal),
            _ => {}
        }
    }

    // sprop に SPS と PPS が両方含まれない場合は inline 経路に委ねる。
    // fmtp 全体不在を Ok(None) で許容するのと同じ方針で、不完全な補助メタデータを fail-fast にしない。
    if sps_list.is_empty() || pps_list.is_empty() {
        return Ok(None);
    }

    let (entry, _frame_size) =
        crate::video::h264::h264_sample_entry_from_sps_pps_lists(sps_list, pps_list)?;
    Ok(Some(entry))
}

fn select_audio_track(
    media: &shiguredo_rtsp::sdp::SdpMedia,
    base_url: &str,
) -> crate::Result<Option<AudioTrackConfig>> {
    let control = extract_control(&media.attributes)
        .ok_or_else(|| Error::new("audio media is missing control attribute"))?;
    let control_url = resolve_rtsp_url(base_url, control)?;

    for payload in &media.formats {
        let Ok(payload_type) = payload.parse::<u8>() else {
            continue;
        };
        let Some((encoding, clock_rate)) = find_rtpmap(&media.attributes, payload_type) else {
            continue;
        };
        if !encoding.eq_ignore_ascii_case("MPEG4-GENERIC") {
            continue;
        }

        let fmtp = find_fmtp(&media.attributes, payload_type)
            .ok_or_else(|| Error::new("audio media is missing fmtp attribute for MPEG4-GENERIC"))?;
        let params = parse_fmtp_parameters(&fmtp);
        let config_hex = params
            .get("config")
            .ok_or_else(|| Error::new("AAC fmtp is missing config parameter"))?;
        let config = parse_hex(config_hex)?;
        let (sample_rate, channels) = crate::audio::aac::parse_audio_specific_config(&config)?;
        let sample_entry =
            crate::audio::aac::create_mp4a_sample_entry(&config, sample_rate, channels)?;
        let size_length = params
            .get("sizelength")
            .and_then(|v| v.parse::<u8>().ok())
            .unwrap_or(13);
        let index_length = params
            .get("indexlength")
            .and_then(|v| v.parse::<u8>().ok())
            .unwrap_or(3);
        let index_delta_length = params
            .get("indexdeltalength")
            .and_then(|v| v.parse::<u8>().ok())
            .unwrap_or(3);

        validate_aac_fmtp_lengths(size_length, index_length, index_delta_length)?;

        return Ok(Some(AudioTrackConfig {
            control_url,
            payload_type,
            clock_rate,
            sample_rate,
            channels,
            sample_entry,
            size_length,
            index_length,
            index_delta_length,
        }));
    }

    Ok(None)
}

fn parse_interleaved_channel(transport_header: &str) -> crate::Result<u8> {
    let transports = RtspTransport::parse_multiple(transport_header);
    for transport in transports {
        if let Some((rtp_channel, _)) = transport.interleaved {
            return Ok(rtp_channel);
        }
    }
    Err(Error::new(
        "RTSP SETUP response is missing interleaved transport",
    ))
}

fn parse_rtsp_session_id(session_header: &str) -> Option<&str> {
    let trimmed = session_header.trim();
    let (session_id, _) = trimmed.split_once(';').unwrap_or((trimmed, ""));
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return None;
    }
    Some(session_id)
}

fn extract_control(attributes: &[SdpAttribute]) -> Option<&str> {
    attributes.iter().find_map(|attr| {
        if let SdpAttribute::Control(value) = attr {
            Some(value.as_str())
        } else {
            None
        }
    })
}

fn find_rtpmap(attributes: &[SdpAttribute], payload_type: u8) -> Option<(String, u32)> {
    attributes.iter().find_map(|attr| {
        if let SdpAttribute::Rtpmap {
            payload_type: pt,
            encoding,
            clock_rate,
            ..
        } = attr
            && *pt == payload_type
        {
            Some((encoding.clone(), *clock_rate))
        } else {
            None
        }
    })
}

fn find_fmtp(attributes: &[SdpAttribute], payload_type: u8) -> Option<String> {
    attributes.iter().find_map(|attr| {
        if let SdpAttribute::Fmtp {
            payload_type: pt,
            parameters,
        } = attr
            && *pt == payload_type
        {
            Some(parameters.clone())
        } else {
            None
        }
    })
}

fn parse_fmtp_parameters(parameters: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for part in parameters.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((k, v)) = part.split_once('=') {
            map.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    map
}

fn parse_hex(hex: &str) -> crate::Result<Vec<u8>> {
    let mut normalized = hex.trim().to_owned();
    if !normalized.len().is_multiple_of(2) {
        normalized.insert(0, '0');
    }
    let mut out = Vec::with_capacity(normalized.len() / 2);
    let bytes = normalized.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let value = u8::from_str_radix(
            std::str::from_utf8(&bytes[i..i + 2])
                .map_err(|e| Error::new(format!("invalid AAC config encoding: {e}")))?,
            16,
        )
        .map_err(|e| Error::new(format!("invalid AAC config value: {e}")))?;
        out.push(value);
        i += 2;
    }
    Ok(out)
}

fn resolve_rtsp_url(base_url: &str, control: &str) -> crate::Result<String> {
    if control.starts_with("rtsp://") || control.starts_with("rtsps://") {
        Uri::parse(control).map_err(|e| Error::new(format!("invalid RTSP control URL: {e}")))?;
        return Ok(control.to_owned());
    }

    let base =
        Uri::parse(base_url).map_err(|e| Error::new(format!("invalid RTSP base URL: {e}")))?;
    let scheme = base
        .scheme()
        .ok_or_else(|| Error::new("RTSP base URL is missing scheme"))?;
    let authority = base
        .authority()
        .ok_or_else(|| Error::new("RTSP base URL is missing authority"))?;

    let resolved = if control.starts_with('/') {
        format!("{scheme}://{authority}{control}")
    } else {
        let mut base_path = base.path().to_owned();
        if base_path.is_empty() {
            base_path = "/".to_owned();
        }
        let parent_end = base_path.rfind('/').unwrap_or(0);
        let parent = &base_path[..=parent_end];
        format!("{scheme}://{authority}{parent}{control}")
    };

    Uri::parse(&resolved).map_err(|e| Error::new(format!("invalid resolved RTSP URL: {e}")))?;
    Ok(resolved)
}

// video decoder task の spawn pattern (issue 0072)。
// 0071 の `src/mp4/reader.rs:1528-1643` を参照実装として写経したもので、
// warm-up 制御 (`discard_mode_tx`) と `TrackSender` は本 endpoint では不要のため落としてある。
// 共通化 (`src/decoder/task.rs` 等への切り出し) は open issue 0073 で最終判断する。

enum DecoderInput {
    Media(crate::MediaFrame),
    Eos,
}

// Drop trait と shutdown(self) を共存させるため join_handle を Option で保持し
// take() で move する。 直接 JoinHandle を持つと Drop 実装型の partial move が
// E0509 で禁止される。
#[derive(Debug)]
struct VideoDecoderTask {
    input_tx: tokio::sync::mpsc::UnboundedSender<DecoderInput>,
    join_handle: Option<tokio::task::JoinHandle<crate::Result<()>>>,
}

impl VideoDecoderTask {
    async fn shutdown(mut self) -> crate::Result<()> {
        let _ = self.input_tx.send(DecoderInput::Eos);
        let handle = self
            .join_handle
            .take()
            .expect("join_handle is Some until shutdown/Drop consumes it");
        match handle.await {
            Ok(result) => result,
            Err(e) if e.is_panic() => {
                tracing::error!("video decoder task panicked: {e}");
                Err(crate::Error::new(format!(
                    "video decoder task panicked: {e}"
                )))
            }
            Err(e) => Err(crate::Error::new(format!(
                "video decoder task join failed: {e}"
            ))),
        }
    }
}

impl Drop for VideoDecoderTask {
    fn drop(&mut self) {
        // 早期 return / panic unwind 経路で task が leak しないよう abort する。
        // shutdown() が先に呼ばれていれば take 済みで None のため何もしない。
        if let Some(handle) = self.join_handle.take() {
            handle.abort();
        }
    }
}

fn spawn_video_decoder_task(
    options: crate::decoder::VideoDecoderOptions,
    mut stats: crate::stats::Stats,
    output_tx: crate::TrackPublisher,
) -> VideoDecoderTask {
    let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel::<DecoderInput>();
    stats.set_default_label("component", "video_decoder");
    let join_handle =
        tokio::spawn(async move { video_decoder_loop(options, stats, input_rx, output_tx).await });
    VideoDecoderTask {
        input_tx,
        join_handle: Some(join_handle),
    }
}

async fn video_decoder_loop(
    options: crate::decoder::VideoDecoderOptions,
    stats: crate::stats::Stats,
    mut input_rx: tokio::sync::mpsc::UnboundedReceiver<DecoderInput>,
    mut output_tx: crate::TrackPublisher,
) -> crate::Result<()> {
    let mut decoder = crate::decoder::AsyncVideoDecoder::new(options, stats);
    loop {
        let input = match input_rx.recv().await {
            Some(input) => input,
            // main が VideoDecoderTask を drop する経路 (通常は shutdown 経由で EOS 送信済み)。
            None => return Ok(()),
        };
        let is_eos = matches!(input, DecoderInput::Eos);
        match input {
            DecoderInput::Media(sample) => decoder.handle_input_sample_sync(Some(sample))?,
            DecoderInput::Eos => decoder.handle_input_sample_sync(None)?,
        }
        // 1 サンプル入力で 0 個以上のフレームが出力されうるため Pending / Finished まで drain する。
        loop {
            match decoder.poll_output_sync()? {
                crate::decoder::DecoderRunOutput::Processed(sample) => {
                    if !output_tx.send_media(sample) {
                        return Ok(());
                    }
                }
                crate::decoder::DecoderRunOutput::Pending => break,
                crate::decoder::DecoderRunOutput::Finished => {
                    let _ = output_tx.send_eos();
                    return Ok(());
                }
            }
        }
        // AsyncVideoDecoder::poll_output_sync が Empty + eos==true を Finished に射影するため到達不能。
        if is_eos {
            unreachable!("video decoder still pending after EOS");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io;
    use std::time::Duration;

    use shiguredo_rtsp::{RtpPacket, rtp::RtpHeader, rtsp_connection::encode_interleaved_frame};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    #[test]
    fn parse_rtsp_input_url_with_credentials() {
        let parsed = parse_rtsp_input_url("rtsp://user:pass@example.com:8554/live")
            .expect("must parse rtsp URL");
        assert_eq!(parsed.host, "example.com");
        assert_eq!(parsed.port, 8554);
        assert!(!parsed.tls);
        assert_eq!(parsed.request_url, "rtsp://example.com:8554/live");
        let credentials = parsed.credentials.expect("credentials must exist");
        assert_eq!(credentials.username, "user");
        assert_eq!(credentials.password, "pass");
    }

    #[test]
    fn parse_rtsp_input_url_rejects_scheme() {
        let err = parse_rtsp_input_url("http://example.com/live").expect_err("must reject");
        assert_eq!(err, "input_url scheme must be rtsp or rtsps");
    }

    #[test]
    fn parse_rtsp_session_id_extracts_id_before_parameters() {
        assert_eq!(parse_rtsp_session_id("abc123;timeout=60"), Some("abc123"));
        assert_eq!(parse_rtsp_session_id(" abc123 "), Some("abc123"));
        assert_eq!(parse_rtsp_session_id(" ;timeout=60"), None);
    }

    #[test]
    fn parse_hex_supports_odd_length() {
        let bytes = parse_hex("121").expect("must parse");
        assert_eq!(bytes, vec![0x01, 0x21]);
    }

    #[test]
    fn depacketize_h264_fu_a() {
        let mut depacketizer = H264RtpDepacketizer::new();
        let start_packet = shiguredo_rtsp::RtpPacket {
            header: shiguredo_rtsp::rtp::RtpHeader::new(96, 1, 1000, 1),
            extension: None,
            payload: vec![0x7c, 0x85, 0x01, 0x02],
            padding_size: 0,
        };
        let mut end_header = shiguredo_rtsp::rtp::RtpHeader::new(96, 2, 1000, 1);
        end_header.marker = true;
        let end_packet = shiguredo_rtsp::RtpPacket {
            header: end_header,
            extension: None,
            payload: vec![0x7c, 0x45, 0x03, 0x04],
            padding_size: 0,
        };

        assert!(
            depacketizer
                .push_packet(start_packet)
                .expect("must parse")
                .is_empty()
        );
        let frames = depacketizer.push_packet(end_packet).expect("must parse");
        assert_eq!(frames.len(), 1);
        assert!(frames[0].keyframe);
        assert_eq!(frames[0].rtp_timestamp, 1000);
        assert_eq!(
            frames[0].data,
            vec![0, 0, 0, 1, 0x65, 0x01, 0x02, 0x03, 0x04]
        );
    }

    #[test]
    fn depacketize_aac_rejects_zero_au_header_length() {
        let depacketizer = AacRtpDepacketizer::new(13, 3, 3);
        let mut header = shiguredo_rtsp::rtp::RtpHeader::new(97, 1, 9000, 1);
        header.marker = true;
        let packet = shiguredo_rtsp::RtpPacket {
            header,
            extension: None,
            payload: vec![0x00, 0x00],
            padding_size: 0,
        };

        let err = depacketizer.depacketize(&packet).expect_err("must reject");
        assert_eq!(
            err.display(),
            "invalid AAC RTP payload: AU header length must be greater than 0"
        );
    }

    #[test]
    fn depacketize_aac_wraps_bit_reader_error() {
        // `au_headers` slice の実 bit 数で消費しきれない要求量 (AU#2 を読みに行く) を仕込む。
        // size_length=13 / index_length=3 / index_delta_length=3 のため、
        // AU#0 + AU#1 で 32 bit、AU#2 の size を読みに行った時点で `read_u` が枯渇 Err を返す。
        // `Error::with_context` の prefix が付くことを reason フィールドで担保する。
        let depacketizer = AacRtpDepacketizer::new(13, 3, 3);
        let mut header = shiguredo_rtsp::rtp::RtpHeader::new(97, 1, 9000, 1);
        header.marker = true;
        // au_headers_length_bits = 33 (= 0x21)。au_headers slice は ceil(33 / 8) = 5 byte
        // (= 40 bit) 利用可能で、ループ条件 `consumed_bits < 33` のもと AU#0 + AU#1 で
        // 32 bit 消費後、3 周目で AU#2 の size 13 bit を要求し bit 45 まで読もうとして
        // bit 40 で枯渇する。
        let payload = vec![
            0x00, 0x21, 0x00, 0x20, 0x00, 0x10, 0x00, 0xaa, 0xbb, 0xcc, 0xdd, 0x11, 0x22,
        ];
        let packet = shiguredo_rtsp::RtpPacket {
            header,
            extension: None,
            payload,
            padding_size: 0,
        };

        let err = depacketizer
            .depacketize(&packet)
            .expect_err("AU header 枯渇で Err になること");
        assert!(
            err.reason.starts_with("invalid AAC AU header: "),
            "Err の reason に AAC AU header context 文言が前置されること: {}",
            err.reason
        );
        assert!(
            err.reason
                .contains("bit reader: exhausted before requested read"),
            "Err の reason に元の BitReader エラー文言が含まれること: {}",
            err.reason
        );
    }

    #[test]
    fn select_audio_track_rejects_size_length_over_32() {
        // RFC 3640 §3.3.6 の値域外 (`sizelength=33`) を SDP fmtp 受領時点で fail-fast する。
        let sdp = build_test_sdp_with_audio_fmtp(
            "profile-level-id=1;mode=AAC-hbr;sizelength=33;indexlength=3;indexdeltalength=3;config=1190",
        );
        let err = parse_audio_track(&sdp).expect_err("sizelength=33 は Err になること");
        assert_eq!(err.display(), "AAC fmtp sizeLength must be 32 or less");
    }

    #[test]
    fn select_audio_track_rejects_index_length_over_32() {
        // RFC 3640 §3.3.6 の値域外 (`indexlength=33`) を SDP fmtp 受領時点で fail-fast する。
        let sdp = build_test_sdp_with_audio_fmtp(
            "profile-level-id=1;mode=AAC-hbr;sizelength=13;indexlength=33;indexdeltalength=3;config=1190",
        );
        let err = parse_audio_track(&sdp).expect_err("indexlength=33 は Err になること");
        assert_eq!(err.display(), "AAC fmtp indexLength must be 32 or less");
    }

    #[test]
    fn select_audio_track_rejects_index_delta_length_over_32() {
        // RFC 3640 §3.3.6 の値域外 (`indexdeltalength=33`) を SDP fmtp 受領時点で fail-fast する。
        let sdp = build_test_sdp_with_audio_fmtp(
            "profile-level-id=1;mode=AAC-hbr;sizelength=13;indexlength=3;indexdeltalength=33;config=1190",
        );
        let err = parse_audio_track(&sdp).expect_err("indexdeltalength=33 は Err になること");
        assert_eq!(
            err.display(),
            "AAC fmtp indexDeltaLength must be 32 or less"
        );
    }

    #[test]
    fn select_audio_track_accepts_lengths_at_boundary_32() {
        // 上限値 (= 32) は共有 BitReader::read_u が Ok を返す境界。
        // `> 32` Err 化と `== 32` 受理の境界を 3 フィールド一括で担保する。
        let sdp = build_test_sdp_with_audio_fmtp(
            "profile-level-id=1;mode=AAC-hbr;sizelength=32;indexlength=32;indexdeltalength=32;config=1190",
        );
        let cfg = parse_audio_track(&sdp)
            .expect("sizelength=32 / indexlength=32 / indexdeltalength=32 は Ok を返すこと")
            .expect("AudioTrackConfig が返ること");
        assert_eq!(cfg.size_length, 32, "size_length が 32 で受理されること");
        assert_eq!(cfg.index_length, 32, "index_length が 32 で受理されること");
        assert_eq!(
            cfg.index_delta_length, 32,
            "index_delta_length が 32 で受理されること"
        );
    }

    #[tokio::test]
    async fn run_rtsp_session_disconnects_after_requesting_audio_and_video() {
        let server = TestRtspServer::spawn(TestRtspServerOptions {
            require_basic_auth: false,
            with_audio: true,
            unsupported_video_codec: false,
            require_session_header: true,
        })
        .await
        .expect("must start test RTSP server");
        let parsed_url = parse_rtsp_input_url(&server.input_url).expect("must parse input URL");
        let root_stats = crate::stats::Stats::new();
        let stats = RtspSubscriberStats::new(root_stats.clone());
        let mut audio_track_tx = None;
        let mut video_decoder_input_tx = None;

        let mut output = RtspOutputContext {
            audio_track_tx: &mut audio_track_tx,
            audio_decoder: &mut None,
            video_decoder_input_tx: &mut video_decoder_input_tx,
        };
        let result = run_rtsp_session(
            &parsed_url,
            true,
            true,
            Duration::from_secs(3),
            &stats,
            &mut output,
        )
        .await;

        assert!(result.is_err(), "session should end with error: {result:?}");
        server.wait().await.expect("server must finish cleanly");

        let entries = root_stats.entries().expect("stats entries");
        assert!(!metric_flag(&entries, "is_connected"));
    }

    #[tokio::test]
    async fn run_rtsp_session_handles_basic_auth_challenge() {
        let server = TestRtspServer::spawn(TestRtspServerOptions {
            require_basic_auth: true,
            with_audio: false,
            unsupported_video_codec: false,
            require_session_header: true,
        })
        .await
        .expect("must start test RTSP server");
        let parsed_url = parse_rtsp_input_url(&server.input_url).expect("must parse input URL");
        let root_stats = crate::stats::Stats::new();
        let stats = RtspSubscriberStats::new(root_stats.clone());
        let mut audio_track_tx = None;
        let mut video_decoder_input_tx = None;

        let mut output = RtspOutputContext {
            audio_track_tx: &mut audio_track_tx,
            audio_decoder: &mut None,
            video_decoder_input_tx: &mut video_decoder_input_tx,
        };
        let result = run_rtsp_session(
            &parsed_url,
            false,
            true,
            Duration::from_secs(1),
            &stats,
            &mut output,
        )
        .await;

        assert!(result.is_err(), "session should end with error: {result:?}");
        server.wait().await.expect("server must finish cleanly");
    }

    #[tokio::test]
    async fn run_rtsp_session_fails_with_unsupported_video_codec() {
        let sdp_text = build_test_sdp(false, true);
        let sdp = Sdp::parse(&sdp_text).expect("must parse test SDP");
        let select_err = select_tracks(&sdp, "rtsp://127.0.0.1/live/", false, true)
            .expect_err("unsupported codec must be rejected by SDP selection");
        assert_eq!(
            select_err.display(),
            "failed to find supported H264 video track in SDP"
        );

        let server = TestRtspServer::spawn(TestRtspServerOptions {
            require_basic_auth: false,
            with_audio: false,
            unsupported_video_codec: true,
            require_session_header: false,
        })
        .await
        .expect("must start test RTSP server");
        let parsed_url = parse_rtsp_input_url(&server.input_url).expect("must parse input URL");
        let root_stats = crate::stats::Stats::new();
        let stats = RtspSubscriberStats::new(root_stats.clone());
        let mut audio_track_tx = None;
        let mut video_decoder_input_tx = None;

        let mut output = RtspOutputContext {
            audio_track_tx: &mut audio_track_tx,
            audio_decoder: &mut None,
            video_decoder_input_tx: &mut video_decoder_input_tx,
        };
        let result = run_rtsp_session(
            &parsed_url,
            false,
            true,
            Duration::ZERO,
            &stats,
            &mut output,
        )
        .await;

        assert!(
            result.is_err(),
            "session should fail for unsupported codec: {result:?}"
        );
        server.wait().await.expect("server must finish cleanly");
    }

    fn metric_flag(entries: &[crate::stats::StatsEntry], name: &str) -> bool {
        entries
            .iter()
            .find(|e| e.metric_name == name)
            .and_then(|e| e.value.as_flag())
            .expect("flag metric must exist")
    }

    #[derive(Debug, Clone, Copy)]
    struct TestRtspServerOptions {
        require_basic_auth: bool,
        with_audio: bool,
        unsupported_video_codec: bool,
        require_session_header: bool,
    }

    struct TestRtspServer {
        input_url: String,
        join_handle: tokio::task::JoinHandle<io::Result<()>>,
    }

    impl TestRtspServer {
        async fn spawn(options: TestRtspServerOptions) -> io::Result<Self> {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let local_addr = listener.local_addr()?;
            let input_url = if options.require_basic_auth {
                format!("rtsp://user:pass@127.0.0.1:{}/live", local_addr.port())
            } else {
                format!("rtsp://127.0.0.1:{}/live", local_addr.port())
            };
            let join_handle = tokio::spawn(async move {
                let (stream, _) = listener.accept().await?;
                run_test_rtsp_server(stream, options).await
            });
            Ok(Self {
                input_url,
                join_handle,
            })
        }

        async fn wait(self) -> io::Result<()> {
            self.join_handle
                .await
                .map_err(|e| io::Error::other(format!("join error: {e}")))?
        }
    }

    async fn run_test_rtsp_server(
        mut stream: TcpStream,
        options: TestRtspServerOptions,
    ) -> io::Result<()> {
        let mut read_buf = Vec::new();
        let mut auth_challenged = false;
        let mut video_rtp_channel = None;
        let mut audio_rtp_channel = None;
        let mut setup_count = 0usize;
        let session_id = "test-session";

        loop {
            let request = match read_rtsp_request(&mut stream, &mut read_buf).await {
                Ok(request) => request,
                Err(err)
                    if matches!(
                        err.kind(),
                        io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset
                    ) =>
                {
                    return Ok(());
                }
                Err(err) => return Err(err),
            };
            match request.method.as_str() {
                "OPTIONS" => {
                    write_rtsp_response(
                        &mut stream,
                        request.cseq,
                        200,
                        "OK",
                        &[("Public", "OPTIONS, DESCRIBE, SETUP, PLAY, GET_PARAMETER")],
                        None,
                    )
                    .await?;
                }
                "DESCRIBE" => {
                    if options.require_basic_auth
                        && !auth_challenged
                        && request
                            .headers
                            .get("authorization")
                            .is_none_or(|value| !value.starts_with("Basic "))
                    {
                        write_rtsp_response(
                            &mut stream,
                            request.cseq,
                            401,
                            "Unauthorized",
                            &[("WWW-Authenticate", "Basic realm=\"test\"")],
                            None,
                        )
                        .await?;
                        auth_challenged = true;
                        continue;
                    }

                    let sdp = build_test_sdp(options.with_audio, options.unsupported_video_codec);
                    write_rtsp_response(
                        &mut stream,
                        request.cseq,
                        200,
                        "OK",
                        &[
                            ("Content-Type", "application/sdp"),
                            ("Content-Base", "rtsp://127.0.0.1/live/"),
                        ],
                        Some(&sdp),
                    )
                    .await?;
                }
                "SETUP" => {
                    if options.require_session_header
                        && setup_count > 0
                        && request.headers.get("session").map(String::as_str) != Some(session_id)
                    {
                        write_rtsp_response(
                            &mut stream,
                            request.cseq,
                            454,
                            "Session Not Found",
                            &[],
                            None,
                        )
                        .await?;
                        return Ok(());
                    }

                    let transport = request.headers.get("transport").ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "missing transport header")
                    })?;
                    let (rtp_channel, rtcp_channel) = parse_interleaved_channels(transport)
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "invalid interleaved channel",
                            )
                        })?;
                    if request.uri.contains("trackID=0") {
                        video_rtp_channel = Some(rtp_channel);
                    } else if request.uri.contains("trackID=1") {
                        audio_rtp_channel = Some(rtp_channel);
                    }

                    let transport_response =
                        format!("RTP/AVP/TCP;unicast;interleaved={rtp_channel}-{rtcp_channel}");
                    write_rtsp_response(
                        &mut stream,
                        request.cseq,
                        200,
                        "OK",
                        &[("Transport", &transport_response), ("Session", session_id)],
                        None,
                    )
                    .await?;
                    setup_count += 1;
                }
                "PLAY" => {
                    if options.require_session_header
                        && request.headers.get("session").map(String::as_str) != Some(session_id)
                    {
                        write_rtsp_response(
                            &mut stream,
                            request.cseq,
                            454,
                            "Session Not Found",
                            &[],
                            None,
                        )
                        .await?;
                        return Ok(());
                    }

                    write_rtsp_response(
                        &mut stream,
                        request.cseq,
                        200,
                        "OK",
                        &[("Session", session_id)],
                        None,
                    )
                    .await?;

                    // PLAY レスポンス待機中の受信処理では RTP イベントを破棄するため、
                    // play_loop 開始後に届くよう少し待ってから RTP を送る。
                    tokio::time::sleep(Duration::from_millis(80)).await;
                    if let Some(channel) = video_rtp_channel {
                        send_test_video_rtp(&mut stream, channel, 90_000).await?;
                    }
                    if options.with_audio
                        && let Some(channel) = audio_rtp_channel
                    {
                        send_test_aac_rtp(&mut stream, channel, 48_000).await?;
                    }
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    return Ok(());
                }
                "GET_PARAMETER" => {
                    if options.require_session_header
                        && request.headers.get("session").map(String::as_str) != Some(session_id)
                    {
                        write_rtsp_response(
                            &mut stream,
                            request.cseq,
                            454,
                            "Session Not Found",
                            &[],
                            None,
                        )
                        .await?;
                        return Ok(());
                    }

                    write_rtsp_response(
                        &mut stream,
                        request.cseq,
                        200,
                        "OK",
                        &[("Session", session_id)],
                        None,
                    )
                    .await?;
                }
                _ => {
                    write_rtsp_response(&mut stream, request.cseq, 400, "Bad Request", &[], None)
                        .await?;
                    return Ok(());
                }
            }
        }
    }

    struct TestRtspRequest {
        method: String,
        uri: String,
        cseq: u32,
        headers: HashMap<String, String>,
    }

    async fn read_rtsp_request(
        stream: &mut TcpStream,
        read_buf: &mut Vec<u8>,
    ) -> io::Result<TestRtspRequest> {
        loop {
            if let Some(pos) = find_header_end(read_buf) {
                let header_bytes = read_buf.drain(..pos + 4).collect::<Vec<_>>();
                let header_text = std::str::from_utf8(&header_bytes).map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("invalid request header: {e}"),
                    )
                })?;
                let mut lines = header_text.split("\r\n");
                let request_line = lines.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "missing request line")
                })?;
                let mut request_parts = request_line.split_whitespace();
                let method = request_parts
                    .next()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing method"))?
                    .to_owned();
                let uri = request_parts
                    .next()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing uri"))?
                    .to_owned();

                let mut headers = HashMap::new();
                for line in lines {
                    if line.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
                    }
                }
                let cseq = headers
                    .get("cseq")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing cseq"))?
                    .parse::<u32>()
                    .map_err(|e| {
                        io::Error::new(io::ErrorKind::InvalidData, format!("invalid cseq: {e}"))
                    })?;
                return Ok(TestRtspRequest {
                    method,
                    uri,
                    cseq,
                    headers,
                });
            }

            let mut temp = [0u8; 4096];
            let n = stream.read(&mut temp).await?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "client closed connection",
                ));
            }
            read_buf.extend_from_slice(&temp[..n]);
        }
    }

    fn find_header_end(buf: &[u8]) -> Option<usize> {
        buf.windows(4).position(|w| w == b"\r\n\r\n")
    }

    async fn write_rtsp_response(
        stream: &mut TcpStream,
        cseq: u32,
        status_code: u16,
        reason: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> io::Result<()> {
        let body = body.unwrap_or("");
        let mut text = format!(
            "RTSP/1.0 {status_code} {reason}\r\nCSeq: {cseq}\r\nContent-Length: {}\r\n",
            body.len()
        );
        for (name, value) in headers {
            text.push_str(name);
            text.push_str(": ");
            text.push_str(value);
            text.push_str("\r\n");
        }
        text.push_str("\r\n");
        stream.write_all(text.as_bytes()).await?;
        if !body.is_empty() {
            stream.write_all(body.as_bytes()).await?;
        }
        stream.flush().await
    }

    fn parse_interleaved_channels(transport: &str) -> Option<(u8, u8)> {
        for part in transport.split(';') {
            let part = part.trim();
            if let Some(value) = part.strip_prefix("interleaved=") {
                let (a, b) = value.split_once('-')?;
                let rtp = a.parse::<u8>().ok()?;
                let rtcp = b.parse::<u8>().ok()?;
                return Some((rtp, rtcp));
            }
        }
        None
    }

    fn build_test_sdp(with_audio: bool, unsupported_video_codec: bool) -> String {
        let video_encoding = if unsupported_video_codec {
            "VP8"
        } else {
            "H264"
        };
        let mut sdp = format!(
            "v=0\r\n\
             o=- 0 0 IN IP4 127.0.0.1\r\n\
             s=hisui-test\r\n\
             t=0 0\r\n\
             a=control:*\r\n\
             m=video 9000 RTP/AVP 96\r\n\
             a=rtpmap:96 {video_encoding}/90000\r\n\
             a=control:trackID=0\r\n"
        );
        if with_audio {
            sdp.push_str(
                "m=audio 9002 RTP/AVP 97\r\n\
                 a=rtpmap:97 MPEG4-GENERIC/48000/2\r\n\
                 a=fmtp:97 profile-level-id=1;mode=AAC-hbr;sizelength=13;indexlength=3;indexdeltalength=3;config=1190\r\n\
                 a=control:trackID=1\r\n",
            );
        }
        sdp
    }

    async fn send_test_video_rtp(
        stream: &mut TcpStream,
        channel: u8,
        timestamp: u32,
    ) -> io::Result<()> {
        let mut header = RtpHeader::new(96, 1, timestamp, 0x01020304);
        header.marker = true;
        // このテストは RTSP セッション経路の疎通確認が目的なので、最小の単一 NAL だけ送る。
        // FU-A/STAP-A の分割・集約ロジック自体は depacketize_h264_fu_a などの単体テストで検証する。
        let packet = RtpPacket::new(header, vec![0x65, 0x88, 0x84]);
        let bytes = encode_interleaved_frame(channel, &packet.build());
        stream.write_all(&bytes).await
    }

    async fn send_test_aac_rtp(
        stream: &mut TcpStream,
        channel: u8,
        timestamp: u32,
    ) -> io::Result<()> {
        let mut header = RtpHeader::new(97, 1, timestamp, 0x0A0B0C0D);
        header.marker = true;
        let payload = vec![0x00, 0x10, 0x00, 0x10, 0x11, 0x22];
        let packet = RtpPacket::new(header, payload);
        let bytes = encode_interleaved_frame(channel, &packet.build());
        stream.write_all(&bytes).await
    }

    // 映像 sample_entry テスト用の Annex-B バイト列フィクスチャ。
    // NAL header: 0x67 = SPS、0x68 = PPS、0x65 = IDR、0x41 = 非 IDR、0x85 = forbidden_zero_bit セット。
    // SPS は `crate::video::h264::tests` で集約管理された実機 SPS の Annex-B 形式を参照する
    // (短い偽 SPS では parse_sps がビット切れで Err を返すため、parse_sps を完走できる実 SPS が必要)。
    fn sps_initial() -> &'static [u8] {
        &crate::video::h264::tests::SPS_320X240_ANNEXB
    }
    fn sps_updated() -> &'static [u8] {
        &crate::video::h264::tests::SPS_1920X1080_ANNEXB
    }
    const PPS: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x68, 0xce, 0x06, 0xe2];
    const IDR: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x21];
    const P_FRAME: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x41, 0x9a, 0x21, 0x6c];
    const BROKEN_NAL: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x85, 0x00, 0x01];

    // Annex-B バイト列から start code prefix 4 バイトを除いた素の NAL バイト列を返す。
    fn nal_payload(annexb: &[u8]) -> &[u8] {
        debug_assert!(
            annexb.len() >= 4,
            "annexb は最低 4 バイトの start code を持つこと"
        );
        &annexb[4..]
    }

    // Annex-B バイト列の配列を Base64 化して `,` 区切りで連結した sprop-parameter-sets 値を作る。
    fn sprop_value_from(parts: &[&[u8]]) -> String {
        assert!(
            !parts.is_empty(),
            "テストヘルパは少なくとも 1 つの NAL を要求する"
        );
        parts
            .iter()
            .map(|nal| Base64::encode_string(nal_payload(nal)))
            .collect::<Vec<_>>()
            .join(",")
    }

    // テスト用の VideoRtpReceiver を最小値で構築する。
    fn build_test_video_receiver() -> VideoRtpReceiver {
        VideoRtpReceiver {
            rtp_channel: 0,
            payload_type: 96,
            timestamp_mapper: TimestampMapper::new(32, 90_000, Duration::ZERO)
                .expect("テスト用の TimestampMapper が構築できること"),
            depacketizer: H264RtpDepacketizer::new(),
            last_sample_entry: None,
        }
    }

    // テスト用の DepacketizedVideoFrame を構築する。
    fn build_test_depacketized_frame(data: Vec<u8>) -> DepacketizedVideoFrame {
        DepacketizedVideoFrame {
            rtp_timestamp: 0,
            keyframe: false,
            data,
        }
    }

    // 既存 `build_test_sdp(false, false)` ベースで `a=fmtp:96 {fmtp_params}` を
    // `a=control:trackID=0` の直前に挿入した SDP テキストを返す。
    fn build_test_sdp_with_fmtp(fmtp_params: &str) -> String {
        format!(
            "v=0\r\n\
             o=- 0 0 IN IP4 127.0.0.1\r\n\
             s=hisui-test\r\n\
             t=0 0\r\n\
             a=control:*\r\n\
             m=video 9000 RTP/AVP 96\r\n\
             a=rtpmap:96 H264/90000\r\n\
             a=fmtp:96 {fmtp_params}\r\n\
             a=control:trackID=0\r\n"
        )
    }

    // MPEG4-GENERIC (AAC) の audio メディアのみを含む SDP テキストを返す。
    // `a=fmtp:97 {fmtp_params}` で fmtp パラメータを差し替えられる。
    fn build_test_sdp_with_audio_fmtp(fmtp_params: &str) -> String {
        format!(
            "v=0\r\n\
             o=- 0 0 IN IP4 127.0.0.1\r\n\
             s=hisui-test\r\n\
             t=0 0\r\n\
             a=control:*\r\n\
             m=audio 9002 RTP/AVP 97\r\n\
             a=rtpmap:97 MPEG4-GENERIC/48000/2\r\n\
             a=fmtp:97 {fmtp_params}\r\n\
             a=control:trackID=1\r\n"
        )
    }

    // SDP テキストから音声メディアを取り出して `select_audio_track` を呼ぶ薄いラッパ。
    fn parse_audio_track(sdp_text: &str) -> crate::Result<Option<AudioTrackConfig>> {
        let parsed = Sdp::parse(sdp_text).expect("テスト用 SDP がパースできること");
        let media = parsed
            .media
            .iter()
            .find(|m| m.media_type.eq_ignore_ascii_case("audio"))
            .expect("audio メディアが SDP に存在すること");
        select_audio_track(media, "rtsp://example.com/")
    }

    // SDP テキストから映像メディアを取り出して `select_video_track` を呼ぶ薄いラッパ。
    fn parse_video_track(sdp_text: &str) -> crate::Result<Option<VideoTrackConfig>> {
        let parsed = Sdp::parse(sdp_text).expect("テスト用 SDP がパースできること");
        let media = parsed
            .media
            .iter()
            .find(|m| m.media_type.eq_ignore_ascii_case("video"))
            .expect("video メディアが SDP に存在すること");
        select_video_track(media, "rtsp://example.com/")
    }

    // SPS + PPS + IDR を Annex-B で連結した frame.data を返す。
    fn concat_sps_pps_idr(sps: &[u8]) -> Vec<u8> {
        let mut data = sps.to_vec();
        data.extend_from_slice(PPS);
        data.extend_from_slice(IDR);
        data
    }

    #[test]
    fn select_video_track_extracts_sample_entry_from_sprop() {
        // sprop-parameter-sets に SPS + PPS を Base64 で連結して渡すと、
        // `VideoTrackConfig.sample_entry` に Avc1 SampleEntry が入る。
        let sprop = sprop_value_from(&[sps_initial(), PPS]);
        let sdp = build_test_sdp_with_fmtp(&format!("sprop-parameter-sets={sprop}"));
        let cfg = parse_video_track(&sdp)
            .expect("正常な sprop-parameter-sets は Ok を返すこと")
            .expect("VideoTrackConfig が返ること");
        let entry = cfg
            .sample_entry
            .expect("sprop-parameter-sets 由来で sample_entry が Some になること");
        match entry {
            SampleEntry::Avc1(avc1) => {
                // avcc_box.sps_list / pps_list が Base64 デコード後の素の NAL バイト列と一致する。
                assert_eq!(
                    avc1.avcc_box.sps_list,
                    vec![nal_payload(sps_initial()).to_vec()],
                    "SPS リストが期待値と一致すること"
                );
                assert_eq!(
                    avc1.avcc_box.pps_list,
                    vec![nal_payload(PPS).to_vec()],
                    "PPS リストが期待値と一致すること"
                );
            }
            other => panic!("Avc1 SampleEntry を期待したが {other:?} が返った"),
        }
    }

    #[test]
    fn select_video_track_returns_none_when_fmtp_missing() {
        // fmtp 不在は Err にせず sample_entry: None を返す。
        let sdp = build_test_sdp(false, false);
        let cfg = parse_video_track(&sdp)
            .expect("fmtp 不在は Ok を返すこと")
            .expect("VideoTrackConfig が返ること");
        assert!(
            cfg.sample_entry.is_none(),
            "fmtp 不在では sample_entry が None になること"
        );
    }

    #[test]
    fn select_video_track_returns_none_when_sprop_missing_in_fmtp() {
        // fmtp 自体は有るが sprop-parameter-sets を含まないケース。Err にせず sample_entry: None を返す。
        let sdp = build_test_sdp_with_fmtp("profile-level-id=42c01e;packetization-mode=1");
        let cfg = parse_video_track(&sdp)
            .expect("sprop-parameter-sets 不在は Ok を返すこと")
            .expect("VideoTrackConfig が返ること");
        assert!(
            cfg.sample_entry.is_none(),
            "sprop-parameter-sets 不在では sample_entry が None になること"
        );
    }

    #[test]
    fn select_video_track_returns_none_when_sprop_empty() {
        // sprop-parameter-sets= で値が空文字列のケース。Err にせず inline 経路に委ねる。
        let sdp = build_test_sdp_with_fmtp("sprop-parameter-sets=");
        let cfg = parse_video_track(&sdp)
            .expect("空文字列 sprop は Ok を返すこと")
            .expect("VideoTrackConfig が返ること");
        assert!(
            cfg.sample_entry.is_none(),
            "空文字列 sprop では sample_entry が None になること"
        );
    }

    #[test]
    fn select_video_track_returns_none_when_sprop_has_only_empty_entries() {
        // 空要素のみの sprop（カンマ区切りで非空要素なし）は Err にせず None を返す。
        let sdp = build_test_sdp_with_fmtp("sprop-parameter-sets=,,");
        let cfg = parse_video_track(&sdp)
            .expect("空要素のみの sprop は Ok を返すこと")
            .expect("VideoTrackConfig が返ること");
        assert!(
            cfg.sample_entry.is_none(),
            "空要素のみの sprop では sample_entry が None になること"
        );
    }

    #[test]
    fn select_video_track_returns_err_on_invalid_base64() {
        // sprop-parameter-sets に Base64 アルファベット外の文字を含む場合は Err を伝播する。
        let sdp = build_test_sdp_with_fmtp("sprop-parameter-sets=!!!");
        let err = parse_video_track(&sdp).expect_err("不正な Base64 では Err を返すこと");
        let display = format!("{err:?}");
        assert!(
            display.contains("invalid sprop-parameter-sets base64"),
            "エラーメッセージに `invalid sprop-parameter-sets base64` が含まれること（実際: {display}）"
        );
    }

    #[test]
    fn select_video_track_returns_none_when_sprop_has_only_sps() {
        // SPS のみ含む sprop は不完全な補助メタデータとして許容し、inline 経路に委ねる。
        let sprop = sprop_value_from(&[sps_initial()]);
        let sdp = build_test_sdp_with_fmtp(&format!("sprop-parameter-sets={sprop}"));
        let cfg = parse_video_track(&sdp)
            .expect("PPS 不在 sprop は Err にせず Ok を返すこと")
            .expect("VideoTrackConfig が返ること");
        assert!(
            cfg.sample_entry.is_none(),
            "PPS 不在 sprop では sample_entry が None になること"
        );
    }

    #[test]
    fn select_video_track_returns_none_when_sprop_has_only_pps() {
        // PPS のみ含む sprop は不完全な補助メタデータとして許容し、inline 経路に委ねる。
        let sprop = sprop_value_from(&[PPS]);
        let sdp = build_test_sdp_with_fmtp(&format!("sprop-parameter-sets={sprop}"));
        let cfg = parse_video_track(&sdp)
            .expect("SPS 不在 sprop は Err にせず Ok を返すこと")
            .expect("VideoTrackConfig が返ること");
        assert!(
            cfg.sample_entry.is_none(),
            "SPS 不在 sprop では sample_entry が None になること"
        );
    }

    #[test]
    fn apply_sample_entry_emits_sample_entry_for_sps_pps_idr_frame() {
        // `last_sample_entry: None` 初期状態で SPS + PPS + IDR の 3 条件揃った frame を投入すると
        // sample_entry が確定して Some になる。
        let mut receiver = build_test_video_receiver();
        let frame = build_test_depacketized_frame(concat_sps_pps_idr(sps_initial()));
        receiver
            .apply_sample_entry(&frame)
            .expect("3 条件揃った frame では Ok を返すこと");
        assert!(
            receiver.last_sample_entry.is_some(),
            "sample_entry が確定して Some になること"
        );
    }

    #[test]
    fn apply_sample_entry_keeps_initial_sample_entry_for_mid_stream_idr_without_sps_pps() {
        // SDP `sprop-parameter-sets` 由来で確定済みの一般的 RTSP カメラの mid-stream IDR
        // （SPS / PPS を inline しない）を模擬する。更新スキップで Arc が維持される。
        let mut receiver = build_test_video_receiver();
        receiver
            .apply_sample_entry(&build_test_depacketized_frame(concat_sps_pps_idr(
                sps_initial(),
            )))
            .expect("初期確定は Ok を返すこと");
        let initial = receiver
            .last_sample_entry
            .clone()
            .expect("初期 sample_entry が確定していること");

        receiver
            .apply_sample_entry(&build_test_depacketized_frame(IDR.to_vec()))
            .expect("SPS / PPS 不在 IDR でも Ok を返すこと（fail-fast にしない）");

        let after = receiver
            .last_sample_entry
            .as_ref()
            .expect("sample_entry が消えていないこと");
        assert!(
            after.ptr_eq(&initial),
            "同一 Arc を共有していること（更新試行をスキップしたため新規 new されないこと）"
        );
    }

    #[test]
    fn apply_sample_entry_keeps_initial_sample_entry_for_mid_stream_idr_without_pps() {
        // mid-stream IDR が SPS のみ inline して PPS を欠くケース。
        // `has_sps && has_pps` が成立しないため 3 条件判定の更新分岐に入らず Arc 維持。
        let mut receiver = build_test_video_receiver();
        receiver
            .apply_sample_entry(&build_test_depacketized_frame(concat_sps_pps_idr(
                sps_initial(),
            )))
            .expect("初期確定は Ok を返すこと");
        let initial = receiver
            .last_sample_entry
            .clone()
            .expect("初期 sample_entry が確定していること");

        let mut sps_idr = sps_initial().to_vec();
        sps_idr.extend_from_slice(IDR);
        receiver
            .apply_sample_entry(&build_test_depacketized_frame(sps_idr))
            .expect("PPS 不在 IDR でも Ok を返すこと（fail-fast にしない）");

        let after = receiver
            .last_sample_entry
            .as_ref()
            .expect("sample_entry が消えていないこと");
        assert!(after.ptr_eq(&initial), "同一 Arc が維持されていること");
    }

    #[test]
    fn apply_sample_entry_keeps_initial_sample_entry_for_mid_stream_idr_without_sps() {
        // mid-stream IDR が PPS のみ inline して SPS を欠くケース。
        // `has_sps && has_pps` が成立しないため 3 条件判定の更新分岐に入らず Arc 維持。
        let mut receiver = build_test_video_receiver();
        receiver
            .apply_sample_entry(&build_test_depacketized_frame(concat_sps_pps_idr(
                sps_initial(),
            )))
            .expect("初期確定は Ok を返すこと");
        let initial = receiver
            .last_sample_entry
            .clone()
            .expect("初期 sample_entry が確定していること");

        let mut pps_idr = PPS.to_vec();
        pps_idr.extend_from_slice(IDR);
        receiver
            .apply_sample_entry(&build_test_depacketized_frame(pps_idr))
            .expect("SPS 不在 IDR でも Ok を返すこと（fail-fast にしない）");

        let after = receiver
            .last_sample_entry
            .as_ref()
            .expect("sample_entry が消えていないこと");
        assert!(after.ptr_eq(&initial), "同一 Arc が維持されていること");
    }

    #[test]
    fn apply_sample_entry_updates_sample_entry_on_mid_stream_sps_change() {
        // mid-stream で SPS の内容が変わった 3 条件揃いの IDR が来たら新値で上書きする。
        let mut receiver = build_test_video_receiver();
        receiver
            .apply_sample_entry(&build_test_depacketized_frame(concat_sps_pps_idr(
                sps_initial(),
            )))
            .expect("初期確定は Ok を返すこと");
        let initial = receiver
            .last_sample_entry
            .clone()
            .expect("初期 sample_entry が確定していること");

        receiver
            .apply_sample_entry(&build_test_depacketized_frame(concat_sps_pps_idr(
                sps_updated(),
            )))
            .expect("SPS 更新時も Ok を返すこと");

        let after = receiver
            .last_sample_entry
            .as_ref()
            .expect("sample_entry が確定していること");
        assert!(
            after.changed_since(Some(&initial)),
            "値が変化していること（changed_since が true を返すこと）"
        );
        assert!(
            !after.ptr_eq(&initial),
            "別 Arc であること（無条件上書きで新 Arc になること）"
        );
    }

    #[test]
    fn apply_sample_entry_skips_update_for_p_frame_only() {
        // P フレームのみは `has_idr=false` で更新分岐に入らない。
        // `last_sample_entry: None` 初期状態のままで Ok を返す。
        let mut receiver = build_test_video_receiver();
        receiver
            .apply_sample_entry(&build_test_depacketized_frame(P_FRAME.to_vec()))
            .expect("P フレームのみでは Ok を返すこと");
        assert!(
            receiver.last_sample_entry.is_none(),
            "last_sample_entry は None のまま変化しないこと"
        );
    }

    #[test]
    fn apply_sample_entry_returns_err_on_broken_nal() {
        // forbidden_zero_bit が立った破損 NAL を含む frame は Err を返す。
        let mut receiver = build_test_video_receiver();
        let err = receiver
            .apply_sample_entry(&build_test_depacketized_frame(BROKEN_NAL.to_vec()))
            .expect_err("forbidden_zero_bit が立った NAL では Err を返すこと");
        let display = format!("{err:?}");
        assert!(
            display.contains("forbidden_zero_bit"),
            "エラーメッセージに `forbidden_zero_bit` が含まれること（実際: {display}）"
        );
    }

    #[test]
    fn select_video_track_returns_err_on_sprop_with_broken_nal() {
        // sprop-parameter-sets に forbidden_zero_bit が立った NAL を含めると Err を返して
        // 接続を打ち切る。apply_sample_entry 経路 (H264AnnexBNalUnits 経由) と対称の挙動。
        let sprop = sprop_value_from(&[sps_initial(), PPS, BROKEN_NAL]);
        let sdp = build_test_sdp_with_fmtp(&format!("sprop-parameter-sets={sprop}"));
        let err = parse_video_track(&sdp)
            .expect_err("forbidden_zero_bit が立った NAL を含む sprop は Err を返すこと");
        let display = format!("{err:?}");
        assert!(
            display.contains("forbidden_zero_bit"),
            "エラーメッセージに `forbidden_zero_bit` が含まれること（実際: {display}）"
        );
    }

    /// spawn_video_decoder_task 直後の shutdown().await が Ok(()) を返す smoke test。
    /// Eos 受信 → Initial の handle_input_sample_sync(None) → poll_output_sync が Finished →
    /// output_tx.send_eos() → task が Ok(()) で return する経路を検証する。
    /// pipeline closed / panic 経路は残懸念 §2 に従い workspace の cargo test で担保する。
    #[tokio::test]
    async fn spawn_then_shutdown_returns_ok() -> crate::Result<()> {
        let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())?;
        let pipeline_handle = pipeline.handle();
        let _pipeline_task = tokio::spawn(async move { pipeline.run().await });

        let processor_handle = pipeline_handle
            .register_processor(
                crate::ProcessorId::new("rtsp_task_smoke_test"),
                crate::ProcessorMetadata::new("rtsp_task_smoke_test"),
            )
            .await
            .expect("register processor");
        let track_id = crate::TrackId::new("rtsp_task_smoke_test_video");
        let output_tx = processor_handle.publish_track(track_id).await?;

        let task = spawn_video_decoder_task(
            crate::decoder::VideoDecoderOptions::default(),
            processor_handle.stats(),
            output_tx,
        );

        task.shutdown().await
    }
}
