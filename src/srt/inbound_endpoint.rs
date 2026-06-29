use std::collections::HashMap;
use std::io::{self, Read};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mpeg2ts::es::{StreamId, StreamType};
use mpeg2ts::pes::PesHeader;
use mpeg2ts::ts::{Pid, ReadTsPacket, TsPacket, TsPacketReader, TsPayload};
use shiguredo_http11::uri::Uri;
use shiguredo_srt::{
    ConnectionEvent, ConnectionOptions, ConnectionOutput, ConnectionState, KeyLength,
    SrtConnection, TimerId, Timestamp,
};
use tokio::net::UdpSocket;
use tokio::time::Instant;

const TS_PACKET_SIZE: usize = 188;

/// SRT Inbound Endpoint
///
/// フィールドの不変条件は `Self::new()` で eager 検証される。
pub struct SrtInboundEndpoint {
    pub(crate) input_url: String,
    pub(crate) output_audio_track_id: Option<crate::TrackId>,
    pub(crate) output_video_track_id: Option<crate::TrackId>,
    pub(crate) options: SrtInboundEndpointOptions,
}

/// `SrtInboundEndpoint` 用オプション群
#[derive(Debug, Clone, Default)]
pub struct SrtInboundEndpointOptions {
    /// SRT caller が送る streamid の期待値（省略時は検証しない）。
    pub stream_id: Option<String>,
    /// SRT 暗号化（KM ハンドシェイク）を有効化するパスフレーズ。
    pub passphrase: Option<String>,
    /// SRT 暗号化の鍵長（passphrase 指定時のみ有効）。
    pub key_length: Option<KeyLength>,
    /// TSBPD 遅延。
    pub tsbpd_delay_ms: Option<Duration>,
}

/// `SrtInboundEndpoint::new()` が返す検証エラー。
#[derive(Debug)]
pub enum SrtInboundEndpointBuildError {
    EmptyInputUrl,
    EmptyStreamId,
    EmptyPassphrase,
    NoTrackId,
}

impl std::fmt::Display for SrtInboundEndpointBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyInputUrl => write!(f, "input_url must not be empty"),
            Self::EmptyStreamId => {
                write!(f, "stream_id must not be empty when specified")
            }
            Self::EmptyPassphrase => {
                write!(f, "passphrase must not be empty when specified")
            }
            Self::NoTrackId => write!(
                f,
                "at least one of output_audio_track_id / output_video_track_id must be set"
            ),
        }
    }
}

#[derive(Debug, Clone)]
struct SrtInboundEndpointStats {
    is_listening_metric: crate::stats::StatsFlag,
    is_connected_metric: crate::stats::StatsFlag,
    audio_codec_metric: crate::stats::StatsString,
    total_input_audio_data_count_metric: crate::stats::StatsCounter,
    last_input_audio_timestamp_metric: crate::stats::StatsDuration,
    video_codec_metric: crate::stats::StatsString,
    total_input_video_frame_count_metric: crate::stats::StatsCounter,
    last_input_video_timestamp_metric: crate::stats::StatsDuration,
}

impl SrtInboundEndpointStats {
    fn new(mut stats: crate::stats::Stats) -> Self {
        Self {
            is_listening_metric: stats.flag("is_listening"),
            is_connected_metric: stats.flag("is_connected"),
            audio_codec_metric: stats.string("audio_codec"),
            total_input_audio_data_count_metric: stats.counter("total_input_audio_data_count"),
            last_input_audio_timestamp_metric: stats.duration("last_input_audio_timestamp"),
            video_codec_metric: stats.string("video_codec"),
            total_input_video_frame_count_metric: stats.counter("total_input_video_frame_count"),
            last_input_video_timestamp_metric: stats.duration("last_input_video_timestamp"),
        }
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

    fn set_listening(&self, value: bool) {
        self.is_listening_metric.set(value);
    }

    fn set_connected(&self, value: bool) {
        self.is_connected_metric.set(value);
    }
}

#[derive(Debug, Clone)]
struct ParsedSrtUrl {
    host: String,
    port: u16,
}

#[derive(Debug, Clone)]
struct SrtEndpointConfig {
    stream_id: Option<String>,
    passphrase: Option<String>,
    key_length: KeyLength,
    tsbpd_delay: u16,
}

struct SrtConnectionContext<'a> {
    peer_addr: &'a mut Option<SocketAddr>,
    demuxer: &'a mut SrtTsDemuxer,
    connection_timestamp_offset: &'a mut Duration,
}

#[derive(Debug)]
struct PendingPesPacket {
    header: PesHeader,
    data: Vec<u8>,
    expected_data_len: Option<usize>,
}

#[derive(Debug)]
enum TsSample {
    Audio(crate::AudioFrame),
    Video(crate::VideoFrame),
}

impl SrtInboundEndpoint {
    /// Start the SRT Inbound Endpoint
    pub async fn run(self, handle: crate::ProcessorHandle) -> crate::Result<()> {
        let parsed_url = parse_srt_url(&self.input_url)
            .map_err(|e| crate::Error::new(format!("invalid input_url: {e}")))?;
        let endpoint_config = self.endpoint_config()?;

        let bind_addr: SocketAddr = format!("{}:{}", parsed_url.host, parsed_url.port)
            .parse()
            .map_err(|e| crate::Error::new(format!("invalid bind address: {e}")))?;
        tracing::debug!("Starting SRT inbound endpoint on {bind_addr}");

        let socket = UdpSocket::bind(bind_addr).await?;
        let mut recv_buf = vec![0u8; 64 * 1024];

        let mut conn = create_listener_connection(&endpoint_config)?;
        let mut peer_addr: Option<SocketAddr> = None;
        let mut timers: HashMap<TimerId, Instant> = HashMap::new();
        let base_time = Instant::now();
        let mut connection_timestamp_offset = Duration::ZERO;

        let mut demuxer = SrtTsDemuxer::new()?;

        let mut video_track_tx = if let Some(track_id) = &self.output_video_track_id {
            Some(handle.publish_track(track_id.clone()).await?)
        } else {
            None
        };
        let mut audio_track_tx = if let Some(track_id) = &self.output_audio_track_id {
            Some(handle.publish_track(track_id.clone()).await?)
        } else {
            None
        };

        let stats = SrtInboundEndpointStats::new(handle.stats());
        stats.set_listening(true);
        stats.set_connected(false);

        // デコーダーを生成する
        let mut video_decoder = if self.output_video_track_id.is_some() {
            let mut decoder_stats = handle.stats();
            decoder_stats.set_default_label("component", "video_decoder");
            Some(crate::decoder::VideoDecoder::new(
                crate::decoder::VideoDecoderOptions {
                    openh264_lib: handle.config().openh264_lib.clone(),
                    ..Default::default()
                },
                decoder_stats,
            ))
        } else {
            None
        };
        let mut audio_decoder = if self.output_audio_track_id.is_some() {
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

        let mut process_polled_events =
            |conn: &mut SrtConnection, peer_addr: &mut Option<SocketAddr>| -> crate::Result<()> {
                while let Some(event) = conn.poll_event() {
                    if let ConnectionEvent::DataReceived { payload, .. } = &event {
                        let samples = demuxer
                            .push_payload(payload)
                            .map_err(|e| e.with_context("failed to parse MPEG-TS payload"))?;
                        publish_samples(
                            samples,
                            &mut audio_track_tx,
                            &mut video_track_tx,
                            &mut audio_decoder,
                            &mut video_decoder,
                            &stats,
                            connection_timestamp_offset,
                        )?;
                    }
                    if should_flush_pending_pes(&event) {
                        let flushed_samples = demuxer.flush_pending()?;
                        publish_samples(
                            flushed_samples,
                            &mut audio_track_tx,
                            &mut video_track_tx,
                            &mut audio_decoder,
                            &mut video_decoder,
                            &stats,
                            connection_timestamp_offset,
                        )?;
                    }
                    let now = now_timestamp(base_time);
                    let mut connection_ctx = SrtConnectionContext {
                        peer_addr,
                        demuxer: &mut demuxer,
                        connection_timestamp_offset: &mut connection_timestamp_offset,
                    };
                    self.handle_connection_event(
                        event,
                        now,
                        conn,
                        &endpoint_config,
                        &mut connection_ctx,
                        &stats,
                    )?;
                }
                Ok(())
            };

        loop {
            process_polled_events(&mut conn, &mut peer_addr)?;

            while let Some(output) = conn.poll_output() {
                handle_connection_output(output, &socket, peer_addr, &mut timers).await?;
            }

            let next_timer = timers
                .iter()
                .min_by_key(|(_, deadline)| **deadline)
                .map(|(timer_id, deadline)| (*timer_id, *deadline));

            let timeout_duration = next_timer
                .map(|(_, deadline)| {
                    deadline
                        .checked_duration_since(Instant::now())
                        .unwrap_or(Duration::ZERO)
                })
                .unwrap_or(Duration::from_secs(60));

            tokio::select! {
                recv_result = socket.recv_from(&mut recv_buf) => {
                    let (len, addr) = recv_result?;
                    if !accept_peer_packet(conn.state(), peer_addr, addr) {
                        continue;
                    }

                    if peer_addr.is_none() {
                        peer_addr = Some(addr);
                        tracing::debug!("SRT peer connected from {addr}");
                    }

                    let now = now_timestamp(base_time);
                    conn.feed_recv_buf(&recv_buf[..len], now)
                        .map_err(|e| crate::Error::new(format!("failed to process SRT packet: {e}")))?;

                    process_polled_events(&mut conn, &mut peer_addr)?;
                }
                _ = tokio::time::sleep(timeout_duration), if next_timer.is_some() => {
                    let (timer_id, _) = next_timer.expect("infallible");
                    timers.remove(&timer_id);
                    let now = now_timestamp(base_time);
                    conn.handle_timer(timer_id, now)
                        .map_err(|e| crate::Error::new(format!("failed to handle SRT timer: {e}")))?;
                }
            }
        }
    }

    fn endpoint_config(&self) -> crate::Result<SrtEndpointConfig> {
        if self.options.passphrase.is_none() && self.options.key_length.is_some() {
            return Err(crate::Error::new(
                "key_length requires passphrase to be specified",
            ));
        }

        Ok(SrtEndpointConfig {
            stream_id: self.options.stream_id.clone(),
            passphrase: self.options.passphrase.clone(),
            key_length: self.options.key_length.unwrap_or(KeyLength::Aes128),
            tsbpd_delay: self
                .options
                .tsbpd_delay_ms
                .map(tsbpd_delay_duration_to_millis)
                .transpose()?
                .unwrap_or(120),
        })
    }

    /// `SrtInboundEndpoint` を構築する。
    pub fn new(
        input_url: String,
        output_audio_track_id: Option<crate::TrackId>,
        output_video_track_id: Option<crate::TrackId>,
        options: SrtInboundEndpointOptions,
    ) -> Result<Self, SrtInboundEndpointBuildError> {
        if input_url.is_empty() {
            return Err(SrtInboundEndpointBuildError::EmptyInputUrl);
        }
        if let Some(id) = &options.stream_id
            && id.is_empty()
        {
            return Err(SrtInboundEndpointBuildError::EmptyStreamId);
        }
        if let Some(pass) = &options.passphrase
            && pass.is_empty()
        {
            return Err(SrtInboundEndpointBuildError::EmptyPassphrase);
        }
        if output_audio_track_id.is_none() && output_video_track_id.is_none() {
            return Err(SrtInboundEndpointBuildError::NoTrackId);
        }
        Ok(Self {
            input_url,
            output_audio_track_id,
            output_video_track_id,
            options,
        })
    }

    fn handle_connection_event(
        &self,
        event: ConnectionEvent,
        now: Timestamp,
        conn: &mut SrtConnection,
        endpoint_config: &SrtEndpointConfig,
        connection_ctx: &mut SrtConnectionContext<'_>,
        stats: &SrtInboundEndpointStats,
    ) -> crate::Result<()> {
        match event {
            ConnectionEvent::Connected => {
                *connection_ctx.connection_timestamp_offset =
                    Duration::from_micros(now.as_micros());
                stats.set_connected(true);
                if let Some(expected_stream_id) = &endpoint_config.stream_id {
                    let actual_stream_id = conn.peer_stream_id();
                    if actual_stream_id != Some(expected_stream_id.as_str()) {
                        tracing::warn!(
                            "SRT peer stream id mismatch: expected={expected_stream_id}, actual={actual_stream_id:?}"
                        );
                        conn.disconnect(now);
                    }
                }
                tracing::debug!("SRT connection established");
            }
            ConnectionEvent::StateChanged(state) => {
                tracing::debug!("SRT state changed: {state:?}");
                if state == ConnectionState::Disconnected {
                    stats.set_connected(false);
                    reset_connection_state(conn, endpoint_config, connection_ctx)?;
                }
            }
            ConnectionEvent::Disconnected { reason } => {
                tracing::warn!("SRT disconnected: {reason}");
                stats.set_connected(false);
                reset_connection_state(conn, endpoint_config, connection_ctx)?;
            }
            ConnectionEvent::Error(reason) => {
                tracing::warn!("SRT connection error: {reason}");
            }
            ConnectionEvent::DataReceived { .. } => {}
            ConnectionEvent::KeyRefreshNeeded { .. } => {
                tracing::debug!("SRT key refresh requested");
            }
        }
        Ok(())
    }
}

fn tsbpd_delay_duration_to_millis(duration: Duration) -> crate::Result<u16> {
    let millis = duration.as_millis();
    u16::try_from(millis)
        .map_err(|_| crate::Error::new(format!("tsbpd_delay_ms must be <= {}", u16::MAX)))
}

fn parse_srt_url(input_url: &str) -> std::result::Result<ParsedSrtUrl, String> {
    let uri = Uri::parse(input_url).map_err(|e| format!("failed to parse url: {e}"))?;
    if uri.scheme() != Some("srt") {
        return Err("scheme must be srt".to_owned());
    }

    let host = uri
        .host()
        .ok_or_else(|| "host is required".to_owned())?
        .to_owned();
    let port = uri.port().ok_or_else(|| "port is required".to_owned())?;

    // Hisui は listener 固定実装のため、query の mode は検証しない。
    Ok(ParsedSrtUrl { host, port })
}

fn create_listener_connection(endpoint_config: &SrtEndpointConfig) -> crate::Result<SrtConnection> {
    let options = ConnectionOptions {
        socket_id: pseudo_random_u32()? & 0x7FFF_FFFF,
        initial_seq: Some(pseudo_random_u32()? & 0x7FFF_FFFF),
        syn_cookie: Some(pseudo_random_u32()?),
        passphrase: endpoint_config.passphrase.clone(),
        key_length: endpoint_config.key_length,
        tsbpd_delay: endpoint_config.tsbpd_delay,
        stream_id: endpoint_config.stream_id.clone(),
        ..Default::default()
    };
    Ok(SrtConnection::new_listener(options))
}

fn pseudo_random_u32() -> crate::Result<u32> {
    let mut bytes = [0u8; 4];
    aws_lc_rs::rand::fill(&mut bytes)
        .map_err(|_| crate::Error::new("failed to generate random bytes with aws-lc-rs"))?;
    Ok(u32::from_le_bytes(bytes))
}

fn publish_samples(
    samples: Vec<TsSample>,
    audio_track_tx: &mut Option<crate::media_pipeline::TrackPublisher>,
    video_track_tx: &mut Option<crate::media_pipeline::TrackPublisher>,
    audio_decoder: &mut Option<crate::decoder::AudioDecoder>,
    video_decoder: &mut Option<crate::decoder::VideoDecoder>,
    stats: &SrtInboundEndpointStats,
    connection_timestamp_offset: Duration,
) -> crate::Result<()> {
    for sample in samples {
        match sample {
            TsSample::Audio(mut frame) => {
                frame.timestamp = frame.timestamp.saturating_add(connection_timestamp_offset);
                let timestamp = frame.timestamp;
                stats.set_audio_codec(crate::types::CodecName::Aac);
                stats.add_input_audio_data_count();
                stats.set_last_input_audio_timestamp(timestamp);
                if let Some(decoder) = audio_decoder
                    && let Some(tx) = audio_track_tx
                {
                    decoder.handle_input_sample(Some(crate::MediaFrame::Audio(
                        std::sync::Arc::new(frame),
                    )))?;
                    // Finished は EOS 入力時にしか発生しないため、通常フレーム処理中は Pending のみ返る
                    if crate::decoder::drain_audio_decoder_output(decoder, tx)?
                        == crate::decoder::DrainResult::PipelineClosed
                    {
                        return Err(crate::Error::new("audio track pipeline closed"));
                    }
                }
            }
            TsSample::Video(mut frame) => {
                frame.timestamp = frame.timestamp.saturating_add(connection_timestamp_offset);
                let timestamp = frame.timestamp;
                stats.set_video_codec(crate::types::CodecName::H264);
                stats.add_input_video_frame_count();
                stats.set_last_input_video_timestamp(timestamp);
                if let Some(decoder) = video_decoder
                    && let Some(tx) = video_track_tx
                {
                    decoder.handle_input_sample(Some(crate::MediaFrame::Video(
                        std::sync::Arc::new(frame),
                    )))?;
                    // Finished は EOS 入力時にしか発生しないため、通常フレーム処理中は Pending のみ返る
                    if crate::decoder::drain_video_decoder_output(decoder, tx)?
                        == crate::decoder::DrainResult::PipelineClosed
                    {
                        return Err(crate::Error::new("video track pipeline closed"));
                    }
                }
            }
        }
    }
    Ok(())
}

fn reset_connection_state(
    conn: &mut SrtConnection,
    endpoint_config: &SrtEndpointConfig,
    connection_ctx: &mut SrtConnectionContext<'_>,
) -> crate::Result<()> {
    *connection_ctx.peer_addr = None;
    *connection_ctx.demuxer = SrtTsDemuxer::new()?;
    *connection_ctx.connection_timestamp_offset = Duration::ZERO;
    *conn = create_listener_connection(endpoint_config)?;
    Ok(())
}

fn accept_peer_packet(
    state: ConnectionState,
    current_peer_addr: Option<SocketAddr>,
    incoming_addr: SocketAddr,
) -> bool {
    match (state, current_peer_addr) {
        (ConnectionState::Connected, Some(current_addr)) => current_addr == incoming_addr,
        _ => true,
    }
}

fn should_flush_pending_pes(event: &ConnectionEvent) -> bool {
    matches!(
        event,
        ConnectionEvent::StateChanged(ConnectionState::Disconnected)
            | ConnectionEvent::Disconnected { .. }
    )
}

async fn handle_connection_output(
    output: ConnectionOutput,
    socket: &UdpSocket,
    peer_addr: Option<SocketAddr>,
    timers: &mut HashMap<TimerId, Instant>,
) -> crate::Result<()> {
    match output {
        ConnectionOutput::SendPacket(buf) => {
            let peer_addr = peer_addr.ok_or_else(|| {
                crate::Error::new("peer address is not set while sending SRT packet")
            })?;
            socket.send_to(&buf, peer_addr).await?;
        }
        ConnectionOutput::SetTimer {
            id,
            duration_micros,
        } => {
            timers.insert(id, Instant::now() + Duration::from_micros(duration_micros));
        }
        ConnectionOutput::ClearTimer { id } => {
            timers.remove(&id);
        }
    }
    Ok(())
}

fn now_timestamp(base_time: Instant) -> Timestamp {
    let elapsed = base_time.elapsed();
    Timestamp::from_micros(elapsed.as_micros() as u64)
}

#[derive(Debug)]
struct SharedReadBufferInner {
    data: Vec<u8>,
    pos: usize,
}

impl SharedReadBufferInner {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            pos: 0,
        }
    }

    fn feed(&mut self, payload: &[u8]) {
        if self.pos == self.data.len() {
            self.data.clear();
            self.pos = 0;
        }
        self.data.extend_from_slice(payload);
    }

    fn available_bytes(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn read_impl(&mut self, buf: &mut [u8]) -> usize {
        let available = self.available_bytes();
        if available == 0 {
            return 0;
        }

        let n = buf.len().min(available);
        let end = self.pos + n;
        buf[..n].copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;

        if self.pos == self.data.len() {
            self.data.clear();
            self.pos = 0;
        }

        n
    }
}

#[derive(Debug, Clone)]
struct SharedReadBuffer {
    inner: Arc<Mutex<SharedReadBufferInner>>,
}

impl SharedReadBuffer {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SharedReadBufferInner::new())),
        }
    }

    fn feed(&self, payload: &[u8]) {
        let mut inner = self.inner.lock().expect("infallible");
        inner.feed(payload);
    }

    fn available_bytes(&self) -> usize {
        let inner = self.inner.lock().expect("infallible");
        inner.available_bytes()
    }
}

impl Read for SharedReadBuffer {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut inner = self.inner.lock().expect("infallible");
        Ok(inner.read_impl(buf))
    }
}

struct SrtTsDemuxer {
    stream: SharedReadBuffer,
    ts_reader: TsPacketReader<SharedReadBuffer>,
    pid_to_stream_type: HashMap<Pid, StreamType>,
    stream_id_to_pid: HashMap<StreamId, Pid>,
    pending_pes: HashMap<Pid, PendingPesPacket>,
    video_timestamp_mapper: crate::timestamp::mapper::TimestampMapper,
    audio_timestamp_mapper: crate::timestamp::mapper::TimestampMapper,
    last_aac_config_key: Option<AacConfigKey>,
    /// `AudioFrame.sample_entry` の不変条件（issue 0030）に従い、
    /// 直近の AAC サンプルエントリーを保持して全 AAC AU に clone して付与する。
    /// `last_aac_config_key` が変化したときに新規生成して両フィールドを更新する。
    last_aac_sample_entry: Option<crate::sample_entry::SharedSampleEntry>,
    /// 直近の SPS / PPS 含有 IDR から構築した H.264 sample_entry を保持し、
    /// 後続の Annex-B フレームに clone して付与する。
    /// SRT Annex-B 入力では IDR の inline NAL ユニットからのみ SPS / PPS を取得する設計のため、
    /// 確定までは `None` で、確定までの全フレームは下流に流さない。
    last_video_sample_entry: Option<crate::sample_entry::SharedSampleEntry>,
    /// 直近の SPS から抽出した解像度を保持し、後続の Annex-B フレームの `VideoFrame.size` に反映する。
    /// `last_video_sample_entry` と同期して IDR 検出時に更新される。
    last_video_frame_size: Option<crate::video::VideoFrameSize>,
}

impl SrtTsDemuxer {
    fn new() -> crate::Result<Self> {
        let stream = SharedReadBuffer::new();
        let ts_reader = TsPacketReader::new(stream.clone());
        Ok(Self {
            stream,
            ts_reader,
            pid_to_stream_type: HashMap::new(),
            stream_id_to_pid: HashMap::new(),
            pending_pes: HashMap::new(),
            video_timestamp_mapper: crate::timestamp::mapper::TimestampMapper::new(
                33,
                90_000,
                Duration::ZERO,
            )?,
            audio_timestamp_mapper: crate::timestamp::mapper::TimestampMapper::new(
                33,
                90_000,
                Duration::ZERO,
            )?,
            last_aac_config_key: None,
            last_aac_sample_entry: None,
            last_video_sample_entry: None,
            last_video_frame_size: None,
        })
    }

    fn push_payload(&mut self, payload: &[u8]) -> crate::Result<Vec<TsSample>> {
        self.stream.feed(payload);

        let mut samples = Vec::new();
        while self.stream.available_bytes() >= TS_PACKET_SIZE {
            let packet = match self.ts_reader.read_ts_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => break,
                Err(e) => {
                    // SRT 受信中は PMT 未読や同期ずれが起こり得るため、
                    // recover 可能なエラーはここで読み飛ばして継続する。
                    let msg = e.to_string();
                    if msg.contains("Unknown PID") || msg.contains("Expected sync byte 0x47") {
                        continue;
                    }
                    return Err(crate::Error::new(format!(
                        "failed to parse TS packet: {msg}"
                    )));
                }
            };

            let mut packet_samples = self.handle_ts_packet(packet)?;
            samples.append(&mut packet_samples);
        }

        Ok(samples)
    }

    fn flush_pending(&mut self) -> crate::Result<Vec<TsSample>> {
        let pending_pes = std::mem::take(&mut self.pending_pes);
        let mut samples = Vec::new();
        for (_, pending) in pending_pes {
            if let Some(expected_data_len) = pending.expected_data_len
                && pending.data.len() < expected_data_len
            {
                continue;
            }
            let mut completed = self.complete_pes(pending)?;
            samples.append(&mut completed);
        }
        Ok(samples)
    }

    fn handle_ts_packet(&mut self, packet: TsPacket) -> crate::Result<Vec<TsSample>> {
        let mut samples = Vec::new();

        match packet.payload {
            Some(TsPayload::Pmt(pmt)) => {
                for es_info in pmt.es_info {
                    self.pid_to_stream_type
                        .insert(es_info.elementary_pid, es_info.stream_type);
                }
            }
            Some(TsPayload::PesStart(pes)) => {
                if self.pid_to_stream_type.contains_key(&packet.header.pid) {
                    self.stream_id_to_pid
                        .insert(pes.header.stream_id, packet.header.pid);
                }

                if let Some(previous) = self.pending_pes.remove(&packet.header.pid) {
                    let mut completed = self.complete_pes(previous)?;
                    samples.append(&mut completed);
                }

                let expected_data_len = pes_expected_data_len(pes.pes_packet_len, &pes.header)?;
                let pending = PendingPesPacket {
                    header: pes.header,
                    data: pes.data.to_vec(),
                    expected_data_len,
                };

                if is_pes_ready(&pending) {
                    let mut completed = self.complete_pes(pending)?;
                    samples.append(&mut completed);
                } else {
                    self.pending_pes.insert(packet.header.pid, pending);
                }
            }
            Some(TsPayload::PesContinuation(bytes)) => {
                let Some(mut pending) = self.pending_pes.remove(&packet.header.pid) else {
                    return Ok(samples);
                };

                pending.data.extend_from_slice(&bytes);
                if let Some(expected_data_len) = pending.expected_data_len
                    && pending.data.len() > expected_data_len
                {
                    return Err(crate::Error::new(format!(
                        "unexpected PES payload length: expected={expected_data_len}, actual={}",
                        pending.data.len()
                    )));
                }

                if is_pes_ready(&pending) {
                    let mut completed = self.complete_pes(pending)?;
                    samples.append(&mut completed);
                } else {
                    self.pending_pes.insert(packet.header.pid, pending);
                }
            }
            _ => {}
        }

        Ok(samples)
    }

    fn complete_pes(&mut self, pending: PendingPesPacket) -> crate::Result<Vec<TsSample>> {
        let stream_type = self
            .stream_id_to_pid
            .get(&pending.header.stream_id)
            .and_then(|pid| self.pid_to_stream_type.get(pid))
            .copied()
            .or_else(|| {
                if pending.header.stream_id.is_video() {
                    Some(StreamType::H264)
                } else if pending.header.stream_id.is_audio() {
                    Some(StreamType::AdtsAac)
                } else {
                    None
                }
            });

        if pending.header.stream_id.is_video() {
            return self
                .build_video_sample(pending, stream_type)
                .map(|sample| sample.into_iter().collect());
        }
        if pending.header.stream_id.is_audio() {
            return self.build_audio_samples(pending, stream_type);
        }
        Ok(Vec::new())
    }

    fn build_video_sample(
        &mut self,
        pending: PendingPesPacket,
        stream_type: Option<StreamType>,
    ) -> crate::Result<Option<TsSample>> {
        match stream_type {
            Some(StreamType::H264) => {}
            Some(other) => {
                return Err(crate::Error::new(format!(
                    "unsupported video stream type: {other:?}"
                )));
            }
            None => return Ok(None),
        }

        let pts = pending
            .header
            .pts
            .ok_or_else(|| crate::Error::new("missing PTS in H264 PES"))?;
        let dts = pending.header.dts.unwrap_or(pts);

        // IDR 判定と SPS / PPS NAL 収集を同じループで実施する (IDR 検出時も break せず最後まで走査)。
        // 複数 SPS / PPS の扱いは `h264_sample_entry_from_sps_pps_lists` の docstring を参照。
        let mut keyframe = false;
        let mut sps_list: Vec<Vec<u8>> = Vec::new();
        let mut pps_list: Vec<Vec<u8>> = Vec::new();
        for nalu in crate::video::h264::H264AnnexBNalUnits::new(&pending.data) {
            let nalu = nalu?;
            match nalu.ty {
                crate::video::h264::H264_NALU_TYPE_IDR => keyframe = true,
                crate::video::h264::H264_NALU_TYPE_SPS => sps_list.push(nalu.data.to_vec()),
                crate::video::h264::H264_NALU_TYPE_PPS => pps_list.push(nalu.data.to_vec()),
                _ => {}
            }
        }

        if keyframe {
            // IDR 内 inline SPS / PPS から sample_entry と VideoFrame.size の両方を構築する。
            // SPS / PPS 不在 IDR や破損 NAL、SPS パース失敗は Err を返して接続を打ち切る (fail-fast)。
            // 正常な H.264 ストリームは IDR に SPS / PPS を inline するため、Err はエンコーダ側の異常とみなす。
            let (entry, frame_size) =
                crate::video::h264::h264_sample_entry_from_sps_pps_lists(sps_list, pps_list)?;
            self.last_video_sample_entry = Some(crate::sample_entry::SharedSampleEntry::new(entry));
            self.last_video_frame_size = Some(frame_size);
        }

        // 初回の SPS / PPS 含有 IDR が来るまでは P フレーム等を破棄する。
        if self.last_video_sample_entry.is_none() {
            return Ok(None);
        }

        let timestamp = self.video_timestamp_mapper.map(dts.as_u64());

        Ok(Some(TsSample::Video(crate::VideoFrame {
            data: pending.data,
            format: crate::video::VideoFormat::H264AnnexB,
            keyframe,
            size: self.last_video_frame_size,
            timestamp,
            sample_entry: self.last_video_sample_entry.clone(),
        })))
    }

    fn build_audio_samples(
        &mut self,
        pending: PendingPesPacket,
        stream_type: Option<StreamType>,
    ) -> crate::Result<Vec<TsSample>> {
        match stream_type {
            Some(StreamType::AdtsAac) => {}
            Some(other) => {
                return Err(crate::Error::new(format!(
                    "unsupported audio stream type: {other:?}"
                )));
            }
            None => return Ok(Vec::new()),
        }

        let pts = pending
            .header
            .pts
            .ok_or_else(|| crate::Error::new("missing PTS in AAC PES"))?;

        let mut samples = Vec::new();
        let mut offset = 0usize;
        let mut frame_index = 0u64;
        while offset < pending.data.len() {
            let header = parse_adts_header(&pending.data[offset..])?;
            let frame_len = header.frame_length as usize;
            let header_len = header.header_length();

            if frame_len == 0 {
                break;
            }
            if offset + frame_len > pending.data.len() {
                break;
            }
            if frame_len <= header_len {
                return Err(crate::Error::new("invalid ADTS frame length"));
            }

            let raw_data = pending.data[offset + header_len..offset + frame_len].to_vec();

            let sample_rate =
                crate::audio::aac::sample_rate_from_sampling_frequency_index(header.sample_rate())?;
            let channels = header.channel_configuration;
            let channels_value = crate::audio::Channels::from_u8(channels)?;
            let aac_config_key = header.config_key();
            // `last_aac_config_key` と `last_aac_sample_entry` は同期更新される。
            // どちらも初期値は None で、config 変化時のみ両方を同じ if 分岐で Some に更新する。
            // 初回 AAC AU 受信時は `last_aac_config_key == None` のため必ず if に入り、
            // 後段の `last_aac_sample_entry.clone()` が None を返すことはない。
            // この同期によって `AudioFrame.sample_entry` の不変条件（全 AAC AU に Some を載せる）が
            // SRT 入力経路でも成立する。
            if self.last_aac_config_key != Some(aac_config_key) {
                let audio_specific_config = header.audio_specific_config();
                let entry = crate::audio::aac::create_mp4a_sample_entry(
                    &audio_specific_config,
                    sample_rate,
                    channels_value,
                )?;
                self.last_aac_config_key = Some(aac_config_key);
                self.last_aac_sample_entry =
                    Some(crate::sample_entry::SharedSampleEntry::new(entry));
            }
            // 不変条件に従い全 AAC AU に保持値を clone して付与する。
            // Arc clone なので安価。
            let sample_entry = self.last_aac_sample_entry.clone();
            let pts_ticks = frame_index
                .saturating_mul(1024)
                .saturating_mul(90_000)
                .checked_div(sample_rate.get() as u64)
                .unwrap_or(0);
            let timestamp = self
                .audio_timestamp_mapper
                .map(pts.as_u64().saturating_add(pts_ticks));
            samples.push(TsSample::Audio(crate::AudioFrame {
                data: raw_data,
                format: crate::audio::AudioFormat::Aac,
                channels: channels_value,
                sample_rate,
                timestamp,
                sample_entry,
            }));

            offset += frame_len;
            frame_index = frame_index.saturating_add(1);
        }

        Ok(samples)
    }
}

fn pes_expected_data_len(pes_packet_len: u16, header: &PesHeader) -> crate::Result<Option<usize>> {
    if pes_packet_len == 0 {
        return Ok(None);
    }

    let optional_header_len = pes_optional_header_len(header);
    if pes_packet_len < optional_header_len {
        return Err(crate::Error::new(format!(
            "invalid PES header length: pes_packet_len={}, optional_header_len={optional_header_len}",
            pes_packet_len
        )));
    }
    Ok(Some((pes_packet_len - optional_header_len) as usize))
}

fn is_pes_ready(pending: &PendingPesPacket) -> bool {
    match pending.expected_data_len {
        Some(expected_data_len) => pending.data.len() >= expected_data_len,
        None => false, // PES 長が不定（0）の場合は次の PES 開始まで継続して連結する
    }
}

fn pes_optional_header_len(header: &PesHeader) -> u16 {
    3 + header.pts.map_or(0, |_| 5) + header.dts.map_or(0, |_| 5) + header.escr.map_or(0, |_| 6)
}

#[derive(Debug, Clone, Copy)]
struct AdtsHeader {
    protection_absent: bool,
    audio_object_type: u8,
    sampling_frequency_index: u8,
    channel_configuration: u8,
    frame_length: u16,
}

impl AdtsHeader {
    fn header_length(self) -> usize {
        if self.protection_absent { 7 } else { 9 }
    }

    fn sample_rate(self) -> u8 {
        self.sampling_frequency_index
    }

    fn config_key(self) -> AacConfigKey {
        AacConfigKey {
            audio_object_type: self.audio_object_type,
            sampling_frequency_index: self.sampling_frequency_index,
            channel_configuration: self.channel_configuration,
        }
    }

    fn audio_specific_config(self) -> Vec<u8> {
        crate::audio::aac::create_audio_specific_config(
            self.audio_object_type,
            self.sampling_frequency_index,
            self.channel_configuration,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AacConfigKey {
    audio_object_type: u8,
    sampling_frequency_index: u8,
    channel_configuration: u8,
}

fn parse_adts_header(data: &[u8]) -> crate::Result<AdtsHeader> {
    if data.len() < 7 {
        return Err(crate::Error::new("ADTS header too short"));
    }

    if data[0] != 0xFF || (data[1] & 0xF0) != 0xF0 {
        return Err(crate::Error::new("invalid ADTS sync word"));
    }

    let protection_absent = (data[1] & 0x01) != 0;
    let audio_object_type = ((data[2] >> 6) & 0x03) + 1;
    let sampling_frequency_index = (data[2] >> 2) & 0x0F;
    let channel_configuration = ((data[2] & 0x01) << 2) | ((data[3] >> 6) & 0x03);
    let frame_length =
        ((data[3] & 0x03) as u16) << 11 | (data[4] as u16) << 3 | ((data[5] >> 5) & 0x07) as u16;

    Ok(AdtsHeader {
        protection_absent,
        audio_object_type,
        sampling_frequency_index,
        channel_configuration,
        frame_length,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_adts_header_extracts_aac_config() {
        let adts = [
            0xFF, 0xF1, 0x50, 0x80, 0x02, 0x7F, 0xFC, // ADTS header
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, // payload
        ];
        let header = parse_adts_header(&adts).expect("must parse ADTS header");

        assert_eq!(header.audio_object_type, 2);
        assert_eq!(header.sampling_frequency_index, 4);
        assert_eq!(header.channel_configuration, 2);
        assert_eq!(header.frame_length, 19);
        assert_eq!(header.audio_specific_config(), vec![0x12, 0x10]);
    }

    // テスト内で PendingPesPacket を組み立てる共通ヘルパー。
    // 各経路向けの薄いラッパー (make_aac_pending_pes / make_h264_pending_pes) はこれを呼ぶ。
    fn make_pending_pes(
        stream_id: mpeg2ts::es::StreamId,
        data: Vec<u8>,
        pts_ticks: u64,
    ) -> PendingPesPacket {
        use mpeg2ts::time::Timestamp;
        PendingPesPacket {
            header: PesHeader {
                stream_id,
                priority: false,
                data_alignment_indicator: true,
                copyright: false,
                original_or_copy: false,
                pts: Some(Timestamp::new(pts_ticks).expect("PTS が範囲内であること")),
                dts: None,
                escr: None,
            },
            data,
            expected_data_len: None,
        }
    }

    // SRT 入力経路の AAC AU 処理が AudioFrame.sample_entry の不変条件を満たすことを検証するヘルパー。
    // `build_audio_samples` に渡す PendingPesPacket をテスト内で組み立てる。
    fn make_aac_pending_pes(data: Vec<u8>, pts_ticks: u64) -> PendingPesPacket {
        use mpeg2ts::es::StreamId;
        make_pending_pes(StreamId::new(StreamId::AUDIO_MIN), data, pts_ticks)
    }

    // ADTS ヘッダ (7 バイト) と 12 バイトのダミー payload から成る AAC AU。
    // 既存テスト `parse_adts_header_extracts_aac_config` で動作確認済みのバイト列。
    // 44.1kHz / stereo / AAC LC / frame_length=19。
    const AAC_AU_STEREO_44_1KHZ: [u8; 19] = [
        0xFF, 0xF1, 0x50, 0x80, 0x02, 0x7F, 0xFC, // ADTS header
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, // payload
    ];

    // 上記と同条件で channel_configuration のみ mono (=1) に変えた AAC AU。
    // ADTS 4 バイト目の上位 2 bit（channel_configuration の下位 2 bit）を 10 → 01 に変更し、
    // 3 バイト目の最終 bit（channel_configuration の MSB）は 0 のまま据え置く。
    // この変更で `parse_adts_header` の `channel_configuration` が 2 → 1 に変わり、
    // `config_key()` が異なる値を返すため `last_aac_sample_entry` の更新分岐に入る。
    const AAC_AU_MONO_44_1KHZ: [u8; 19] = [
        0xFF, 0xF1, 0x50, 0x40, 0x02, 0x7F, 0xFC, // ADTS header (mono)
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, // payload
    ];

    // AudioFrame.sample_entry の不変条件が SRT 入力経路の「config 連続」シナリオで成立することを検証する。
    // 同一の ADTS 設定を持つ AU を 3 個含む PES を `build_audio_samples` に渡し、
    // 全 AU の sample_entry が `Some` でありかつ初回 AU と等価（changed_since=false）であることを確認する。
    // 等価性まで検証するのは、demuxer が dummy SampleEntry を毎回新規生成しても素通りしないようにするため。
    #[test]
    fn srt_aac_emits_sample_entry_on_every_au_with_constant_config() -> crate::Result<()> {
        let mut demuxer = SrtTsDemuxer::new()?;
        let mut pes_data = Vec::new();
        for _ in 0..3 {
            pes_data.extend_from_slice(&AAC_AU_STEREO_44_1KHZ);
        }
        let pending = make_aac_pending_pes(pes_data, 100_000);
        let samples = demuxer.build_audio_samples(pending, Some(StreamType::AdtsAac))?;
        assert_eq!(samples.len(), 3, "3 AU が分解されていること");

        let mut first_entry: Option<crate::sample_entry::SharedSampleEntry> = None;
        for (idx, sample) in samples.into_iter().enumerate() {
            let TsSample::Audio(frame) = sample else {
                panic!("AU #{idx} が音声サンプルとして取り出せること");
            };
            let entry = frame
                .sample_entry
                .unwrap_or_else(|| panic!("AU #{idx} に sample_entry が載っていること"));
            if let Some(ref first) = first_entry {
                assert!(
                    !entry.changed_since(Some(first)),
                    "config 連続時、AU #{idx} の sample_entry が初回と等価であること"
                );
            } else {
                first_entry = Some(entry);
            }
        }
        Ok(())
    }

    // SRT 入力経路の「config 変化」シナリオ。1 PES 内で channel_configuration を
    // stereo → mono → stereo と変化させ、全 AU に sample_entry が載ること、
    // config 変化を跨ぐ AU 間で sample_entry が変化することを確認する。
    // `last_aac_config_key` と `last_aac_sample_entry` の同期更新が正しく機能していることを検証する。
    #[test]
    fn srt_aac_updates_sample_entry_on_config_change() -> crate::Result<()> {
        let mut demuxer = SrtTsDemuxer::new()?;
        let mut pes_data = Vec::new();
        pes_data.extend_from_slice(&AAC_AU_STEREO_44_1KHZ);
        pes_data.extend_from_slice(&AAC_AU_MONO_44_1KHZ);
        pes_data.extend_from_slice(&AAC_AU_STEREO_44_1KHZ);
        let pending = make_aac_pending_pes(pes_data, 100_000);
        let samples = demuxer.build_audio_samples(pending, Some(StreamType::AdtsAac))?;
        assert_eq!(samples.len(), 3, "3 AU が分解されていること");

        let mut entries = Vec::new();
        for (idx, sample) in samples.into_iter().enumerate() {
            let TsSample::Audio(frame) = sample else {
                panic!("AU #{idx} が音声サンプルとして取り出せること");
            };
            let entry = frame
                .sample_entry
                .unwrap_or_else(|| panic!("AU #{idx} に sample_entry が載っていること"));
            entries.push(entry);
        }

        // stereo → mono への変化で sample_entry が更新される。
        assert!(
            entries[1].changed_since(Some(&entries[0])),
            "stereo → mono で sample_entry が変化すること"
        );
        // mono → stereo への変化でも更新される。
        assert!(
            entries[2].changed_since(Some(&entries[1])),
            "mono → stereo で sample_entry が変化すること"
        );
        Ok(())
    }

    #[test]
    fn create_aac_sample_entry_keeps_config_in_esds() {
        let sample_entry = crate::audio::aac::create_mp4a_sample_entry(
            &[0x12, 0x10],
            crate::audio::SampleRate::from_u32(44_100).expect("must create sample rate"),
            crate::audio::Channels::STEREO,
        )
        .expect("must create AAC sample entry");

        let shiguredo_mp4::boxes::SampleEntry::Mp4a(mp4a) = sample_entry else {
            panic!("expected Mp4a sample entry");
        };

        assert_eq!(mp4a.audio.channelcount, 2);
        assert_eq!(mp4a.audio.samplerate.integer, 44_100);

        let dec_specific_info = mp4a
            .esds_box
            .es
            .dec_config_descr
            .dec_specific_info
            .as_ref()
            .expect("AudioSpecificConfig must exist");
        assert_eq!(dec_specific_info.payload, vec![0x12, 0x10]);
    }

    // `build_video_sample` に渡す PendingPesPacket をテスト内で組み立てる。
    // `StreamId::new_video` で `is_video()` 型検査により誤値混入を弾く。
    fn make_h264_pending_pes(data: Vec<u8>, pts_ticks: u64) -> PendingPesPacket {
        use mpeg2ts::es::StreamId;
        make_pending_pes(
            StreamId::new_video(StreamId::VIDEO_MIN)
                .expect("VIDEO_MIN が映像範囲の有効な値であること"),
            data,
            pts_ticks,
        )
    }

    // テスト用の NAL ユニット定数。先頭 4 バイトは start code prefix。
    // 5 バイト目が NAL header で、上位 1 bit が forbidden_zero_bit、下位 5 bit が nal_unit_type。
    // payload バイト列は隣接 NAL の誤分割を避けるため `0x00, 0x00, 0x01` シーケンスを含めない。
    //
    // SPS バイト列は ffmpeg + libx264 で生成した実機 SPS（解像度抽出までビット位置が届く完全 SPS）。
    // 生成手順は `src/video/h264.rs` の `mod tests` 冒頭コメントを参照。

    // nal_unit_type=7（SPS）。解像度 1920x1080 (Baseline)。`parse_sps` が (1920, 1080) を返す。
    // SPS バイト列本体は `crate::video::h264::tests::SPS_1920X1080` で集約管理されており、
    // ここでは Annex-B 形式 (先頭 4 バイト start code 付き) を関数で取り出す。
    fn sps_initial() -> &'static [u8] {
        &crate::video::h264::tests::SPS_1920X1080_ANNEXB
    }

    // sps_initial() とは異なる解像度（1280x720、Baseline）の SPS。mid-stream で SPS が変化したシナリオの検証用。
    // SPS_UPDATED は SRT inbound テスト専用のためローカルに定義する。`parse_sps` が (1280, 720) を返す。
    const SPS_UPDATED: [u8; 29] = [
        0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xc0, 0x1f, 0xd9, 0x00, 0x50, 0x05, 0xbb, 0x01, 0x10,
        0x00, 0x00, 0x03, 0x00, 0x10, 0x00, 0x00, 0x03, 0x03, 0xc0, 0xf1, 0x83, 0x24, 0x80,
    ];

    // nal_unit_type=8（PPS）。
    const PPS: [u8; 8] = [0x00, 0x00, 0x00, 0x01, 0x68, 0xce, 0x06, 0xe2];

    // nal_unit_type=5（IDR）。
    const IDR: [u8; 8] = [0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x21];

    // nal_unit_type=1（non-IDR coded slice、P フレーム）。
    const P_FRAME: [u8; 8] = [0x00, 0x00, 0x00, 0x01, 0x41, 0x9a, 0x21, 0x6c];

    // VideoFrame.sample_entry の不変条件が SRT 入力経路の「SPS / PPS 含有 IDR で確定 + 後続 P フレーム付与」シナリオで成立することを検証する。
    // SPS + PPS + IDR を含む PES と P フレームのみの PES を順に投入し、3 フレーム全てに `Some` が載り、
    // 後続フレームの sample_entry が初回と等価（`changed_since=false`）であることを確認する。
    #[test]
    fn srt_h264_emits_sample_entry_on_every_frame_after_sps_pps_idr() -> crate::Result<()> {
        let mut demuxer = SrtTsDemuxer::new()?;

        let pes1 = [sps_initial(), &PPS, &IDR].concat();
        let samples1 = demuxer
            .build_video_sample(make_h264_pending_pes(pes1, 100_000), Some(StreamType::H264))?;
        let sample1 =
            samples1.expect("SPS / PPS 含有 IDR で sample_entry が確定し、フレームが流れること");
        let TsSample::Video(frame1) = sample1 else {
            panic!("映像サンプルとして取り出せること");
        };
        let entry1 = frame1
            .sample_entry
            .clone()
            .expect("初回 IDR に sample_entry が載っていること");

        let pes2 = P_FRAME.to_vec();
        let samples2 = demuxer
            .build_video_sample(make_h264_pending_pes(pes2, 103_000), Some(StreamType::H264))?;
        let sample2 = samples2.expect("確定後の P フレームが流れること");
        let TsSample::Video(frame2) = sample2 else {
            panic!("映像サンプルとして取り出せること");
        };
        let entry2 = frame2
            .sample_entry
            .expect("確定後の P フレームに sample_entry が載っていること");
        assert!(
            !entry2.changed_since(Some(&entry1)),
            "確定後の P フレームの sample_entry が初回 IDR と等価であること"
        );

        let pes3 = P_FRAME.to_vec();
        let samples3 = demuxer
            .build_video_sample(make_h264_pending_pes(pes3, 106_000), Some(StreamType::H264))?;
        let sample3 = samples3.expect("2 つ目の P フレームも流れること");
        let TsSample::Video(frame3) = sample3 else {
            panic!("映像サンプルとして取り出せること");
        };
        let entry3 = frame3
            .sample_entry
            .expect("2 つ目の P フレームにも sample_entry が載っていること");
        assert!(
            !entry3.changed_since(Some(&entry1)),
            "2 つ目の P フレームの sample_entry も初回 IDR と等価であること"
        );

        Ok(())
    }

    // IDR より後ろに SPS / PPS が並ぶ Annex-B ストリームでも sample_entry が確定することを検証する。
    // build_video_sample が PES データ全体を走査して SPS / PPS を収集することの回帰防止。
    #[test]
    fn srt_h264_emits_sample_entry_on_idr_with_trailing_sps_pps() -> crate::Result<()> {
        let mut demuxer = SrtTsDemuxer::new()?;

        // IDR を先頭に置き、その後ろに SPS と PPS を並べる。
        let pes = [&IDR[..], sps_initial(), &PPS].concat();
        let samples = demuxer
            .build_video_sample(make_h264_pending_pes(pes, 100_000), Some(StreamType::H264))?;
        let sample = samples.expect("IDR 後置 SPS / PPS でも sample_entry が確定して流れること");
        let TsSample::Video(frame) = sample else {
            panic!("映像サンプルとして取り出せること");
        };
        assert!(
            frame.sample_entry.is_some(),
            "IDR 後置 SPS / PPS の PES に sample_entry が載っていること"
        );

        Ok(())
    }

    // SPS / PPS 不在の IDR が来た場合は Err を返して接続を打ち切る方針の回帰防止。
    // 正常な H.264 ストリームは IDR に SPS / PPS を inline するため、不在はエンコーダ側の異常とみなす。
    #[test]
    fn srt_h264_returns_err_on_idr_without_sps_pps() {
        let mut demuxer = SrtTsDemuxer::new().expect("demuxer 生成に成功すること");

        let pes = IDR.to_vec();
        let result =
            demuxer.build_video_sample(make_h264_pending_pes(pes, 100_000), Some(StreamType::H264));
        assert!(result.is_err(), "SPS / PPS 不在 IDR は Err を返すこと");
    }

    // PPS 不在（SPS のみ含有）の IDR は `h264_sample_entry_from_sps_pps_lists` が
    // `missing H.264 PPS` Err を返し、それが上位に伝播することを検証する。
    #[test]
    fn srt_h264_returns_err_on_idr_with_only_sps() {
        let mut demuxer = SrtTsDemuxer::new().expect("demuxer 生成に成功すること");

        let pes = [sps_initial(), &IDR].concat();
        let result =
            demuxer.build_video_sample(make_h264_pending_pes(pes, 100_000), Some(StreamType::H264));
        assert!(result.is_err(), "PPS 不在 IDR は Err を返すこと");
    }

    // SPS 不在（PPS のみ含有）の IDR は `h264_sample_entry_from_sps_pps_lists` が
    // `missing H.264 SPS` Err を返し、それが上位に伝播することを検証する。
    #[test]
    fn srt_h264_returns_err_on_idr_with_only_pps() {
        let mut demuxer = SrtTsDemuxer::new().expect("demuxer 生成に成功すること");

        let pes = [&PPS[..], &IDR].concat();
        let result =
            demuxer.build_video_sample(make_h264_pending_pes(pes, 100_000), Some(StreamType::H264));
        assert!(result.is_err(), "SPS 不在 IDR は Err を返すこと");
    }

    // build_video_sample が返す sample_entry の `Avc1Box.visual.width / .height` と、
    // VideoFrame.size の両方が SPS 由来の実値（1920x1080 / 1280x720）になっていることを直接検証する。
    // h264_sample_entry_from_sps_pps_lists が IDR ごとに sample_entry / VideoFrame.size に
    // 解像度を反映することの回帰防止。
    #[test]
    fn srt_h264_sample_entry_and_size_reflect_sps_dimensions() -> crate::Result<()> {
        use shiguredo_mp4::boxes::SampleEntry;

        let mut demuxer = SrtTsDemuxer::new()?;

        // 初期 IDR: sps_initial() から 1920x1080 を抽出して埋め込む
        let pes1 = [sps_initial(), &PPS, &IDR].concat();
        let samples1 = demuxer
            .build_video_sample(make_h264_pending_pes(pes1, 100_000), Some(StreamType::H264))?;
        let sample1 = samples1.expect("sps_initial() 含有 IDR でフレームが流れること");
        let TsSample::Video(frame1) = sample1 else {
            panic!("映像サンプルとして取り出せること");
        };
        let entry1 = frame1
            .sample_entry
            .clone()
            .expect("初期 IDR に sample_entry が載っていること");
        let SampleEntry::Avc1(avc1) = entry1.get() else {
            panic!("AVC1 サンプルエントリーであること");
        };
        assert_eq!(
            (avc1.visual.width, avc1.visual.height),
            (1920, 1080),
            "Avc1Box.visual に sps_initial() 由来の解像度 1920x1080 が埋め込まれていること"
        );
        assert_eq!(
            frame1.size,
            Some(crate::video::VideoFrameSize::new(1920, 1080)?),
            "VideoFrame.size に sps_initial() 由来の解像度 1920x1080 が反映されていること"
        );

        // 後続の P フレームも同じ解像度を引き継いで size に反映されること
        let pes_p = P_FRAME.to_vec();
        let samples_p = demuxer.build_video_sample(
            make_h264_pending_pes(pes_p, 103_000),
            Some(StreamType::H264),
        )?;
        let sample_p = samples_p.expect("P フレームが流れること");
        let TsSample::Video(frame_p) = sample_p else {
            panic!("映像サンプルとして取り出せること");
        };
        assert_eq!(
            frame_p.size,
            Some(crate::video::VideoFrameSize::new(1920, 1080)?),
            "P フレームにも初期 IDR 由来の解像度 1920x1080 が引き継がれていること"
        );

        // mid-stream で SPS_UPDATED に切り替わると Avc1Box / VideoFrame.size の両方が更新されること
        let pes2 = [&SPS_UPDATED[..], &PPS, &IDR].concat();
        let samples2 = demuxer
            .build_video_sample(make_h264_pending_pes(pes2, 106_000), Some(StreamType::H264))?;
        let sample2 = samples2.expect("SPS_UPDATED 含有 IDR でフレームが流れること");
        let TsSample::Video(frame2) = sample2 else {
            panic!("映像サンプルとして取り出せること");
        };
        let entry2 = frame2
            .sample_entry
            .clone()
            .expect("新 IDR に sample_entry が載っていること");
        let SampleEntry::Avc1(avc1_2) = entry2.get() else {
            panic!("AVC1 サンプルエントリーであること");
        };
        assert_eq!(
            (avc1_2.visual.width, avc1_2.visual.height),
            (1280, 720),
            "Avc1Box.visual に SPS_UPDATED 由来の解像度 1280x720 が更新されていること"
        );
        assert_eq!(
            frame2.size,
            Some(crate::video::VideoFrameSize::new(1280, 720)?),
            "VideoFrame.size にも SPS_UPDATED 由来の解像度 1280x720 が更新されていること"
        );

        Ok(())
    }

    // IDR の間に P フレームを複数挟んでも、`last_video_frame_size` が直前の IDR 由来の値を
    // 保持し続けて全 P フレームの `VideoFrame.size` に反映されること、
    // 次の IDR (mid-stream SPS 更新) で新値に切り替わって以降の P フレームに反映されることの回帰防止。
    #[test]
    fn srt_h264_video_frame_size_persists_across_p_frames_between_sps_changes() -> crate::Result<()>
    {
        let mut demuxer = SrtTsDemuxer::new()?;

        // 初期 IDR (sps_initial() = 1920x1080) を確定させる
        let pes_idr1 = [sps_initial(), &PPS, &IDR].concat();
        let TsSample::Video(_) = demuxer
            .build_video_sample(
                make_h264_pending_pes(pes_idr1, 100_000),
                Some(StreamType::H264),
            )?
            .expect("初期 IDR でフレームが流れること")
        else {
            panic!("映像サンプルとして取り出せること");
        };

        // P フレームを 2 連続投入し、いずれも 1920x1080 を保持していること
        for (i, ts) in [(1, 103_000), (2, 106_000)] {
            let pes_p = P_FRAME.to_vec();
            let TsSample::Video(frame_p) = demuxer
                .build_video_sample(make_h264_pending_pes(pes_p, ts), Some(StreamType::H264))?
                .expect("P フレームが流れること")
            else {
                panic!("映像サンプルとして取り出せること");
            };
            assert_eq!(
                frame_p.size,
                Some(crate::video::VideoFrameSize::new(1920, 1080)?),
                "{i} 個目の P フレームは初期 IDR 由来の 1920x1080 を保持すること"
            );
        }

        // mid-stream で SPS_UPDATED (1280x720) を含む IDR を投入し、`last_video_frame_size` が切り替わること
        let pes_idr2 = [&SPS_UPDATED[..], &PPS, &IDR].concat();
        let TsSample::Video(frame_idr2) = demuxer
            .build_video_sample(
                make_h264_pending_pes(pes_idr2, 109_000),
                Some(StreamType::H264),
            )?
            .expect("更新 IDR でフレームが流れること")
        else {
            panic!("映像サンプルとして取り出せること");
        };
        assert_eq!(
            frame_idr2.size,
            Some(crate::video::VideoFrameSize::new(1280, 720)?),
            "更新 IDR から VideoFrame.size が SPS_UPDATED 由来の 1280x720 に切り替わること"
        );

        // 更新後の P フレームも新値 (1280x720) を保持していること
        for (i, ts) in [(1, 112_000), (2, 115_000)] {
            let pes_p = P_FRAME.to_vec();
            let TsSample::Video(frame_p) = demuxer
                .build_video_sample(make_h264_pending_pes(pes_p, ts), Some(StreamType::H264))?
                .expect("更新後の P フレームが流れること")
            else {
                panic!("映像サンプルとして取り出せること");
            };
            assert_eq!(
                frame_p.size,
                Some(crate::video::VideoFrameSize::new(1280, 720)?),
                "更新後 {i} 個目の P フレームは更新 IDR 由来の 1280x720 を保持すること"
            );
        }

        Ok(())
    }

    // mid-stream で SPS / PPS が含有 IDR と一緒に更新された場合の挙動を検証する。
    // 確定後に SPS_UPDATED + PPS + IDR を投入すると `last_video_sample_entry` が新値に上書きされ、
    // 新 IDR 自身に新 entry が載って下流に流れる。
    #[test]
    fn srt_h264_updates_sample_entry_on_mid_stream_sps_change() -> crate::Result<()> {
        let mut demuxer = SrtTsDemuxer::new()?;

        let pes1 = [sps_initial(), &PPS, &IDR].concat();
        let samples1 = demuxer
            .build_video_sample(make_h264_pending_pes(pes1, 100_000), Some(StreamType::H264))?;
        let sample1 = samples1.expect("sps_initial() 含有 IDR で初期確定して流れること");
        let TsSample::Video(frame1) = sample1 else {
            panic!("映像サンプルとして取り出せること");
        };
        let entry1 = frame1
            .sample_entry
            .clone()
            .expect("初期確定 IDR に sample_entry が載っていること");

        let pes2 = [&SPS_UPDATED[..], &PPS, &IDR].concat();
        let samples2 = demuxer
            .build_video_sample(make_h264_pending_pes(pes2, 103_000), Some(StreamType::H264))?;
        let sample2 = samples2.expect("SPS_UPDATED 含有 IDR で新値に更新されて流れること");
        let TsSample::Video(frame2) = sample2 else {
            panic!("映像サンプルとして取り出せること");
        };
        let entry2 = frame2
            .sample_entry
            .expect("新 IDR に sample_entry が載っていること");
        assert!(
            entry2.changed_since(Some(&entry1)),
            "mid-stream SPS 変化で sample_entry が初期と異なる値に更新されていること"
        );

        Ok(())
    }
}
