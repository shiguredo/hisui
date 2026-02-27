use std::rc::Rc;
use std::time::{Duration, Instant};

use shiguredo_http11::{Request, Response, uri::Uri};
use shiguredo_webrtc::{
    AdaptedVideoTrackSource, CxxString, IceServer, MediaType, PeerConnection,
    PeerConnectionDependencies, PeerConnectionFactory, PeerConnectionObserver,
    PeerConnectionObserverBuilder, PeerConnectionRtcConfiguration, RtpCodecCapabilityVector,
    RtpTransceiverDirection, RtpTransceiverInit,
};
use tokio::io::AsyncWriteExt;

use crate::{Error, MediaFrame, Message, ProcessorHandle, TrackId};

const AV_SYNC_LOG_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct WhipPublisher {
    pub output_url: String,
    pub input_video_track_id: Option<TrackId>,
    pub input_audio_track_id: Option<TrackId>,
    pub bearer_token: Option<String>,
    pub video_codec_preferences: Vec<VideoCodecPreference>,
}

impl nojson::DisplayJson for WhipPublisher {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("outputUrl", &self.output_url)?;
            if let Some(track_id) = &self.input_video_track_id {
                f.member("inputVideoTrackId", track_id)?;
            }
            if let Some(track_id) = &self.input_audio_track_id {
                f.member("inputAudioTrackId", track_id)?;
            }
            if let Some(token) = &self.bearer_token {
                f.member("bearerToken", token)?;
            }
            f.member("videoCodecPreferences", &self.video_codec_preferences)
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for WhipPublisher {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let output_url: String = value.to_member("outputUrl")?.required()?.try_into()?;
        if let Err(e) = validate_output_url(&output_url) {
            return Err(value.to_member("outputUrl")?.required()?.invalid(e));
        }

        let input_video_track_id: Option<TrackId> =
            value.to_member("inputVideoTrackId")?.try_into()?;
        let input_audio_track_id: Option<TrackId> =
            value.to_member("inputAudioTrackId")?.try_into()?;
        let bearer_token: Option<String> = value.to_member("bearerToken")?.try_into()?;
        let bearer_token = match bearer_token {
            Some(token) => {
                let token = token.trim();
                if token.is_empty() {
                    return Err(value
                        .to_member("bearerToken")?
                        .required()?
                        .invalid("bearerToken must not be empty"));
                }
                Some(token.to_owned())
            }
            None => None,
        };
        let codecs_raw: Option<Vec<String>> =
            value.to_member("videoCodecPreferences")?.try_into()?;
        let video_codec_preferences =
            parse_video_codec_preferences(codecs_raw).map_err(|e| value.invalid(e))?;

        Ok(Self {
            output_url,
            input_video_track_id,
            input_audio_track_id,
            bearer_token,
            video_codec_preferences,
        })
    }
}

impl WhipPublisher {
    pub async fn run(self, handle: ProcessorHandle) -> crate::Result<()> {
        handle.notify_ready();

        tracing::info!("WHIP session connecting: output_url={}", self.output_url);
        let connect_started_at = Instant::now();
        let mut session = WhipSession::connect(
            &self.output_url,
            self.bearer_token.as_deref(),
            self.input_video_track_id.is_some(),
            &self.video_codec_preferences,
        )
        .await?;
        tracing::info!(
            "WHIP session connected: elapsed_ms={}",
            connect_started_at.elapsed().as_millis()
        );

        let mut video_rx = self
            .input_video_track_id
            .clone()
            .map(|track_id| handle.subscribe_track(track_id));
        let mut audio_rx = self
            .input_audio_track_id
            .clone()
            .map(|track_id| handle.subscribe_track(track_id));
        tracing::info!(
            "WHIP input tracks configured: video_track={}, audio_track={}",
            self.input_video_track_id
                .as_ref()
                .map(|id| id.get())
                .unwrap_or("<none>"),
            self.input_audio_track_id
                .as_ref()
                .map(|id| id.get())
                .unwrap_or("<none>")
        );

        if video_rx.is_none() && audio_rx.is_none() {
            tracing::warn!("WHIP publisher has no input tracks; waiting indefinitely");
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        }

        let run_result = async {
            loop {
                let mut close_video = false;
                let mut close_audio = false;
                match (video_rx.as_mut(), audio_rx.as_mut()) {
                    (Some(video_rx), Some(audio_rx)) => {
                        tokio::select! {
                            message = video_rx.recv() => {
                                if handle_video_message(&mut session, &self.input_video_track_id, message)? {
                                    close_video = true;
                                }
                            }
                            message = audio_rx.recv() => {
                                if handle_audio_message(&mut session, &self.input_audio_track_id, message)? {
                                    close_audio = true;
                                }
                            }
                        }
                    }
                    (Some(video_rx), None) => {
                        if handle_video_message(
                            &mut session,
                            &self.input_video_track_id,
                            video_rx.recv().await,
                        )? {
                            close_video = true;
                        }
                    }
                    (None, Some(audio_rx)) => {
                        if handle_audio_message(
                            &mut session,
                            &self.input_audio_track_id,
                            audio_rx.recv().await,
                        )? {
                            close_audio = true;
                        }
                    }
                    (None, None) => break,
                }

                if close_video {
                    video_rx = None;
                }
                if close_audio {
                    audio_rx = None;
                }
            }
            Ok(())
        }
        .await;

        session.disconnect().await;
        run_result
    }
}

fn handle_video_message(
    session: &mut WhipSession,
    track_id: &Option<TrackId>,
    message: Message,
) -> crate::Result<bool> {
    match message {
        Message::Media(MediaFrame::Video(frame)) => {
            session.push_video_frame(&frame)?;
            Ok(false)
        }
        Message::Media(MediaFrame::Audio(_)) => Err(Error::new(format!(
            "expected a video sample on track {}, but got an audio sample",
            track_id.as_ref().map(|id| id.get()).unwrap_or("<none>")
        ))),
        Message::Eos => Ok(true),
        Message::Syn(_) => Ok(false),
    }
}

fn handle_audio_message(
    session: &mut WhipSession,
    track_id: &Option<TrackId>,
    message: Message,
) -> crate::Result<bool> {
    match message {
        Message::Media(MediaFrame::Audio(frame)) => {
            session.push_audio_frame(&frame)?;
            Ok(false)
        }
        Message::Media(MediaFrame::Video(_)) => Err(Error::new(format!(
            "expected an audio sample on track {}, but got a video sample",
            track_id.as_ref().map(|id| id.get()).unwrap_or("<none>")
        ))),
        Message::Eos => Ok(true),
        Message::Syn(_) => Ok(false),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodecPreference {
    Av1,
    H264,
    Vp8,
    Vp9,
}

impl VideoCodecPreference {
    fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "av1" => Some(Self::Av1),
            "h264" => Some(Self::H264),
            "vp8" => Some(Self::Vp8),
            "vp9" => Some(Self::Vp9),
            _ => None,
        }
    }

    fn as_name(self) -> &'static str {
        match self {
            Self::Av1 => "AV1",
            Self::H264 => "H264",
            Self::Vp8 => "VP8",
            Self::Vp9 => "VP9",
        }
    }
}

impl nojson::DisplayJson for VideoCodecPreference {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(self.as_name())
    }
}

struct WhipSession {
    /// `PeerConnectionFactory` のスコープを保持するために参照を持つ
    _factory_bundle: Rc<crate::webrtc_factory::WebRtcFactoryBundle>,
    /// `PeerConnectionObserver` のコールバック登録を維持するために参照を持つ
    _observer: PeerConnectionObserver,
    pc: Option<PeerConnection>,
    video_source: Option<AdaptedVideoTrackSource>,
    audio_sink: crate::webrtc_audio::WebRtcAudioTransportSink,
    /// `VideoTrack` の生存期間を維持するために参照を持つ
    _video_track: Option<shiguredo_webrtc::VideoTrack>,
    resource_url: Option<String>,
    bearer_token: Option<String>,
    negotiated_video_codecs: Vec<String>,
    av_sync_metrics: AvSyncMetrics,
}

impl WhipSession {
    async fn connect(
        output_url: &str,
        bearer_token: Option<&str>,
        create_video_track: bool,
        video_codec_preferences: &[VideoCodecPreference],
    ) -> crate::Result<Self> {
        let (factory_bundle, audio_sink) =
            crate::webrtc_factory::WebRtcFactoryBundle::new_with_audio_transport_sink()?;
        let factory_bundle = Rc::new(factory_bundle);
        let factory = factory_bundle.factory();

        let observer = PeerConnectionObserverBuilder::new()
            .on_connection_change(|state| {
                tracing::info!("WHIP PeerConnection state changed: {state:?}");
            })
            .on_ice_candidate(|candidate| {
                let sdp_mid = candidate
                    .sdp_mid()
                    .unwrap_or_else(|_| "<unknown>".to_owned());
                let sdp_mline_index = candidate.sdp_mline_index();
                match candidate.to_string() {
                    Ok(c) => {
                        tracing::debug!(
                            "WHIP local ICE candidate gathered: sdpMid={sdp_mid}, sdpMLineIndex={sdp_mline_index}, candidate={c}"
                        );
                    }
                    Err(e) => {
                        tracing::debug!(
                            "WHIP local ICE candidate gathered: sdpMid={sdp_mid}, sdpMLineIndex={sdp_mline_index}, candidate=<failed to stringify: {e}>"
                        );
                    }
                }
            })
            .build();
        let mut deps = PeerConnectionDependencies::new(&observer);
        let mut pc_config = PeerConnectionRtcConfiguration::new();
        let mut pc = PeerConnection::create(factory.as_ref(), &mut pc_config, &mut deps)
            .map_err(|e| Error::new(format!("failed to create PeerConnection: {e}")))?;

        add_audio_transceiver(&pc, factory.as_ref())?;

        let (video_source, video_track) = if create_video_track {
            let video_source = AdaptedVideoTrackSource::new();
            let video_track_source = video_source.cast_to_video_track_source();
            let track_id = shiguredo_webrtc::random_string(16);
            let video_track = factory
                .create_video_track(&video_track_source, &track_id)
                .map_err(|e| Error::new(format!("failed to create video track: {e}")))?;

            let mut init = RtpTransceiverInit::new();
            init.set_direction(RtpTransceiverDirection::SendOnly);
            let mut stream_ids = init.stream_ids();
            let stream_id = CxxString::from_str(&shiguredo_webrtc::random_string(16));
            stream_ids.push(&stream_id);

            let mut transceiver = pc
                .add_transceiver_with_track(&video_track, &mut init)
                .map_err(|e| Error::new(format!("failed to add video transceiver: {e}")))?;
            let codecs = select_video_codecs(factory.as_ref(), video_codec_preferences)?;
            transceiver
                .set_codec_preferences(&codecs)
                .map_err(|e| Error::new(format!("failed to set video codec preferences: {e}")))?;
            (Some(video_source), Some(video_track))
        } else {
            let mut init = RtpTransceiverInit::new();
            init.set_direction(RtpTransceiverDirection::SendOnly);
            let mut transceiver = pc
                .add_transceiver(MediaType::Video, &mut init)
                .map_err(|e| Error::new(format!("failed to add video transceiver: {e}")))?;
            let codecs = select_video_codecs(factory.as_ref(), video_codec_preferences)?;
            transceiver
                .set_codec_preferences(&codecs)
                .map_err(|e| Error::new(format!("failed to set video codec preferences: {e}")))?;
            (None, None)
        };

        let (resource_url, negotiated_video_codecs) =
            exchange_offer_answer(&mut pc, output_url, bearer_token).await?;

        Ok(Self {
            _factory_bundle: factory_bundle,
            _observer: observer,
            pc: Some(pc),
            audio_sink,
            video_source,
            _video_track: video_track,
            resource_url,
            bearer_token: bearer_token.map(str::to_owned),
            negotiated_video_codecs,
            av_sync_metrics: AvSyncMetrics::default(),
        })
    }

    fn push_video_frame(&mut self, frame: &crate::VideoFrame) -> crate::Result<()> {
        self.av_sync_metrics.observe_video_frame(
            frame.timestamp,
            self.negotiated_video_codecs.first().map(String::as_str),
        );
        let source = self
            .video_source
            .as_mut()
            .ok_or_else(|| Error::new("video track is not configured"))?;
        crate::webrtc_video::push_i420_frame(source, frame)
    }

    fn push_audio_frame(&mut self, frame: &crate::AudioFrame) -> crate::Result<()> {
        self.av_sync_metrics.observe_audio_frame(
            frame.timestamp,
            self.negotiated_video_codecs.first().map(String::as_str),
        );
        match self.audio_sink.push_i16be_stereo_48khz(frame) {
            Ok(()) => Ok(()),
            Err(e) if e.reason == "audio transport is not ready" => Ok(()),
            Err(e) => Err(e),
        }
    }

    async fn disconnect(&mut self) {
        self.pc = None;
        if let Some(resource_url) = self.resource_url.take() {
            match crate::webrtc_http::send_delete_resource(
                &resource_url,
                self.bearer_token.as_deref(),
                "Hisui-WhipPublisher",
                "output URL",
                "outputUrl",
                "WHIP",
            )
            .await
            {
                Ok(()) => tracing::info!("WHIP resource deleted: {resource_url}"),
                Err(e) => tracing::warn!("failed to delete WHIP resource: {}", e.display()),
            }
        }
    }
}

#[derive(Debug, Default)]
struct AvSyncMetrics {
    first_audio_timestamp: Option<Duration>,
    first_video_timestamp: Option<Duration>,
    last_audio_timestamp: Option<Duration>,
    last_video_timestamp: Option<Duration>,
    last_audio_arrival: Option<Instant>,
    last_video_arrival: Option<Instant>,
    min_av_diff_us: Option<i128>,
    max_av_diff_us: Option<i128>,
    last_log_at: Option<Instant>,
}

impl AvSyncMetrics {
    fn observe_audio_frame(&mut self, timestamp: Duration, negotiated_video_codec: Option<&str>) {
        let now = Instant::now();
        if self.first_audio_timestamp.is_none() {
            self.first_audio_timestamp = Some(timestamp);
        }
        self.last_audio_timestamp = Some(timestamp);
        self.last_audio_arrival = Some(now);
        self.maybe_log(now, negotiated_video_codec);
    }

    fn observe_video_frame(&mut self, timestamp: Duration, negotiated_video_codec: Option<&str>) {
        let now = Instant::now();
        if self.first_video_timestamp.is_none() {
            self.first_video_timestamp = Some(timestamp);
        }
        self.last_video_timestamp = Some(timestamp);
        self.last_video_arrival = Some(now);
        self.maybe_log(now, negotiated_video_codec);
    }

    fn maybe_log(&mut self, now: Instant, negotiated_video_codec: Option<&str>) {
        let Some(audio_timestamp) = self.last_audio_timestamp else {
            return;
        };
        let Some(video_timestamp) = self.last_video_timestamp else {
            return;
        };

        let audio_timestamp_us = duration_to_i128_micros(audio_timestamp);
        let video_timestamp_us = duration_to_i128_micros(video_timestamp);
        let av_diff_us = audio_timestamp_us - video_timestamp_us;
        self.min_av_diff_us = Some(
            self.min_av_diff_us
                .map_or(av_diff_us, |v| v.min(av_diff_us)),
        );
        self.max_av_diff_us = Some(
            self.max_av_diff_us
                .map_or(av_diff_us, |v| v.max(av_diff_us)),
        );

        if let Some(last_log_at) = self.last_log_at
            && now.duration_since(last_log_at) < AV_SYNC_LOG_INTERVAL
        {
            return;
        }
        self.last_log_at = Some(now);

        let arrival_diff_us = match (self.last_audio_arrival, self.last_video_arrival) {
            (Some(audio_arrival), Some(video_arrival)) => {
                Some(signed_duration_diff_micros(audio_arrival, video_arrival))
            }
            _ => None,
        };

        tracing::debug!(
            "WHIP AV sync metrics: first_audio_timestamp_us={:?}, first_video_timestamp_us={:?}, audio_timestamp_us={}, video_timestamp_us={}, av_diff_us={}, arrival_diff_us={:?}, min_av_diff_us={:?}, max_av_diff_us={:?}, negotiated_video_codec={}",
            self.first_audio_timestamp.map(|v| v.as_micros()),
            self.first_video_timestamp.map(|v| v.as_micros()),
            audio_timestamp_us,
            video_timestamp_us,
            av_diff_us,
            arrival_diff_us,
            self.min_av_diff_us,
            self.max_av_diff_us,
            negotiated_video_codec.unwrap_or("<unknown>")
        );
    }
}

fn signed_duration_diff_micros(lhs: Instant, rhs: Instant) -> i128 {
    if lhs >= rhs {
        duration_to_i128_micros(lhs.duration_since(rhs))
    } else {
        -duration_to_i128_micros(rhs.duration_since(lhs))
    }
}

fn duration_to_i128_micros(duration: Duration) -> i128 {
    i128::try_from(duration.as_micros()).unwrap_or(i128::MAX)
}

fn validate_output_url(output_url: &str) -> Result<(), String> {
    let uri = Uri::parse(output_url).map_err(|e| e.to_string())?;
    let scheme = uri
        .scheme()
        .ok_or_else(|| "outputUrl must contain URL scheme".to_owned())?;
    if scheme != "http" && scheme != "https" {
        return Err("outputUrl scheme must be http or https".to_owned());
    }
    uri.host()
        .ok_or_else(|| "outputUrl must contain host".to_owned())?;
    Ok(())
}

fn parse_video_codec_preferences(
    codecs_raw: Option<Vec<String>>,
) -> Result<Vec<VideoCodecPreference>, String> {
    let default = vec![
        VideoCodecPreference::Av1,
        VideoCodecPreference::H264,
        VideoCodecPreference::Vp8,
    ];
    let Some(codecs_raw) = codecs_raw else {
        return Ok(default);
    };
    if codecs_raw.is_empty() {
        return Err("videoCodecPreferences must not be empty".to_owned());
    }

    let mut codecs = Vec::with_capacity(codecs_raw.len());
    for codec in codecs_raw {
        let Some(codec) = VideoCodecPreference::from_name(&codec) else {
            return Err(format!(
                "unsupported video codec in videoCodecPreferences: {codec}"
            ));
        };
        codecs.push(codec);
    }

    Ok(codecs)
}

fn add_audio_transceiver(
    pc: &PeerConnection,
    factory: &PeerConnectionFactory,
) -> crate::Result<()> {
    let mut init = RtpTransceiverInit::new();
    init.set_direction(RtpTransceiverDirection::SendOnly);
    let mut transceiver = pc
        .add_transceiver(MediaType::Audio, &mut init)
        .map_err(|e| Error::new(format!("failed to add audio transceiver: {e}")))?;

    let caps = factory.get_rtp_sender_capabilities(MediaType::Audio);
    let mut codecs = RtpCodecCapabilityVector::new(0);
    let source = caps.codecs();
    for i in 0..source.len() {
        let Some(codec) = source.get(i) else {
            continue;
        };
        let name = codec
            .name()
            .map_err(|e| Error::new(format!("failed to get audio codec name: {e}")))?;
        if name.eq_ignore_ascii_case("opus") {
            codecs.push_ref(&codec);
            break;
        }
    }
    if codecs.is_empty() {
        return Err(Error::new(
            "Opus codec is not available in this WebRTC build",
        ));
    }

    transceiver
        .set_codec_preferences(&codecs)
        .map_err(|e| Error::new(format!("failed to set audio codec preferences: {e}")))?;

    Ok(())
}

fn select_video_codecs(
    factory: &PeerConnectionFactory,
    preferences: &[VideoCodecPreference],
) -> crate::Result<RtpCodecCapabilityVector> {
    let caps = factory.get_rtp_sender_capabilities(MediaType::Video);
    let source = caps.codecs();
    let mut selected = std::collections::BTreeSet::new();
    let mut codecs = RtpCodecCapabilityVector::new(0);

    for preferred in preferences {
        let expected = preferred.as_name().to_ascii_lowercase();
        for i in 0..source.len() {
            let Some(codec) = source.get(i) else {
                continue;
            };
            let name = codec
                .name()
                .map_err(|e| Error::new(format!("failed to get video codec name: {e}")))?
                .to_ascii_lowercase();
            if name == expected && selected.insert(name) {
                codecs.push_ref(&codec);
            }
        }
    }

    // RTX はメインコーデックに付随するため、可能なら最後に追加する。
    for i in 0..source.len() {
        let Some(codec) = source.get(i) else {
            continue;
        };
        let name = codec
            .name()
            .map_err(|e| Error::new(format!("failed to get video codec name: {e}")))?
            .to_ascii_lowercase();
        if name == "rtx" && selected.insert(name) {
            codecs.push_ref(&codec);
        }
    }

    if codecs.is_empty() {
        let names = preferences
            .iter()
            .map(|c| c.as_name())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(Error::new(format!(
            "none of requested video codecs are supported: {names}"
        )));
    }

    Ok(codecs)
}

async fn exchange_offer_answer(
    pc: &mut PeerConnection,
    output_url: &str,
    bearer_token: Option<&str>,
) -> crate::Result<(Option<String>, Vec<String>)> {
    let offer_sdp = crate::webrtc_sdp::create_offer_sdp(pc)?;
    log_sdp_candidates("WHIP offer", &offer_sdp);

    let response = send_offer(output_url, bearer_token, &offer_sdp).await?;
    if response.status_code != 201 {
        return Err(Error::new(format!(
            "WHIP endpoint returned unexpected status code: {}",
            response.status_code
        )));
    }

    // Link ヘッダーの ICE server を先に適用してから local offer をセットする。
    // これにより、ICE 候補収集開始時点で TURN/STUN 設定を反映できる。
    apply_ice_servers_from_link_header(pc, &response)?;
    crate::webrtc_sdp::set_local_offer(pc, &offer_sdp)?;

    let location = response.get_header("Location").map(str::to_owned);
    let answer_sdp = String::from_utf8(response.body)
        .map_err(|e| Error::new(format!("failed to decode answer SDP as UTF-8: {e}")))?;
    if answer_sdp.trim().is_empty() {
        return Err(Error::new("WHIP endpoint returned empty answer SDP"));
    }
    log_sdp_candidates("WHIP answer", &answer_sdp);
    let answer_video_codecs = audit_offer_and_answer_sdp(&offer_sdp, &answer_sdp);
    crate::webrtc_sdp::set_remote_answer(pc, &answer_sdp)?;

    let resource_url = match location.as_deref() {
        Some(location) => match crate::webrtc_http::resolve_resource_url(output_url, location) {
            Ok(url) => Some(url),
            Err(e) => {
                tracing::warn!(
                    "failed to resolve WHIP resource URL from Location header: {}",
                    e.display()
                );
                None
            }
        },
        None => {
            tracing::debug!("WHIP response does not contain Location header");
            None
        }
    };

    Ok((resource_url, answer_video_codecs))
}

fn apply_ice_servers_from_link_header(
    pc: &mut PeerConnection,
    response: &Response,
) -> crate::Result<()> {
    let Some(link_header) = response.get_header("Link") else {
        tracing::debug!("WHIP response does not contain Link header for ICE servers");
        return Ok(());
    };
    let parsed = crate::webrtc_http::parse_link_header(link_header);
    if parsed.urls.is_empty() {
        tracing::debug!("WHIP Link header does not include ICE server URLs");
        return Ok(());
    }
    tracing::debug!(
        "WHIP Link header parsed: urls={:?}, username_present={}, credential_present={}",
        parsed.urls,
        parsed.username.is_some(),
        parsed.credential.is_some()
    );

    let mut config = PeerConnectionRtcConfiguration::new();
    let mut server = IceServer::new();
    for url in parsed.urls {
        server.add_url(&url);
    }
    if let Some(username) = parsed.username {
        server.set_username(&username);
    }
    if let Some(credential) = parsed.credential {
        server.set_password(&credential);
    }
    config.servers().push(&server);

    pc.set_configuration(&mut config)
        .map_err(|e| Error::new(format!("failed to apply ICE servers from Link header: {e}")))?;

    Ok(())
}

fn log_sdp_candidates(label: &str, sdp: &str) {
    let mut candidates = Vec::new();
    let mut has_end_of_candidates = false;

    for line in sdp.lines() {
        let line = line.trim();
        if line.starts_with("a=candidate:") {
            candidates.push(line);
        } else if line == "a=end-of-candidates" {
            has_end_of_candidates = true;
        }
    }

    tracing::debug!(
        "{label} SDP candidate summary: count={}, end_of_candidates={}",
        candidates.len(),
        has_end_of_candidates
    );

    for line in candidates.iter().take(10) {
        tracing::debug!("{label} SDP candidate: {line}");
    }
    if candidates.len() > 10 {
        tracing::debug!(
            "{label} SDP candidate lines are omitted: {}",
            candidates.len() - 10
        );
    }
}

#[derive(Debug, Clone)]
struct SdpMediaSection {
    kind: String,
    port: Option<u16>,
    payload_types: Vec<String>,
    mid: Option<String>,
    direction: Option<String>,
    msid_present: bool,
    codecs: Vec<String>,
}

#[derive(Debug, Default, Clone)]
struct SdpSummary {
    bundle_mids: Vec<String>,
    media_sections: Vec<SdpMediaSection>,
}

fn parse_sdp_summary(sdp: &str) -> SdpSummary {
    let mut summary = SdpSummary::default();
    let mut current_section: Option<SdpMediaSection> = None;

    for raw_line in sdp.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("a=group:BUNDLE") {
            summary.bundle_mids = rest
                .split_whitespace()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            continue;
        }

        if let Some(rest) = line.strip_prefix("m=") {
            if let Some(section) = current_section.take() {
                summary.media_sections.push(section);
            }

            let mut fields = rest.split_whitespace();
            let kind = fields.next().unwrap_or_default().to_owned();
            let port = fields.next().and_then(|v| v.parse::<u16>().ok());
            let _proto = fields.next();
            let payload_types = fields.map(ToOwned::to_owned).collect::<Vec<_>>();

            current_section = Some(SdpMediaSection {
                kind,
                port,
                payload_types,
                mid: None,
                direction: None,
                msid_present: false,
                codecs: Vec::new(),
            });
            continue;
        }

        let Some(section) = current_section.as_mut() else {
            continue;
        };

        if let Some(mid) = line.strip_prefix("a=mid:") {
            section.mid = Some(mid.to_owned());
            continue;
        }
        if let Some(msid) = line.strip_prefix("a=msid:")
            && !msid.is_empty()
        {
            section.msid_present = true;
            continue;
        }
        if matches!(
            line,
            "a=sendrecv" | "a=sendonly" | "a=recvonly" | "a=inactive"
        ) {
            section.direction = Some(line["a=".len()..].to_owned());
            continue;
        }
        if let Some(rest) = line.strip_prefix("a=rtpmap:")
            && let Some((payload_type, format)) = rest.split_once(' ')
        {
            let codec = format
                .split('/')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !codec.is_empty()
                && section.payload_types.iter().any(|v| v == payload_type)
                && !section.codecs.iter().any(|v| v == &codec)
            {
                section.codecs.push(codec);
            }
        }
    }

    if let Some(section) = current_section.take() {
        summary.media_sections.push(section);
    }

    summary
}

fn audit_offer_and_answer_sdp(offer_sdp: &str, answer_sdp: &str) -> Vec<String> {
    let offer = parse_sdp_summary(offer_sdp);
    let answer = parse_sdp_summary(answer_sdp);

    log_sdp_summary("WHIP offer", &offer);
    log_sdp_summary("WHIP answer", &answer);

    for issue in validate_sdp_summary("offer", &offer) {
        tracing::warn!("WHIP SDP audit issue: {issue}");
    }
    for issue in validate_sdp_summary("answer", &answer) {
        tracing::warn!("WHIP SDP audit issue: {issue}");
    }
    for issue in validate_offer_answer_compatibility(&offer, &answer) {
        tracing::warn!("WHIP SDP audit issue: {issue}");
    }

    answer
        .media_sections
        .iter()
        .find(|section| section.kind == "video")
        .map(|section| section.codecs.clone())
        .unwrap_or_default()
}

fn log_sdp_summary(label: &str, summary: &SdpSummary) {
    tracing::debug!(
        "{label} SDP media summary: media_count={}, bundle_mids={:?}",
        summary.media_sections.len(),
        summary.bundle_mids
    );
    for section in &summary.media_sections {
        tracing::debug!(
            "{label} SDP media: kind={}, mid={}, port={:?}, direction={}, msid_present={}, payload_types={:?}, codecs={:?}",
            section.kind,
            section.mid.as_deref().unwrap_or("<missing>"),
            section.port,
            section.direction.as_deref().unwrap_or("<missing>"),
            section.msid_present,
            section.payload_types,
            section.codecs
        );
    }
}

fn validate_sdp_summary(label: &str, summary: &SdpSummary) -> Vec<String> {
    let mut issues = Vec::new();
    let has_audio = summary
        .media_sections
        .iter()
        .any(|section| section.kind == "audio");
    let has_video = summary
        .media_sections
        .iter()
        .any(|section| section.kind == "video");
    if !has_audio {
        issues.push(format!("{label}: audio m-line is missing"));
    }
    if !has_video {
        issues.push(format!("{label}: video m-line is missing"));
    }

    if summary.bundle_mids.is_empty() {
        issues.push(format!("{label}: a=group:BUNDLE is missing or empty"));
    }

    let mut mids = std::collections::BTreeSet::new();
    for section in &summary.media_sections {
        let Some(mid) = section.mid.as_deref() else {
            issues.push(format!("{label}: {} m-line is missing a=mid", section.kind));
            continue;
        };
        if !mids.insert(mid.to_owned()) {
            issues.push(format!("{label}: duplicate mid detected: {mid}"));
        }
        if !summary
            .bundle_mids
            .iter()
            .any(|bundle_mid| bundle_mid == mid)
        {
            issues.push(format!(
                "{label}: mid {mid} is not listed in a=group:BUNDLE"
            ));
        }
        if section.direction.is_none() {
            issues.push(format!(
                "{label}: {} m-line (mid={mid}) is missing direction attribute",
                section.kind
            ));
        }
        if matches!(
            section.direction.as_deref(),
            Some("sendrecv") | Some("sendonly")
        ) && !section.msid_present
        {
            issues.push(format!(
                "{label}: {} m-line (mid={mid}) is missing a=msid",
                section.kind
            ));
        }
    }

    issues
}

fn validate_offer_answer_compatibility(offer: &SdpSummary, answer: &SdpSummary) -> Vec<String> {
    let mut issues = Vec::new();
    let answer_by_mid = answer
        .media_sections
        .iter()
        .filter_map(|section| section.mid.as_deref().map(|mid| (mid, section)))
        .collect::<std::collections::BTreeMap<_, _>>();

    for offer_section in &offer.media_sections {
        let Some(mid) = offer_section.mid.as_deref() else {
            continue;
        };

        let Some(answer_section) = answer_by_mid.get(mid) else {
            issues.push(format!("answer: m-line for mid {mid} is missing"));
            continue;
        };

        if answer_section.kind != offer_section.kind {
            issues.push(format!(
                "answer: mid {mid} media kind mismatch: offer={}, answer={}",
                offer_section.kind, answer_section.kind
            ));
        }

        if answer_section.port == Some(0) {
            issues.push(format!("answer: mid {mid} is rejected (port=0)"));
        }

        if offer_section.direction.as_deref() != Some("sendonly") {
            issues.push(format!(
                "offer: mid {mid} direction is not sendonly: {}",
                offer_section.direction.as_deref().unwrap_or("<missing>")
            ));
        }

        if matches!(answer_section.direction.as_deref(), Some("sendonly")) {
            issues.push(format!(
                "answer: mid {mid} direction is sendonly, expected recvonly or inactive"
            ));
        }
    }

    issues
}

async fn send_offer(
    output_url: &str,
    bearer_token: Option<&str>,
    offer_sdp: &str,
) -> crate::Result<Response> {
    let target = crate::webrtc_http::build_request_target(output_url, "output URL", "outputUrl")?;
    let mut request = Request::new("POST", &target.path_and_query)
        .header("Host", &target.host_header)
        .header("Content-Type", "application/sdp")
        .header("Connection", "close")
        .header("User-Agent", "Hisui-WhipPublisher");
    let authorization = bearer_token.map(|token| format!("Bearer {token}"));
    if let Some(value) = authorization.as_deref() {
        request = request.header("Authorization", value);
    }
    let request = request.body(offer_sdp.as_bytes().to_vec());

    let mut stream = crate::tcp::TcpOrTlsStream::connect(&target.host, target.port, target.tls)
        .await
        .map_err(|e| Error::new(format!("failed to connect WHIP endpoint: {e}")))?;
    stream
        .write_all(&request.encode())
        .await
        .map_err(|e| Error::new(format!("failed to send WHIP request: {e}")))?;
    stream
        .flush()
        .await
        .map_err(|e| Error::new(format!("failed to flush WHIP request: {e}")))?;

    crate::webrtc_http::read_http_response(&mut stream, "WHIP").await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whip_publisher_params_defaults_are_applied() {
        let json = r#"{
            "outputUrl":"https://example.com/whip/live",
            "inputVideoTrackId":"video-main"
        }"#;
        let publisher: WhipPublisher = crate::json::parse_str(json).expect("parse");

        assert_eq!(publisher.output_url, "https://example.com/whip/live");
        assert_eq!(
            publisher.input_video_track_id.as_ref().map(|id| id.get()),
            Some("video-main")
        );
        assert!(publisher.input_audio_track_id.is_none());
        assert!(publisher.bearer_token.is_none());
        assert_eq!(
            publisher.video_codec_preferences,
            vec![
                VideoCodecPreference::Av1,
                VideoCodecPreference::H264,
                VideoCodecPreference::Vp8,
            ]
        );
    }

    #[test]
    fn whip_publisher_rejects_invalid_url_scheme() {
        let json = r#"{
            "outputUrl":"ws://example.com/whip/live",
            "inputVideoTrackId":"video-main"
        }"#;
        let result: crate::Result<WhipPublisher> = crate::json::parse_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn whip_publisher_rejects_unknown_video_codec() {
        let json = r#"{
            "outputUrl":"https://example.com/whip/live",
            "inputVideoTrackId":"video-main",
            "videoCodecPreferences":["AV1","H266"]
        }"#;
        let result: crate::Result<WhipPublisher> = crate::json::parse_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn whip_publisher_accepts_bearer_token() {
        let json = r#"{
            "outputUrl":"https://example.com/whip/live",
            "inputVideoTrackId":"video-main",
            "bearerToken":"  test-token  "
        }"#;
        let publisher: WhipPublisher = crate::json::parse_str(json).expect("parse");
        assert_eq!(publisher.bearer_token.as_deref(), Some("test-token"));
    }

    #[test]
    fn whip_publisher_rejects_empty_bearer_token() {
        let json = r#"{
            "outputUrl":"https://example.com/whip/live",
            "inputVideoTrackId":"video-main",
            "bearerToken":"   "
        }"#;
        let result: crate::Result<WhipPublisher> = crate::json::parse_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn whip_publisher_accepts_audio_track_id() {
        let json = r#"{
            "outputUrl":"https://example.com/whip/live",
            "inputAudioTrackId":"audio-main"
        }"#;
        let publisher: WhipPublisher = crate::json::parse_str(json).expect("parse");
        assert!(publisher.input_video_track_id.is_none());
        assert_eq!(
            publisher.input_audio_track_id.as_ref().map(|id| id.get()),
            Some("audio-main")
        );
    }

    #[test]
    fn whip_publisher_accepts_without_track_ids() {
        let json = r#"{
            "outputUrl":"https://example.com/whip/live"
        }"#;
        let publisher: WhipPublisher = crate::json::parse_str(json).expect("parse");
        assert!(publisher.input_video_track_id.is_none());
        assert!(publisher.input_audio_track_id.is_none());
    }

    #[test]
    fn parse_sdp_summary_extracts_bundle_and_media() {
        let sdp = "\
v=0
o=- 1 1 IN IP4 127.0.0.1
s=-
t=0 0
a=group:BUNDLE 0 1
m=audio 9 UDP/TLS/RTP/SAVPF 111
a=mid:0
a=sendonly
a=msid:stream-a track-a
a=rtpmap:111 opus/48000/2
m=video 9 UDP/TLS/RTP/SAVPF 39
a=mid:1
a=sendonly
a=msid:stream-v track-v
a=rtpmap:39 AV1/90000
";
        let summary = parse_sdp_summary(sdp);

        assert_eq!(summary.bundle_mids, vec!["0".to_owned(), "1".to_owned()]);
        assert_eq!(summary.media_sections.len(), 2);
        assert_eq!(summary.media_sections[0].kind, "audio");
        assert_eq!(summary.media_sections[1].kind, "video");
        assert_eq!(summary.media_sections[1].codecs, vec!["av1".to_owned()]);
    }

    #[test]
    fn validate_offer_answer_compatibility_accepts_sendonly_recvonly_pair() {
        let offer = parse_sdp_summary(
            "\
v=0
o=- 1 1 IN IP4 127.0.0.1
s=-
t=0 0
a=group:BUNDLE 0 1
m=audio 9 UDP/TLS/RTP/SAVPF 111
a=mid:0
a=sendonly
a=msid:stream-a track-a
a=rtpmap:111 opus/48000/2
m=video 9 UDP/TLS/RTP/SAVPF 39
a=mid:1
a=sendonly
a=msid:stream-v track-v
a=rtpmap:39 AV1/90000
",
        );
        let answer = parse_sdp_summary(
            "\
v=0
o=- 1 1 IN IP4 127.0.0.1
s=-
t=0 0
a=group:BUNDLE 0 1
m=audio 9 UDP/TLS/RTP/SAVPF 111
a=mid:0
a=recvonly
a=rtpmap:111 opus/48000/2
m=video 9 UDP/TLS/RTP/SAVPF 39
a=mid:1
a=recvonly
a=rtpmap:39 AV1/90000
",
        );

        let offer_issues = validate_sdp_summary("offer", &offer);
        let answer_issues = validate_sdp_summary("answer", &answer);
        let compatibility_issues = validate_offer_answer_compatibility(&offer, &answer);

        assert!(offer_issues.is_empty(), "{offer_issues:?}");
        assert!(answer_issues.is_empty(), "{answer_issues:?}");
        assert!(compatibility_issues.is_empty(), "{compatibility_issues:?}");
    }
}
