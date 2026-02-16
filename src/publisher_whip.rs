use std::sync::Arc;
use std::time::Duration;

use shiguredo_http11::{Request, Response, ResponseDecoder, uri::Uri};
use shiguredo_webrtc::{
    AdaptedVideoTrackSource, AudioDecoderFactory, AudioDeviceModule, AudioDeviceModuleAudioLayer,
    AudioEncoderFactory, AudioProcessingBuilder, CreateSessionDescriptionObserver, CxxString,
    Environment, IceServer, MediaType, PeerConnection, PeerConnectionDependencies,
    PeerConnectionFactory, PeerConnectionFactoryDependencies, PeerConnectionObserver,
    PeerConnectionObserverBuilder, PeerConnectionOfferAnswerOptions,
    PeerConnectionRtcConfiguration, RtcEventLogFactory, RtpCodecCapabilityVector,
    RtpTransceiverDirection, RtpTransceiverInit, SdpType, SessionDescription,
    SetLocalDescriptionObserver, SetRemoteDescriptionObserver, Thread, VideoDecoderFactory,
    VideoEncoderFactory,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use crate::{Error, MediaSample, Message, ProcessorHandle, TrackId};

#[derive(Debug, Clone)]
pub struct WhipPublisher {
    pub whip_url: String,
    pub video_track_id: TrackId,
    pub audio_mline: bool,
    pub video_codec_preferences: Vec<VideoCodecPreference>,
}

impl nojson::DisplayJson for WhipPublisher {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("whipUrl", &self.whip_url)?;
            f.member("videoTrackId", &self.video_track_id)?;
            f.member("audioMline", self.audio_mline)?;
            f.member("videoCodecPreferences", &self.video_codec_preferences)
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for WhipPublisher {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let whip_url: String = value.to_member("whipUrl")?.required()?.try_into()?;
        validate_whip_url(&whip_url)
            .map_err(|e| value.to_member("whipUrl")?.required()?.invalid(e))?;

        let video_track_id: TrackId = value.to_member("videoTrackId")?.required()?.try_into()?;
        let audio_mline = value.to_member("audioMline")?.try_into()?.unwrap_or(true);
        let codecs_raw: Option<Vec<String>> =
            value.to_member("videoCodecPreferences")?.try_into()?;
        let video_codec_preferences =
            parse_video_codec_preferences(codecs_raw).map_err(|e| value.invalid(e))?;

        Ok(Self {
            whip_url,
            video_track_id,
            audio_mline,
            video_codec_preferences,
        })
    }
}

impl WhipPublisher {
    pub async fn run(self, handle: ProcessorHandle) -> crate::Result<()> {
        let mut session = WhipSession::connect(
            &self.whip_url,
            self.audio_mline,
            &self.video_codec_preferences,
        )
        .await?;

        let mut input_rx = handle.subscribe_track(self.video_track_id.clone());

        loop {
            match input_rx.recv().await {
                Message::Media(MediaSample::Video(frame)) => {
                    session.push_video_frame(&frame)?;
                }
                Message::Media(MediaSample::Audio(_)) => {
                    return Err(Error::new(format!(
                        "expected a video sample on track {}, but got an audio sample",
                        self.video_track_id
                    )));
                }
                Message::Eos => {
                    session.disconnect();
                    break;
                }
                Message::Syn(_) => {}
            }
        }

        Ok(())
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
        deps.set_event_log_factory(RtcEventLogFactory::new());
        let adm = AudioDeviceModule::new(&env, AudioDeviceModuleAudioLayer::Dummy).ok()?;
        deps.set_audio_device_module(&adm);
        deps.set_audio_encoder_factory(&AudioEncoderFactory::builtin());
        deps.set_audio_decoder_factory(&AudioDecoderFactory::builtin());
        deps.set_video_encoder_factory(VideoEncoderFactory::builtin());
        deps.set_video_decoder_factory(VideoDecoderFactory::builtin());
        deps.set_audio_processing_builder(AudioProcessingBuilder::new_builtin());
        deps.enable_media();

        let factory = PeerConnectionFactory::create_modular(&mut deps).ok()?;
        Some(Self {
            factory: Arc::new(factory),
            _network: network,
            _worker: worker,
            _signaling: signaling,
        })
    }

    fn factory(&self) -> &PeerConnectionFactory {
        self.factory.as_ref()
    }
}

struct WhipSession {
    #[allow(dead_code)]
    factory_holder: Arc<FactoryHolder>,
    #[allow(dead_code)]
    observer: PeerConnectionObserver,
    pc: Option<PeerConnection>,
    video_source: AdaptedVideoTrackSource,
    #[allow(dead_code)]
    video_track: shiguredo_webrtc::VideoTrack,
}

impl WhipSession {
    async fn connect(
        whip_url: &str,
        audio_mline: bool,
        video_codec_preferences: &[VideoCodecPreference],
    ) -> crate::Result<Self> {
        #[allow(clippy::arc_with_non_send_sync)]
        let factory_holder = Arc::new(
            FactoryHolder::new()
                .ok_or_else(|| Error::new("failed to create WebRTC PeerConnectionFactory"))?,
        );

        let observer = PeerConnectionObserverBuilder::new()
            .on_connection_change(|state| {
                tracing::info!("WHIP PeerConnection state changed: {state:?}");
            })
            .build();
        let mut deps = PeerConnectionDependencies::new(&observer);
        let mut pc_config = PeerConnectionRtcConfiguration::new();
        let pc = PeerConnection::create(factory_holder.factory(), &mut pc_config, &mut deps)
            .map_err(|e| Error::new(format!("failed to create PeerConnection: {e}")))?;

        if audio_mline {
            add_audio_transceiver(&pc, factory_holder.factory())?;
        }

        let video_source = AdaptedVideoTrackSource::new();
        let video_track_source = video_source.cast_to_video_track_source();
        let track_id = shiguredo_webrtc::random_string(16);
        let video_track = factory_holder
            .factory()
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
        let codecs = select_video_codecs(factory_holder.factory(), video_codec_preferences)?;
        transceiver
            .set_codec_preferences(&codecs)
            .map_err(|e| Error::new(format!("failed to set video codec preferences: {e}")))?;

        exchange_offer_answer(&pc, whip_url).await?;

        Ok(Self {
            factory_holder,
            observer,
            pc: Some(pc),
            video_source,
            video_track,
        })
    }

    fn push_video_frame(&mut self, frame: &crate::VideoFrame) -> crate::Result<()> {
        push_i420_frame(&mut self.video_source, frame)
    }

    fn disconnect(&mut self) {
        self.pc = None;
    }
}

fn validate_whip_url(whip_url: &str) -> Result<(), String> {
    let uri = Uri::parse(whip_url).map_err(|e| e.to_string())?;
    let scheme = uri
        .scheme()
        .ok_or_else(|| "whipUrl must contain URL scheme".to_owned())?;
    if scheme != "http" && scheme != "https" {
        return Err("whipUrl scheme must be http or https".to_owned());
    }
    uri.host()
        .ok_or_else(|| "whipUrl must contain host".to_owned())?;
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

async fn exchange_offer_answer(pc: &PeerConnection, whip_url: &str) -> crate::Result<()> {
    let offer_sdp = create_offer_sdp(pc)?;
    set_local_offer(pc, &offer_sdp)?;

    let response = send_offer(whip_url, &offer_sdp).await?;
    if response.status_code != 201 {
        return Err(Error::new(format!(
            "WHIP endpoint returned unexpected status code: {}",
            response.status_code
        )));
    }
    apply_ice_servers_from_link_header(pc, &response)?;

    let answer_sdp = String::from_utf8(response.body)
        .map_err(|e| Error::new(format!("failed to decode answer SDP as UTF-8: {e}")))?;
    if answer_sdp.trim().is_empty() {
        return Err(Error::new("WHIP endpoint returned empty answer SDP"));
    }
    set_remote_answer(pc, &answer_sdp)?;

    Ok(())
}

fn create_offer_sdp(pc: &PeerConnection) -> crate::Result<String> {
    let mut options = PeerConnectionOfferAnswerOptions::new();
    options.set_offer_to_receive_audio(0);
    options.set_offer_to_receive_video(0);

    let (offer_tx, offer_rx) = std::sync::mpsc::channel::<crate::Result<String>>();
    let offer_tx_err = offer_tx.clone();
    let mut offer_observer = CreateSessionDescriptionObserver::new(
        move |description| {
            let sdp = description
                .to_string()
                .map_err(|e| Error::new(format!("failed to convert offer SDP to string: {e}")));
            let _ = offer_tx.send(sdp);
        },
        move |error| {
            let message = error.message().unwrap_or_else(|_| "unknown".to_owned());
            let _ = offer_tx_err.send(Err(Error::new(format!("create_offer failed: {message}"))));
        },
    );
    pc.create_offer(&mut offer_observer, &mut options);

    offer_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| Error::new("create_offer timed out"))?
}

fn set_local_offer(pc: &PeerConnection, offer_sdp: &str) -> crate::Result<()> {
    let offer = SessionDescription::new(SdpType::Offer, offer_sdp)
        .map_err(|e| Error::new(format!("failed to parse local offer SDP: {e}")))?;
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    let observer = SetLocalDescriptionObserver::new(move |error| {
        let message = if error.ok() {
            None
        } else {
            Some(error.message().unwrap_or_else(|_| "unknown".to_owned()))
        };
        let _ = tx.send(message);
    });
    pc.set_local_description(offer, &observer);

    let result = rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| Error::new("set_local_description timed out"))?;
    if let Some(message) = result {
        return Err(Error::new(format!(
            "set_local_description failed: {message}"
        )));
    }

    Ok(())
}

fn set_remote_answer(pc: &PeerConnection, answer_sdp: &str) -> crate::Result<()> {
    let answer = SessionDescription::new(SdpType::Answer, answer_sdp)
        .map_err(|e| Error::new(format!("failed to parse remote answer SDP: {e}")))?;
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    let observer = SetRemoteDescriptionObserver::new(move |error| {
        let message = if error.ok() {
            None
        } else {
            Some(error.message().unwrap_or_else(|_| "unknown".to_owned()))
        };
        let _ = tx.send(message);
    });
    pc.set_remote_description(answer, &observer);

    let result = rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| Error::new("set_remote_description timed out"))?;
    if let Some(message) = result {
        return Err(Error::new(format!(
            "set_remote_description failed: {message}"
        )));
    }

    Ok(())
}

fn apply_ice_servers_from_link_header(
    pc: &PeerConnection,
    response: &Response,
) -> crate::Result<()> {
    let Some(link_header) = response.get_header("Link") else {
        return Ok(());
    };
    let parsed = parse_link_header(link_header);
    if parsed.urls.is_empty() {
        return Ok(());
    }

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

struct RequestTarget {
    host: String,
    port: u16,
    path_and_query: String,
    host_header: String,
    tls: bool,
}

fn build_request_target(url: &str) -> crate::Result<RequestTarget> {
    let uri = Uri::parse(url).map_err(|e| Error::new(format!("invalid WHIP URL: {e}")))?;

    let scheme = uri
        .scheme()
        .ok_or_else(|| Error::new("whipUrl must contain URL scheme"))?;
    let tls = match scheme {
        "http" => false,
        "https" => true,
        _ => return Err(Error::new("whipUrl scheme must be http or https")),
    };

    let host = uri
        .host()
        .ok_or_else(|| Error::new("whipUrl must contain host"))?
        .to_owned();
    let default_port = if tls { 443 } else { 80 };
    let port = uri.port().unwrap_or(default_port);

    let mut path_and_query = uri.path().to_owned();
    if path_and_query.is_empty() {
        path_and_query = "/".to_owned();
    }
    if let Some(query) = uri.query() {
        path_and_query.push('?');
        path_and_query.push_str(query);
    }

    let host_header = if (!tls && port != 80) || (tls && port != 443) {
        format!("{host}:{port}")
    } else {
        host.clone()
    };

    Ok(RequestTarget {
        host,
        port,
        path_and_query,
        host_header,
        tls,
    })
}

async fn send_offer(whip_url: &str, offer_sdp: &str) -> crate::Result<Response> {
    let target = build_request_target(whip_url)?;
    let request = Request::new("POST", &target.path_and_query)
        .header("Host", &target.host_header)
        .header("Content-Type", "application/sdp")
        .header("Connection", "close")
        .header("User-Agent", "Hisui-WhipPublisher")
        .body(offer_sdp.as_bytes().to_vec());

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

    read_http_response(&mut stream).await
}

async fn read_http_response<T>(stream: &mut T) -> crate::Result<Response>
where
    T: AsyncRead + Unpin,
{
    let mut decoder = ResponseDecoder::new();
    let mut buf = [0u8; 4096];

    loop {
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| Error::new(format!("failed to read WHIP response: {e}")))?;
        if n == 0 {
            return Err(Error::new(
                "connection closed before a complete WHIP response was received",
            ));
        }

        decoder
            .feed(&buf[..n])
            .map_err(|e| Error::new(format!("failed to decode WHIP response: {e}")))?;
        if let Some(response) = decoder
            .decode()
            .map_err(|e| Error::new(format!("failed to decode WHIP response: {e}")))?
        {
            return Ok(response);
        }
    }
}

struct ParsedLinkHeader {
    urls: Vec<String>,
    username: Option<String>,
    credential: Option<String>,
}

fn parse_link_header(header: &str) -> ParsedLinkHeader {
    let mut urls = Vec::new();
    let mut username = None;
    let mut credential = None;

    for part in header.split(',') {
        let part = part.trim();
        if let Some(start) = part.find('<')
            && let Some(end) = part[start + 1..].find('>')
        {
            urls.push(part[start + 1..start + 1 + end].to_owned());
        }

        if username.is_none() {
            username = extract_quoted_param(part, "username");
        }
        if credential.is_none() {
            credential = extract_quoted_param(part, "credential");
        }
    }

    ParsedLinkHeader {
        urls,
        username,
        credential,
    }
}

fn extract_quoted_param(text: &str, key: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let pattern = format!("{key}=\"");
    let pos = lower.find(&pattern)?;
    let start = pos + pattern.len();
    let rest = &text[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

fn push_i420_frame(
    source: &mut AdaptedVideoTrackSource,
    frame: &crate::VideoFrame,
) -> crate::Result<()> {
    if frame.format != crate::video::VideoFormat::I420 {
        return Err(Error::new(format!(
            "unsupported video format for WHIP: {} (expected I420)",
            frame.format
        )));
    }

    let width = frame.width;
    let height = frame.height;
    if width == 0 || height == 0 {
        return Err(Error::new("invalid frame size: width or height is zero"));
    }

    let uv_width = width.div_ceil(2);
    let uv_height = height.div_ceil(2);
    let y_size = width * height;
    let uv_size = uv_width * uv_height;
    if frame.data.len() < y_size + uv_size * 2 {
        return Err(Error::new(format!(
            "insufficient I420 frame data: got {} bytes",
            frame.data.len()
        )));
    }

    let (y_plane, rest) = frame.data.split_at(y_size);
    let (u_plane, v_plane) = rest.split_at(uv_size);
    let buffer = shiguredo_webrtc::I420Buffer::new(width as i32, height as i32);

    // I420Buffer の各 plane は API から書き込み可能領域として扱う。
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

    let timestamp_us = i64::try_from(frame.timestamp.as_micros()).unwrap_or(i64::MAX);
    let video_frame = shiguredo_webrtc::VideoFrame::from_i420(&buffer, timestamp_us);
    source.on_frame(&video_frame);
    Ok(())
}

unsafe fn copy_plane(dst: *mut u8, dst_stride: usize, src: &[u8], width: usize, height: usize) {
    for row in 0..height {
        let src_offset = row * width;
        let dst_offset = row * dst_stride;
        unsafe {
            std::ptr::copy_nonoverlapping(src.as_ptr().add(src_offset), dst.add(dst_offset), width);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whip_publisher_params_defaults_are_applied() {
        let json = r#"{
            "whipUrl":"https://example.com/whip/live",
            "videoTrackId":"video-main"
        }"#;
        let publisher: WhipPublisher = crate::json::parse_str(json).expect("parse");

        assert_eq!(publisher.whip_url, "https://example.com/whip/live");
        assert_eq!(publisher.video_track_id.get(), "video-main");
        assert!(publisher.audio_mline);
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
            "whipUrl":"ws://example.com/whip/live",
            "videoTrackId":"video-main"
        }"#;
        let result: orfail::Result<WhipPublisher> = crate::json::parse_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn whip_publisher_rejects_unknown_video_codec() {
        let json = r#"{
            "whipUrl":"https://example.com/whip/live",
            "videoTrackId":"video-main",
            "videoCodecPreferences":["AV1","H266"]
        }"#;
        let result: orfail::Result<WhipPublisher> = crate::json::parse_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn parse_link_header_extracts_urls_and_credentials() {
        let parsed = parse_link_header(
            r#"<turn:turn.example.com?transport=udp>; rel="ice-server"; username="user"; credential="pass""#,
        );
        assert_eq!(parsed.urls.len(), 1);
        assert_eq!(parsed.urls[0], "turn:turn.example.com?transport=udp");
        assert_eq!(parsed.username.as_deref(), Some("user"));
        assert_eq!(parsed.credential.as_deref(), Some("pass"));
    }

    #[test]
    fn build_request_target_preserves_query() {
        let target = build_request_target("https://example.com:8443/whip/live?foo=bar")
            .expect("build request target");
        assert_eq!(target.host, "example.com");
        assert_eq!(target.port, 8443);
        assert_eq!(target.path_and_query, "/whip/live?foo=bar");
        assert_eq!(target.host_header, "example.com:8443");
        assert!(target.tls);
    }
}
