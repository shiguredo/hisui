use std::rc::Rc;
use std::time::Duration;

use shiguredo_http11::{Request, Response, uri::Uri};
use shiguredo_webrtc::{
    AdaptedVideoTrackSource, CxxString, IceServer, MediaType, PeerConnection,
    PeerConnectionDependencies, PeerConnectionFactory, PeerConnectionObserver,
    PeerConnectionObserverBuilder, PeerConnectionRtcConfiguration, RtpCodecCapabilityVector,
    RtpTransceiverDirection, RtpTransceiverInit,
};
use tokio::io::AsyncWriteExt;

use crate::{Error, MediaSample, Message, ProcessorHandle, TrackId};

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
        let mut session = WhipSession::connect(
            &self.output_url,
            self.bearer_token.as_deref(),
            self.input_video_track_id.is_some(),
            &self.video_codec_preferences,
        )
        .await?;

        let mut video_rx = self
            .input_video_track_id
            .clone()
            .map(|track_id| handle.subscribe_track(track_id));
        let mut audio_rx = self
            .input_audio_track_id
            .clone()
            .map(|track_id| handle.subscribe_track(track_id));

        if video_rx.is_none() && audio_rx.is_none() {
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
        Message::Media(MediaSample::Video(frame)) => {
            session.push_video_frame(&frame)?;
            Ok(false)
        }
        Message::Media(MediaSample::Audio(_)) => Err(Error::new(format!(
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
        Message::Media(MediaSample::Audio(frame)) => {
            session.push_audio_frame(&frame)?;
            Ok(false)
        }
        Message::Media(MediaSample::Video(_)) => Err(Error::new(format!(
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
        let pc = PeerConnection::create(factory.as_ref(), &mut pc_config, &mut deps)
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

            let transceiver = pc
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
            let transceiver = pc
                .add_transceiver(MediaType::Video, &mut init)
                .map_err(|e| Error::new(format!("failed to add video transceiver: {e}")))?;
            let codecs = select_video_codecs(factory.as_ref(), video_codec_preferences)?;
            transceiver
                .set_codec_preferences(&codecs)
                .map_err(|e| Error::new(format!("failed to set video codec preferences: {e}")))?;
            (None, None)
        };

        let resource_url = exchange_offer_answer(&pc, output_url, bearer_token).await?;

        Ok(Self {
            _factory_bundle: factory_bundle,
            _observer: observer,
            pc: Some(pc),
            audio_sink,
            video_source,
            _video_track: video_track,
            resource_url,
            bearer_token: bearer_token.map(str::to_owned),
        })
    }

    fn push_video_frame(&mut self, frame: &crate::VideoFrame) -> crate::Result<()> {
        let source = self
            .video_source
            .as_mut()
            .ok_or_else(|| Error::new("video track is not configured"))?;
        crate::webrtc_video::push_i420_frame(source, frame)
    }

    fn push_audio_frame(&mut self, frame: &crate::AudioData) -> crate::Result<()> {
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
                Err(e) => tracing::warn!("failed to delete WHIP resource: {e}"),
            }
        }
    }
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
    let transceiver = pc
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
    pc: &PeerConnection,
    output_url: &str,
    bearer_token: Option<&str>,
) -> crate::Result<Option<String>> {
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
    crate::webrtc_sdp::set_remote_answer(pc, &answer_sdp)?;

    let resource_url = match location.as_deref() {
        Some(location) => match crate::webrtc_http::resolve_resource_url(output_url, location) {
            Ok(url) => Some(url),
            Err(e) => {
                tracing::warn!("failed to resolve WHIP resource URL from Location header: {e}");
                None
            }
        },
        None => {
            tracing::debug!("WHIP response does not contain Location header");
            None
        }
    };

    Ok(resource_url)
}

fn apply_ice_servers_from_link_header(
    pc: &PeerConnection,
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
        let result: orfail::Result<WhipPublisher> = crate::json::parse_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn whip_publisher_rejects_unknown_video_codec() {
        let json = r#"{
            "outputUrl":"https://example.com/whip/live",
            "inputVideoTrackId":"video-main",
            "videoCodecPreferences":["AV1","H266"]
        }"#;
        let result: orfail::Result<WhipPublisher> = crate::json::parse_str(json);
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
        let result: orfail::Result<WhipPublisher> = crate::json::parse_str(json);
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
}
