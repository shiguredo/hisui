use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

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
    Error, MessageSender, ProcessorHandle, TrackId,
    audio::{AudioFormat, AudioFrame, Channels, SampleRate},
    timestamp_mapper::TimestampMapper,
    video::{VideoFormat, VideoFrame},
};

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(20);
const RECONNECT_DELAY_INITIAL: Duration = Duration::from_millis(500);
const RECONNECT_DELAY_MAX: Duration = Duration::from_secs(5);
const AAC_AU_SAMPLES: u64 = 1024;
const DEFAULT_RTSP_PORT: u16 = 554;

#[derive(Debug, Clone)]
pub struct RtspSubscriber {
    pub input_url: String,
    pub output_video_track_id: Option<TrackId>,
    pub output_audio_track_id: Option<TrackId>,
}

impl nojson::DisplayJson for RtspSubscriber {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("inputUrl", &self.input_url)?;
            if let Some(track_id) = &self.output_video_track_id {
                f.member("outputVideoTrackId", track_id)?;
            }
            if let Some(track_id) = &self.output_audio_track_id {
                f.member("outputAudioTrackId", track_id)?;
            }
            Ok(())
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for RtspSubscriber {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let input_url: String = value.to_member("inputUrl")?.required()?.try_into()?;
        // TryFrom では nojson のエラー位置情報を維持したまま invalid(...) を返すため、
        // ここでは URL の妥当性チェックだけ行う。
        if let Err(e) = validate_input_url(&input_url) {
            return Err(value.to_member("inputUrl")?.required()?.invalid(e));
        }

        let output_video_track_id: Option<TrackId> =
            value.to_member("outputVideoTrackId")?.try_into()?;
        let output_audio_track_id: Option<TrackId> =
            value.to_member("outputAudioTrackId")?.try_into()?;
        if output_video_track_id.is_none() && output_audio_track_id.is_none() {
            return Err(value.invalid("outputAudioTrackId or outputVideoTrackId is required"));
        }

        Ok(Self {
            input_url,
            output_video_track_id,
            output_audio_track_id,
        })
    }
}

impl RtspSubscriber {
    pub async fn run(self, handle: ProcessorHandle) -> crate::Result<()> {
        let parsed_url = parse_rtsp_input_url(&self.input_url)
            .map_err(|e| Error::new(format!("invalid inputUrl: {e}")))?;
        let want_audio = self.output_audio_track_id.is_some();
        let want_video = self.output_video_track_id.is_some();

        let mut audio_track_tx = if let Some(track_id) = &self.output_audio_track_id {
            Some(handle.publish_track(track_id.clone()).await?)
        } else {
            None
        };
        let mut video_track_tx = if let Some(track_id) = &self.output_video_track_id {
            Some(handle.publish_track(track_id.clone()).await?)
        } else {
            None
        };

        let stats = RtspSubscriberStats::new(handle.stats());
        stats.set_connected(false);
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        let started_at = Instant::now();
        let mut reconnect_backoff = ReconnectBackoff::new();

        loop {
            let connection_offset = started_at.elapsed();
            let session_result = run_rtsp_session(
                &parsed_url,
                want_audio,
                want_video,
                connection_offset,
                &stats,
                &mut audio_track_tx,
                &mut video_track_tx,
            )
            .await;

            match session_result {
                Ok(()) => {
                    stats.set_connected(false);
                    reconnect_backoff.reset();
                    tracing::warn!("RTSP session closed; reconnecting");
                }
                Err(SessionError::Fatal(e)) => return Err(e),
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
}

#[derive(Debug)]
struct AudioRtpReceiver {
    rtp_channel: u8,
    payload_type: u8,
    timestamp_mapper: TimestampMapper,
    depacketizer: AacRtpDepacketizer,
    sample_rate: SampleRate,
    channels: Channels,
    sample_entry: SampleEntry,
    sent_sample_entry: bool,
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
    audio_track_tx: &mut Option<MessageSender>,
    video_track_tx: &mut Option<MessageSender>,
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

    runner
        .play_loop(audio_track_tx, video_track_tx, stats)
        .await
        .inspect_err(|_| {
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
                sample_entry: audio.sample_entry,
                sent_sample_entry: false,
            });
        }

        let keepalive_uri = self.keepalive_uri.clone();
        self.send_request_expect_success(RtspMethod::Play, &keepalive_uri, |req| req)
            .await?;

        Ok(())
    }

    async fn play_loop(
        &mut self,
        audio_track_tx: &mut Option<MessageSender>,
        video_track_tx: &mut Option<MessageSender>,
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
                    self.process_events(audio_track_tx, video_track_tx, stats)?;
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
        audio_track_tx: &mut Option<MessageSender>,
        video_track_tx: &mut Option<MessageSender>,
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
                    self.handle_rtp_packet(channel, packet, audio_track_tx, video_track_tx, stats)?
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
        audio_track_tx: &mut Option<MessageSender>,
        video_track_tx: &mut Option<MessageSender>,
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
                let video_frame = VideoFrame {
                    data: frame.data,
                    format: VideoFormat::H264AnnexB,
                    keyframe: frame.keyframe,
                    size: None,
                    timestamp,
                    sample_entry: None,
                };
                if let Some(tx) = video_track_tx.as_mut()
                    && !tx.send_video(video_frame)
                {
                    return Err(SessionError::Retryable(Error::new(
                        "video track sender is closed",
                    )));
                }
                stats.add_input_video_frame_count();
                stats.set_last_input_video_timestamp(timestamp);
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
                let sample_entry = if audio_receiver.sent_sample_entry {
                    None
                } else {
                    audio_receiver.sent_sample_entry = true;
                    Some(audio_receiver.sample_entry.clone())
                };
                let audio_frame = AudioFrame {
                    data: access_unit.data,
                    format: AudioFormat::Aac,
                    channels: audio_receiver.channels,
                    sample_rate: audio_receiver.sample_rate,
                    timestamp,
                    sample_entry,
                };
                if let Some(tx) = audio_track_tx.as_mut()
                    && !tx.send_audio(audio_frame)
                {
                    return Err(SessionError::Retryable(Error::new(
                        "audio track sender is closed",
                    )));
                }
                stats.add_input_audio_data_count();
                stats.set_last_input_audio_timestamp(timestamp);
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
                            "Digest auth requires credentials in inputUrl",
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
                if nal_unit_type == crate::video_h264::H264_NALU_TYPE_IDR {
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
                        && header & 0x1f == crate::video_h264::H264_NALU_TYPE_IDR
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
                    if reconstructed_nal & 0x1f == crate::video_h264::H264_NALU_TYPE_IDR {
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

#[derive(Debug, Clone)]
struct AudioAccessUnit {
    rtp_timestamp: u32,
    data: Vec<u8>,
}

#[derive(Debug)]
struct AacRtpDepacketizer {
    size_length: u8,
    index_length: u8,
    index_delta_length: u8,
}

impl AacRtpDepacketizer {
    fn new(size_length: u8, index_length: u8, index_delta_length: u8) -> Self {
        Self {
            size_length,
            index_length,
            index_delta_length,
        }
    }

    fn depacketize(
        &self,
        packet: &shiguredo_rtsp::RtpPacket,
    ) -> crate::Result<Vec<AudioAccessUnit>> {
        if packet.payload.len() < 2 {
            return Err(Error::new(
                "invalid AAC RTP payload: missing AU header length",
            ));
        }

        let au_headers_length_bits =
            u16::from_be_bytes([packet.payload[0], packet.payload[1]]) as usize;
        let au_headers_length_bytes = au_headers_length_bits.div_ceil(8);
        if packet.payload.len() < 2 + au_headers_length_bytes {
            return Err(Error::new(
                "invalid AAC RTP payload: AU headers are truncated",
            ));
        }

        let au_headers = &packet.payload[2..2 + au_headers_length_bytes];
        let mut bit_reader = BitReader::new(au_headers);
        let mut au_sizes = Vec::new();
        let mut first = true;
        let mut consumed_bits = 0usize;
        while consumed_bits < au_headers_length_bits {
            let size = bit_reader.read_bits(self.size_length)? as usize;
            consumed_bits = consumed_bits.saturating_add(self.size_length as usize);
            let index_bits = if first {
                self.index_length
            } else {
                self.index_delta_length
            };
            let _ = bit_reader.read_bits(index_bits)?;
            consumed_bits = consumed_bits.saturating_add(index_bits as usize);
            first = false;
            au_sizes.push(size);
        }

        let mut data_offset = 2 + au_headers_length_bytes;
        let mut access_units = Vec::with_capacity(au_sizes.len());
        for (index, au_size) in au_sizes.into_iter().enumerate() {
            if data_offset + au_size > packet.payload.len() {
                return Err(Error::new("invalid AAC RTP payload: AU data is truncated"));
            }

            let raw_timestamp = packet
                .header
                .timestamp
                .wrapping_add((index as u32).saturating_mul(AAC_AU_SAMPLES as u32));
            access_units.push(AudioAccessUnit {
                rtp_timestamp: raw_timestamp,
                data: packet.payload[data_offset..data_offset + au_size].to_vec(),
            });
            data_offset += au_size;
        }

        Ok(access_units)
    }
}

#[derive(Debug)]
struct BitReader<'a> {
    bytes: &'a [u8],
    bit_offset: usize,
}

impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            bit_offset: 0,
        }
    }

    fn read_bits(&mut self, bit_count: u8) -> crate::Result<u32> {
        if bit_count == 0 {
            return Ok(0);
        }
        let total_bits = self.bytes.len().saturating_mul(8);
        let end = self.bit_offset.saturating_add(bit_count as usize);
        if end > total_bits {
            return Err(Error::new("bitstream is truncated"));
        }

        let mut value = 0u32;
        for _ in 0..bit_count {
            let byte_index = self.bit_offset / 8;
            let bit_index = 7 - (self.bit_offset % 8);
            let bit = (self.bytes[byte_index] >> bit_index) & 1;
            value = (value << 1) | u32::from(bit);
            self.bit_offset += 1;
        }
        Ok(value)
    }
}

fn validate_input_url(input_url: &str) -> Result<(), String> {
    // 実行時の run では ParsedRtspUrl を接続に使うため再度 parse するが、
    // パラメータ検証では nojson 向けに文字列エラーへ変換する用途に限定する。
    parse_rtsp_input_url(input_url).map(|_| ())
}

fn parse_rtsp_input_url(input_url: &str) -> Result<ParsedRtspUrl, String> {
    let uri = Uri::parse(input_url).map_err(|e| format!("failed to parse URL: {e}"))?;
    let scheme = uri
        .scheme()
        .ok_or_else(|| "inputUrl must contain URL scheme".to_owned())?;
    let tls = match scheme {
        "rtsp" => false,
        "rtsps" => true,
        _ => return Err("inputUrl scheme must be rtsp or rtsps".to_owned()),
    };
    let host = uri
        .host()
        .ok_or_else(|| "inputUrl must contain host".to_owned())?
        .to_owned();
    let port = uri.port().unwrap_or(DEFAULT_RTSP_PORT);
    let authority = uri
        .authority()
        .ok_or_else(|| "inputUrl must contain authority".to_owned())?;
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
        return Err("inputUrl must not contain empty username".to_owned());
    }

    let (username, password) = match userinfo.split_once(':') {
        Some((username, password)) => (username, password),
        None => (userinfo, ""),
    };
    if username.is_empty() {
        return Err("inputUrl username must not be empty".to_owned());
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
            return Ok(Some(VideoTrackConfig {
                control_url,
                payload_type,
                clock_rate,
            }));
        }
    }

    Ok(None)
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
        let (sample_rate, channels) = crate::audio_aac::parse_audio_specific_config(&config)?;
        let sample_entry =
            crate::audio_aac::create_mp4a_sample_entry(&config, sample_rate, channels)?;
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

        if size_length == 0 {
            return Err(Error::new("AAC fmtp sizeLength must be greater than 0"));
        }

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
        assert_eq!(err, "inputUrl scheme must be rtsp or rtsps");
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
    fn depacketize_aac_with_multiple_aus() {
        let depacketizer = AacRtpDepacketizer::new(13, 3, 3);
        let mut header = shiguredo_rtsp::rtp::RtpHeader::new(97, 1, 9000, 1);
        header.marker = true;
        // AU-header-length: 32 bits
        // AU#0 size=4 index=0 -> 0000000000100 000
        // AU#1 size=2 index-delta=0 -> 0000000000010 000
        let payload = vec![
            0x00, 0x20, 0x00, 0x20, 0x00, 0x10, 0xaa, 0xbb, 0xcc, 0xdd, 0x11, 0x22,
        ];
        let packet = shiguredo_rtsp::RtpPacket {
            header,
            extension: None,
            payload,
            padding_size: 0,
        };

        let aus = depacketizer.depacketize(&packet).expect("must depacketize");
        assert_eq!(aus.len(), 2);
        assert_eq!(aus[0].rtp_timestamp, 9000);
        assert_eq!(aus[0].data, vec![0xaa, 0xbb, 0xcc, 0xdd]);
        assert_eq!(aus[1].rtp_timestamp, 10024);
        assert_eq!(aus[1].data, vec![0x11, 0x22]);
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
        let mut video_track_tx = None;

        let result = run_rtsp_session(
            &parsed_url,
            true,
            true,
            Duration::from_secs(3),
            &stats,
            &mut audio_track_tx,
            &mut video_track_tx,
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
        let mut video_track_tx = None;

        let result = run_rtsp_session(
            &parsed_url,
            false,
            true,
            Duration::from_secs(1),
            &stats,
            &mut audio_track_tx,
            &mut video_track_tx,
        )
        .await;

        assert!(result.is_err(), "session should end with error: {result:?}");
        server.wait().await.expect("server must finish cleanly");
    }

    #[tokio::test]
    async fn run_rtsp_session_fails_with_unsupported_video_codec() {
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
        let mut video_track_tx = None;

        let result = run_rtsp_session(
            &parsed_url,
            false,
            true,
            Duration::ZERO,
            &stats,
            &mut audio_track_tx,
            &mut video_track_tx,
        )
        .await;

        assert!(matches!(result, Err(SessionError::Fatal(_))));
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
                    if options.unsupported_video_codec {
                        return Ok(());
                    }
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
}
