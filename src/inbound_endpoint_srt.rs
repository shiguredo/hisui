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
pub struct SrtInboundEndpoint {
    pub input_url: String,
    pub output_audio_track_id: Option<crate::TrackId>,
    pub output_video_track_id: Option<crate::TrackId>,
    // SRT caller が送る streamid の期待値（省略時は検証しない）。
    pub stream_id: Option<String>,
    // SRT 暗号化（KM ハンドシェイク）を有効化するパスフレーズ。
    pub passphrase: Option<String>,
    // SRT 暗号化の鍵長（passphrase 指定時のみ有効）。
    pub key_length: Option<KeyLength>,
    // TSBPD 遅延。JSON-RPC ではミリ秒の u16 で受け取り、内部では Duration で保持する。
    pub tsbpd_delay_ms: Option<Duration>,
}

#[derive(Debug, Clone)]
struct SrtInboundEndpointStats {
    is_listening_metric: crate::stats::StatsFlag,
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
            .map_err(|e| crate::Error::new(format!("invalid inputUrl: {e}")))?;
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

        let mut demuxer = SrtTsDemuxer::new();

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
                            &stats,
                            connection_timestamp_offset,
                        );
                    }
                    if should_flush_pending_pes(&event) {
                        let flushed_samples = demuxer.flush_pending()?;
                        publish_samples(
                            flushed_samples,
                            &mut audio_track_tx,
                            &mut video_track_tx,
                            &stats,
                            connection_timestamp_offset,
                        );
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
        if self.passphrase.is_none() && self.key_length.is_some() {
            return Err(crate::Error::new(
                "keyLength requires passphrase to be specified",
            ));
        }

        Ok(SrtEndpointConfig {
            stream_id: self.stream_id.clone(),
            passphrase: self.passphrase.clone(),
            key_length: self.key_length.unwrap_or(KeyLength::Aes128),
            tsbpd_delay: self
                .tsbpd_delay_ms
                .map(tsbpd_delay_duration_to_millis)
                .transpose()?
                .unwrap_or(120),
        })
    }

    fn handle_connection_event(
        &self,
        event: ConnectionEvent,
        now: Timestamp,
        conn: &mut SrtConnection,
        endpoint_config: &SrtEndpointConfig,
        connection_ctx: &mut SrtConnectionContext<'_>,
    ) -> crate::Result<()> {
        match event {
            ConnectionEvent::Connected => {
                *connection_ctx.connection_timestamp_offset =
                    Duration::from_micros(now.as_micros());
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
                    reset_connection_state(conn, endpoint_config, connection_ctx)?;
                }
            }
            ConnectionEvent::Disconnected { reason } => {
                tracing::warn!("SRT disconnected: {reason}");
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

impl nojson::DisplayJson for SrtInboundEndpoint {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("inputUrl", &self.input_url)?;
            if let Some(track_id) = &self.output_audio_track_id {
                f.member("outputAudioTrackId", track_id)?;
            }
            if let Some(track_id) = &self.output_video_track_id {
                f.member("outputVideoTrackId", track_id)?;
            }
            if let Some(stream_id) = &self.stream_id {
                f.member("streamId", stream_id)?;
            }
            if let Some(passphrase) = &self.passphrase {
                f.member("passphrase", passphrase)?;
            }
            if let Some(key_length) = self.key_length {
                f.member("keyLength", key_length_to_rpc_value(key_length))?;
            }
            if let Some(tsbpd_delay_ms) = self
                .tsbpd_delay_ms
                .map(tsbpd_delay_duration_to_millis)
                .transpose()
                .map_err(|_| std::fmt::Error)?
            {
                f.member("tsbpdDelayMs", tsbpd_delay_ms)?;
            }
            Ok(())
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for SrtInboundEndpoint {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let input_url: String = value.to_member("inputUrl")?.required()?.try_into()?;
        let output_audio_track_id: Option<crate::TrackId> =
            value.to_member("outputAudioTrackId")?.try_into()?;
        let output_video_track_id: Option<crate::TrackId> =
            value.to_member("outputVideoTrackId")?.try_into()?;

        if output_audio_track_id.is_none() && output_video_track_id.is_none() {
            return Err(value.invalid("outputAudioTrackId or outputVideoTrackId is required"));
        }

        let stream_id = parse_optional_non_empty_string(value, "streamId")?;
        let passphrase = parse_optional_non_empty_string(value, "passphrase")?;
        let key_length = parse_optional_key_length(value)?;
        let tsbpd_delay_ms_raw: Option<u16> = value.to_member("tsbpdDelayMs")?.try_into()?;
        let tsbpd_delay_ms = tsbpd_delay_ms_raw.map(|ms| Duration::from_millis(ms as u64));

        if passphrase.is_none() && key_length.is_some() {
            return Err(value
                .to_member("keyLength")?
                .required()?
                .invalid("keyLength requires passphrase"));
        }

        if let Err(e) = parse_srt_url(&input_url) {
            return Err(value.to_member("inputUrl")?.required()?.invalid(e));
        }

        Ok(Self {
            input_url,
            output_audio_track_id,
            output_video_track_id,
            stream_id,
            passphrase,
            key_length,
            tsbpd_delay_ms,
        })
    }
}

fn parse_optional_non_empty_string(
    value: nojson::RawJsonValue<'_, '_>,
    member: &str,
) -> std::result::Result<Option<String>, nojson::JsonParseError> {
    let raw: Option<String> = value.to_member(member)?.try_into()?;
    match raw {
        Some(s) => {
            if s.is_empty() {
                return Err(value
                    .to_member(member)?
                    .required()?
                    .invalid(format!("{member} must not be empty")));
            }
            Ok(Some(s))
        }
        None => Ok(None),
    }
}

fn parse_optional_key_length(
    value: nojson::RawJsonValue<'_, '_>,
) -> std::result::Result<Option<KeyLength>, nojson::JsonParseError> {
    let raw: Option<u16> = value.to_member("keyLength")?.try_into()?;
    let Some(raw) = raw else {
        return Ok(None);
    };

    let key_length = match raw {
        16 => KeyLength::Aes128,
        32 => KeyLength::Aes256,
        _ => {
            return Err(value
                .to_member("keyLength")?
                .required()?
                .invalid("keyLength must be one of 16 or 32"));
        }
    };

    Ok(Some(key_length))
}

fn key_length_to_rpc_value(key_length: KeyLength) -> u16 {
    match key_length {
        KeyLength::Aes128 => 16,
        KeyLength::Aes256 => 32,
    }
}

fn tsbpd_delay_duration_to_millis(duration: Duration) -> crate::Result<u16> {
    let millis = duration.as_millis();
    u16::try_from(millis)
        .map_err(|_| crate::Error::new(format!("tsbpdDelayMs must be <= {}", u16::MAX)))
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
    audio_track_tx: &mut Option<crate::media_pipeline::MessageSender>,
    video_track_tx: &mut Option<crate::media_pipeline::MessageSender>,
    stats: &SrtInboundEndpointStats,
    connection_timestamp_offset: Duration,
) {
    for sample in samples {
        match sample {
            TsSample::Audio(mut frame) => {
                frame.timestamp = frame.timestamp.saturating_add(connection_timestamp_offset);
                let timestamp = frame.timestamp;
                if let Some(tx) = audio_track_tx {
                    tx.send_audio(frame);
                }
                stats.set_audio_codec(crate::types::CodecName::Aac);
                stats.add_input_audio_data_count();
                stats.set_last_input_audio_timestamp(timestamp);
            }
            TsSample::Video(mut frame) => {
                frame.timestamp = frame.timestamp.saturating_add(connection_timestamp_offset);
                let timestamp = frame.timestamp;
                if let Some(tx) = video_track_tx {
                    tx.send_video(frame);
                }
                stats.set_video_codec(crate::types::CodecName::H264);
                stats.add_input_video_frame_count();
                stats.set_last_input_video_timestamp(timestamp);
            }
        }
    }
}

fn reset_connection_state(
    conn: &mut SrtConnection,
    endpoint_config: &SrtEndpointConfig,
    connection_ctx: &mut SrtConnectionContext<'_>,
) -> crate::Result<()> {
    *connection_ctx.peer_addr = None;
    *connection_ctx.demuxer = SrtTsDemuxer::new();
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
    base_video_timestamp: Option<Duration>,
    base_audio_timestamp: Option<Duration>,
    received_video_keyframe: bool,
}

impl SrtTsDemuxer {
    fn new() -> Self {
        let stream = SharedReadBuffer::new();
        let ts_reader = TsPacketReader::new(stream.clone());
        Self {
            stream,
            ts_reader,
            pid_to_stream_type: HashMap::new(),
            stream_id_to_pid: HashMap::new(),
            pending_pes: HashMap::new(),
            base_video_timestamp: None,
            base_audio_timestamp: None,
            received_video_keyframe: false,
        }
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

        let mut keyframe = false;
        for nalu in crate::video_h264::H264AnnexBNalUnits::new(&pending.data) {
            let nalu = nalu?;
            if nalu.ty == crate::video_h264::H264_NALU_TYPE_IDR {
                keyframe = true;
                break;
            }
        }

        if !self.received_video_keyframe && !keyframe {
            return Ok(None);
        }
        if keyframe {
            self.received_video_keyframe = true;
        }

        let timestamp = timestamp_90khz_to_duration(dts.as_u64());
        let base_timestamp = *self.base_video_timestamp.get_or_insert(timestamp);
        let relative_timestamp = timestamp.saturating_sub(base_timestamp);

        Ok(Some(TsSample::Video(crate::VideoFrame {
            data: pending.data,
            format: crate::video::VideoFormat::H264AnnexB,
            keyframe,
            width: 0,
            height: 0,
            timestamp: relative_timestamp,
            duration: Duration::ZERO,
            sample_entry: None, // Annex-B 入力では sample_entry は付与しない
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

            let sample_rate = header.sample_rate();
            let sample_rate_value = crate::audio::SampleRate::from_u32(sample_rate)?;
            let channels = header.channel_configuration;
            let pts_ticks = frame_index
                .saturating_mul(1024)
                .saturating_mul(90_000)
                .checked_div(sample_rate as u64)
                .unwrap_or(0);
            let timestamp = timestamp_90khz_to_duration(pts.as_u64().saturating_add(pts_ticks));
            let base_timestamp = *self.base_audio_timestamp.get_or_insert(timestamp);
            let relative_timestamp = timestamp.saturating_sub(base_timestamp);
            let duration = Duration::from_micros(
                (1_000_000u64)
                    .saturating_mul(1024)
                    .checked_div(sample_rate as u64)
                    .unwrap_or(0),
            );

            samples.push(TsSample::Audio(crate::AudioFrame {
                data: raw_data,
                format: crate::audio::AudioFormat::Aac,
                channels: crate::audio::Channels::from_u16(channels)?,
                sample_rate: sample_rate_value,
                timestamp: relative_timestamp,
                duration,
                sample_entry: None,
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

fn timestamp_90khz_to_duration(timestamp: u64) -> Duration {
    Duration::from_micros(timestamp.saturating_mul(1_000_000) / 90_000)
}

#[derive(Debug, Clone, Copy)]
struct AdtsHeader {
    protection_absent: bool,
    sampling_frequency_index: u8,
    channel_configuration: u16,
    frame_length: u16,
}

impl AdtsHeader {
    fn header_length(self) -> usize {
        if self.protection_absent { 7 } else { 9 }
    }

    fn sample_rate(self) -> u32 {
        match self.sampling_frequency_index {
            0 => 96_000,
            1 => 88_200,
            2 => 64_000,
            3 => 48_000,
            4 => 44_100,
            5 => 32_000,
            6 => 24_000,
            7 => 22_050,
            8 => 16_000,
            9 => 12_000,
            10 => 11_025,
            11 => 8_000,
            12 => 7_350,
            _ => 48_000,
        }
    }
}

fn parse_adts_header(data: &[u8]) -> crate::Result<AdtsHeader> {
    if data.len() < 7 {
        return Err(crate::Error::new("ADTS header too short"));
    }

    if data[0] != 0xFF || (data[1] & 0xF0) != 0xF0 {
        return Err(crate::Error::new("invalid ADTS sync word"));
    }

    let protection_absent = (data[1] & 0x01) != 0;
    let sampling_frequency_index = (data[2] >> 2) & 0x0F;
    let channel_configuration = ((data[2] & 0x01) as u16) << 2 | ((data[3] >> 6) & 0x03) as u16;
    let frame_length =
        ((data[3] & 0x03) as u16) << 11 | (data[4] as u16) << 3 | ((data[5] >> 5) & 0x07) as u16;

    Ok(AdtsHeader {
        protection_absent,
        sampling_frequency_index,
        channel_configuration,
        frame_length,
    })
}
