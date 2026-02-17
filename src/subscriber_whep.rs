use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use shiguredo_http11::{Request, Response, ResponseDecoder, uri::Uri};
use shiguredo_webrtc::{
    MediaType, PeerConnection, PeerConnectionDependencies, PeerConnectionObserver,
    PeerConnectionObserverBuilder, PeerConnectionRtcConfiguration, PeerConnectionState,
    RtpTransceiverDirection, RtpTransceiverInit, VideoSink, VideoSinkBuilder, VideoSinkWants,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use crate::{Error, MessageSender, ProcessorHandle, TrackId};

const DEFAULT_VIDEO_FRAME_DURATION: Duration = Duration::from_millis(33);

#[derive(Debug, Clone)]
pub struct WhepSubscriber {
    pub input_url: String,
    pub output_video_track_id: Option<TrackId>,
    pub output_audio_track_id: Option<TrackId>,
    pub bearer_token: Option<String>,
}

impl nojson::DisplayJson for WhepSubscriber {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("inputUrl", &self.input_url)?;
            if let Some(track_id) = &self.output_video_track_id {
                f.member("outputVideoTrackId", track_id)?;
            }
            if let Some(track_id) = &self.output_audio_track_id {
                f.member("outputAudioTrackId", track_id)?;
            }
            if let Some(token) = &self.bearer_token {
                f.member("bearerToken", token)?;
            }
            Ok(())
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for WhepSubscriber {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let input_url: String = value.to_member("inputUrl")?.required()?.try_into()?;
        if let Err(e) = validate_input_url(&input_url) {
            return Err(value.to_member("inputUrl")?.required()?.invalid(e));
        }

        let output_video_track_id: Option<TrackId> =
            value.to_member("outputVideoTrackId")?.try_into()?;
        if output_video_track_id.is_none() {
            return Err(value.invalid("outputVideoTrackId is required for now"));
        }

        let output_audio_track_id: Option<TrackId> =
            value.to_member("outputAudioTrackId")?.try_into()?;
        if output_audio_track_id.is_some() {
            return Err(value.invalid("outputAudioTrackId is not supported yet"));
        }

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

        Ok(Self {
            input_url,
            output_video_track_id,
            output_audio_track_id,
            bearer_token,
        })
    }
}

impl WhepSubscriber {
    pub async fn run(self, handle: ProcessorHandle) -> crate::Result<()> {
        let output_video_track_id = self
            .output_video_track_id
            .clone()
            .ok_or_else(|| Error::new("outputVideoTrackId is required for now"))?;
        let mut output_video_sender = handle
            .publish_track(output_video_track_id.clone())
            .await
            .map_err(|e| {
                Error::new(format!(
                    "failed to publish output video track {output_video_track_id}: {e}"
                ))
            })?;

        let mut session =
            WhepSession::connect(&self.input_url, self.bearer_token.as_deref()).await?;
        let run_result = session.forward_video(&mut output_video_sender).await;
        output_video_sender.send_eos();
        session.disconnect().await;
        run_result
    }
}

#[derive(Debug)]
enum WhepEvent {
    ConnectionChange(PeerConnectionState),
    VideoTrackRemoved,
}

struct AttachedVideoTrackState {
    sink: VideoSink,
    current_track: Option<shiguredo_webrtc::VideoTrack>,
}

struct WhepSession {
    /// `PeerConnectionFactory` のスコープを保持するために参照を持つ
    _factory_bundle: Rc<crate::webrtc_factory::WebRtcFactoryBundle>,
    /// `PeerConnectionObserver` のコールバック登録を維持するために参照を持つ
    _observer: PeerConnectionObserver,
    pc: Option<PeerConnection>,
    /// 受信映像トラックの sink と参照を維持するために保持する
    _video_track_state: Arc<Mutex<AttachedVideoTrackState>>,
    frame_rx: tokio::sync::mpsc::UnboundedReceiver<crate::VideoFrame>,
    event_rx: tokio::sync::mpsc::UnboundedReceiver<WhepEvent>,
    resource_url: Option<String>,
    bearer_token: Option<String>,
}

impl WhepSession {
    async fn connect(input_url: &str, bearer_token: Option<&str>) -> crate::Result<Self> {
        let factory_bundle = Rc::new(crate::webrtc_factory::WebRtcFactoryBundle::new()?);
        let factory = factory_bundle.factory();

        let (frame_tx, frame_rx) = tokio::sync::mpsc::unbounded_channel::<crate::VideoFrame>();
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<WhepEvent>();
        let last_video_timestamp = Arc::new(Mutex::new(None::<Duration>));
        let last_video_timestamp_for_sink = last_video_timestamp.clone();
        let sink = VideoSinkBuilder::new(move |frame| {
            match convert_webrtc_video_frame_to_i420(
                frame,
                &last_video_timestamp_for_sink,
                DEFAULT_VIDEO_FRAME_DURATION,
            ) {
                Ok(video_frame) => {
                    let _ = frame_tx.send(video_frame);
                }
                Err(e) => {
                    tracing::warn!("failed to convert incoming WHEP video frame: {e}");
                }
            }
        })
        .build();
        let video_track_state = Arc::new(Mutex::new(AttachedVideoTrackState {
            sink,
            current_track: None,
        }));
        let video_track_state_for_track = video_track_state.clone();
        let video_track_state_for_remove = video_track_state.clone();
        let last_video_timestamp_for_track = last_video_timestamp.clone();
        let last_video_timestamp_for_remove = last_video_timestamp.clone();
        let event_tx_for_conn = event_tx.clone();
        let event_tx_for_remove = event_tx.clone();
        let observer = PeerConnectionObserverBuilder::new()
            .on_connection_change(move |state| {
                let _ = event_tx_for_conn.send(WhepEvent::ConnectionChange(state));
            })
            .on_track(move |transceiver| {
                let receiver = transceiver.receiver();
                let track = receiver.track();
                let kind = match track.kind() {
                    Ok(kind) => kind,
                    Err(e) => {
                        tracing::warn!("failed to get incoming track kind: {e}");
                        return;
                    }
                };
                if kind == "audio" {
                    // 現状の webrtc-rs API には受信 AudioTrack から PCM を取り出す sink API がないため、
                    // WHEP subscriber の音声受信は未対応とする。
                    tracing::info!("WHEP incoming audio track is not supported yet");
                    return;
                }
                if kind != "video" {
                    tracing::debug!("ignoring unsupported incoming track kind: {kind}");
                    return;
                }

                let video_track = track.cast_to_video_track();
                if let Ok(mut state) = video_track_state_for_track.lock() {
                    if let Some(current) = state.current_track.as_ref()
                        && current.as_ptr() == video_track.as_ptr()
                    {
                        return;
                    }
                    if let Some(track) = state.current_track.take() {
                        track.remove_sink(&state.sink);
                    }
                    let wants = VideoSinkWants::new();
                    video_track.add_or_update_sink(&state.sink, &wants);
                    state.current_track = Some(video_track);
                    if let Ok(mut ts) = last_video_timestamp_for_track.lock() {
                        *ts = None;
                    }
                }
            })
            .on_remove_track(move |receiver| {
                let track = receiver.track();
                let kind = match track.kind() {
                    Ok(kind) => kind,
                    Err(_) => return,
                };
                if kind != "video" {
                    return;
                }
                let removed_track = track.cast_to_video_track();
                if let Ok(mut state) = video_track_state_for_remove.lock()
                    && let Some(current) = state.current_track.as_ref()
                    && current.as_ptr() == removed_track.as_ptr()
                {
                    if let Some(track) = state.current_track.take() {
                        track.remove_sink(&state.sink);
                    }
                    if let Ok(mut ts) = last_video_timestamp_for_remove.lock() {
                        *ts = None;
                    }
                    let _ = event_tx_for_remove.send(WhepEvent::VideoTrackRemoved);
                }
            })
            .build();

        let mut deps = PeerConnectionDependencies::new(&observer);
        let mut pc_config = PeerConnectionRtcConfiguration::new();
        let pc = PeerConnection::create(factory.as_ref(), &mut pc_config, &mut deps)
            .map_err(|e| Error::new(format!("failed to create PeerConnection: {e}")))?;

        add_recv_transceiver(&pc, MediaType::Audio)?;
        add_recv_transceiver(&pc, MediaType::Video)?;

        let resource_url = exchange_offer_answer(&pc, input_url, bearer_token).await?;

        Ok(Self {
            _factory_bundle: factory_bundle,
            _observer: observer,
            pc: Some(pc),
            _video_track_state: video_track_state,
            frame_rx,
            event_rx,
            resource_url,
            bearer_token: bearer_token.map(str::to_owned),
        })
    }

    async fn forward_video(
        &mut self,
        output_video_sender: &mut MessageSender,
    ) -> crate::Result<()> {
        loop {
            tokio::select! {
                maybe_frame = self.frame_rx.recv() => {
                    let Some(frame) = maybe_frame else {
                        break;
                    };
                    if !output_video_sender.send_video(frame) {
                        break;
                    }
                }
                maybe_event = self.event_rx.recv() => {
                    let Some(event) = maybe_event else {
                        break;
                    };
                    match event {
                        WhepEvent::ConnectionChange(state) => {
                            tracing::info!("WHEP PeerConnection state changed: {state:?}");
                            if matches!(state, PeerConnectionState::Failed | PeerConnectionState::Closed) {
                                break;
                            }
                        }
                        WhepEvent::VideoTrackRemoved => {
                            tracing::info!("WHEP video track removed");
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn disconnect(&mut self) {
        self.pc = None;
        if let Some(resource_url) = self.resource_url.take() {
            match delete_resource(
                &resource_url,
                self.bearer_token.as_deref(),
                "Hisui-WhepSubscriber",
            )
            .await
            {
                Ok(()) => tracing::info!("WHEP resource deleted: {resource_url}"),
                Err(e) => tracing::warn!("failed to delete WHEP resource: {e}"),
            }
        }
    }
}

fn add_recv_transceiver(pc: &PeerConnection, media_type: MediaType) -> crate::Result<()> {
    let mut init = RtpTransceiverInit::new();
    init.set_direction(RtpTransceiverDirection::RecvOnly);
    pc.add_transceiver(media_type, &mut init)
        .map_err(|e| Error::new(format!("failed to add recv transceiver: {e}")))?;
    Ok(())
}

async fn exchange_offer_answer(
    pc: &PeerConnection,
    input_url: &str,
    bearer_token: Option<&str>,
) -> crate::Result<Option<String>> {
    let offer_sdp = crate::webrtc_sdp::create_offer_sdp_recvonly(pc)?;
    log_sdp_candidates("WHEP offer", &offer_sdp);

    let response = send_offer(input_url, bearer_token, &offer_sdp).await?;
    if response.status_code != 201 {
        return Err(Error::new(format!(
            "WHEP endpoint returned unexpected status code: {}",
            response.status_code
        )));
    }

    apply_ice_servers_from_link_header(pc, &response)?;
    crate::webrtc_sdp::set_local_offer(pc, &offer_sdp)?;

    let location = response.get_header("Location").map(str::to_owned);
    let answer_sdp = String::from_utf8(response.body)
        .map_err(|e| Error::new(format!("failed to decode answer SDP as UTF-8: {e}")))?;
    if answer_sdp.trim().is_empty() {
        return Err(Error::new("WHEP endpoint returned empty answer SDP"));
    }
    log_sdp_candidates("WHEP answer", &answer_sdp);
    crate::webrtc_sdp::set_remote_answer(pc, &answer_sdp)?;

    let resource_url = match location.as_deref() {
        Some(location) => match resolve_resource_url(input_url, location) {
            Ok(url) => Some(url),
            Err(e) => {
                tracing::warn!("failed to resolve WHEP resource URL from Location header: {e}");
                None
            }
        },
        None => {
            tracing::debug!("WHEP response does not contain Location header");
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
        tracing::debug!("WHEP response does not contain Link header for ICE servers");
        return Ok(());
    };
    let parsed = parse_link_header(link_header);
    if parsed.urls.is_empty() {
        tracing::debug!("WHEP Link header does not include ICE server URLs");
        return Ok(());
    }
    tracing::debug!(
        "WHEP Link header parsed: urls={:?}, username_present={}, credential_present={}",
        parsed.urls,
        parsed.username.is_some(),
        parsed.credential.is_some()
    );

    let mut config = PeerConnectionRtcConfiguration::new();
    let mut server = shiguredo_webrtc::IceServer::new();
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
    let mut media_direction_lines = Vec::new();
    let mut current_media = None::<String>;

    for line in sdp.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("m=") {
            let media = rest.split_whitespace().next().unwrap_or("unknown");
            current_media = Some(media.to_owned());
        }
        if let Some(direction) = line.strip_prefix("a=")
            && matches!(direction, "sendrecv" | "sendonly" | "recvonly" | "inactive")
        {
            let media = current_media.as_deref().unwrap_or("unknown");
            media_direction_lines.push(format!("{media}:{direction}"));
        }
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
    if !media_direction_lines.is_empty() {
        tracing::debug!(
            "{label} SDP media directions: {}",
            media_direction_lines.join(", ")
        );
    }

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

struct RequestTarget {
    host: String,
    port: u16,
    path_and_query: String,
    host_header: String,
    tls: bool,
}

fn build_request_target(url: &str) -> crate::Result<RequestTarget> {
    let uri = Uri::parse(url).map_err(|e| Error::new(format!("invalid URL: {e}")))?;

    let scheme = uri
        .scheme()
        .ok_or_else(|| Error::new("URL must contain URL scheme"))?;
    let tls = match scheme {
        "http" => false,
        "https" => true,
        _ => return Err(Error::new("URL scheme must be http or https")),
    };

    let host = uri
        .host()
        .ok_or_else(|| Error::new("URL must contain host"))?
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

async fn send_offer(
    input_url: &str,
    bearer_token: Option<&str>,
    offer_sdp: &str,
) -> crate::Result<Response> {
    let target = build_request_target(input_url)?;
    let mut request = Request::new("POST", &target.path_and_query)
        .header("Host", &target.host_header)
        .header("Content-Type", "application/sdp")
        .header("Connection", "close")
        .header("User-Agent", "Hisui-WhepSubscriber");
    let authorization = bearer_token.map(|token| format!("Bearer {token}"));
    if let Some(value) = authorization.as_deref() {
        request = request.header("Authorization", value);
    }
    let request = request.body(offer_sdp.as_bytes().to_vec());

    let mut stream = crate::tcp::TcpOrTlsStream::connect(&target.host, target.port, target.tls)
        .await
        .map_err(|e| Error::new(format!("failed to connect WHEP endpoint: {e}")))?;
    stream
        .write_all(&request.encode())
        .await
        .map_err(|e| Error::new(format!("failed to send WHEP request: {e}")))?;
    stream
        .flush()
        .await
        .map_err(|e| Error::new(format!("failed to flush WHEP request: {e}")))?;

    read_http_response(&mut stream).await
}

async fn delete_resource(
    resource_url: &str,
    bearer_token: Option<&str>,
    user_agent: &str,
) -> crate::Result<()> {
    let target = build_request_target(resource_url)?;
    let mut request = Request::new("DELETE", &target.path_and_query)
        .header("Host", &target.host_header)
        .header("Connection", "close")
        .header("User-Agent", user_agent);
    let authorization = bearer_token.map(|token| format!("Bearer {token}"));
    if let Some(value) = authorization.as_deref() {
        request = request.header("Authorization", value);
    }
    let request = request.body(Vec::new());

    let mut stream = crate::tcp::TcpOrTlsStream::connect(&target.host, target.port, target.tls)
        .await
        .map_err(|e| Error::new(format!("failed to connect resource endpoint: {e}")))?;
    stream
        .write_all(&request.encode())
        .await
        .map_err(|e| Error::new(format!("failed to send resource DELETE request: {e}")))?;
    stream
        .flush()
        .await
        .map_err(|e| Error::new(format!("failed to flush resource DELETE request: {e}")))?;

    let response = read_http_response(&mut stream).await?;
    if !(200..300).contains(&response.status_code) {
        return Err(Error::new(format!(
            "resource endpoint returned unexpected status code for DELETE: {}",
            response.status_code
        )));
    }
    Ok(())
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
            .map_err(|e| Error::new(format!("failed to read WHEP response: {e}")))?;
        if n == 0 {
            return Err(Error::new(
                "connection closed before a complete WHEP response was received",
            ));
        }

        decoder
            .feed(&buf[..n])
            .map_err(|e| Error::new(format!("failed to decode WHEP response: {e}")))?;
        if let Some(response) = decoder
            .decode()
            .map_err(|e| Error::new(format!("failed to decode WHEP response: {e}")))?
        {
            return Ok(response);
        }
    }
}

fn validate_input_url(input_url: &str) -> Result<(), String> {
    let uri = Uri::parse(input_url).map_err(|e| e.to_string())?;
    let scheme = uri
        .scheme()
        .ok_or_else(|| "inputUrl must contain URL scheme".to_owned())?;
    if scheme != "http" && scheme != "https" {
        return Err("inputUrl scheme must be http or https".to_owned());
    }
    uri.host()
        .ok_or_else(|| "inputUrl must contain host".to_owned())?;
    Ok(())
}

fn convert_webrtc_video_frame_to_i420(
    frame: shiguredo_webrtc::VideoFrameRef<'_>,
    last_video_timestamp: &Arc<Mutex<Option<Duration>>>,
    default_duration: Duration,
) -> crate::Result<crate::VideoFrame> {
    let buffer = frame.buffer();
    let width = usize::try_from(buffer.width())
        .map_err(|_| Error::new("incoming video frame width is negative"))?;
    let height = usize::try_from(buffer.height())
        .map_err(|_| Error::new("incoming video frame height is negative"))?;
    if width == 0 || height == 0 {
        return Err(Error::new("incoming video frame size is invalid"));
    }

    let timestamp = if frame.timestamp_us() <= 0 {
        Duration::ZERO
    } else {
        Duration::from_micros(frame.timestamp_us() as u64)
    };
    let duration = if let Ok(mut last) = last_video_timestamp.lock() {
        let duration = match *last {
            Some(prev) if timestamp > prev => timestamp.saturating_sub(prev),
            _ => default_duration,
        };
        *last = Some(timestamp);
        duration
    } else {
        default_duration
    };

    let y_stride = usize::try_from(buffer.stride_y())
        .map_err(|_| Error::new("incoming video frame Y stride is negative"))?;
    let u_stride = usize::try_from(buffer.stride_u())
        .map_err(|_| Error::new("incoming video frame U stride is negative"))?;
    let v_stride = usize::try_from(buffer.stride_v())
        .map_err(|_| Error::new("incoming video frame V stride is negative"))?;
    let input_frame = crate::VideoFrame {
        source_id: None,
        data: Vec::new(),
        format: crate::video::VideoFormat::I420,
        keyframe: true,
        width,
        height,
        timestamp,
        duration,
        sample_entry: None,
    };
    Ok(crate::VideoFrame::new_i420(
        input_frame,
        width,
        height,
        buffer.y_data(),
        buffer.u_data(),
        buffer.v_data(),
        y_stride,
        u_stride,
        v_stride,
    ))
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

fn resolve_resource_url(base_url: &str, location: &str) -> crate::Result<String> {
    if location.starts_with("http://") || location.starts_with("https://") {
        Uri::parse(location).map_err(|e| Error::new(format!("invalid resource URL: {e}")))?;
        return Ok(location.to_owned());
    }

    let base = Uri::parse(base_url).map_err(|e| Error::new(format!("invalid base URL: {e}")))?;
    let scheme = base
        .scheme()
        .ok_or_else(|| Error::new("base URL must contain URL scheme"))?;
    let host = base
        .host()
        .ok_or_else(|| Error::new("base URL must contain host"))?;
    let default_port = if scheme == "https" { 443 } else { 80 };
    let port = base.port().unwrap_or(default_port);
    let authority = if (scheme == "http" && port != 80) || (scheme == "https" && port != 443) {
        format!("{host}:{port}")
    } else {
        host.to_owned()
    };

    let path_and_query = if location.starts_with('/') {
        location.to_owned()
    } else {
        let mut base_path = base.path().to_owned();
        if base_path.is_empty() {
            base_path = "/".to_owned();
        }
        let parent_end = base_path.rfind('/').unwrap_or(0);
        let parent = &base_path[..=parent_end];
        format!("{parent}{location}")
    };
    Uri::parse(&format!("{scheme}://{authority}{path_and_query}"))
        .map_err(|e| Error::new(format!("invalid resolved resource URL: {e}")))?;
    Ok(format!("{scheme}://{authority}{path_and_query}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whep_subscriber_requires_input_url_and_video_track_id() {
        let json = r#"{
            "inputUrl":"https://example.com/whep/live"
        }"#;
        let result: orfail::Result<WhepSubscriber> = crate::json::parse_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn whep_subscriber_rejects_invalid_url_scheme() {
        let json = r#"{
            "inputUrl":"ws://example.com/whep/live",
            "outputVideoTrackId":"video-main"
        }"#;
        let result: orfail::Result<WhepSubscriber> = crate::json::parse_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn whep_subscriber_rejects_empty_bearer_token() {
        let json = r#"{
            "inputUrl":"https://example.com/whep/live",
            "outputVideoTrackId":"video-main",
            "bearerToken":"   "
        }"#;
        let result: orfail::Result<WhepSubscriber> = crate::json::parse_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn whep_subscriber_rejects_output_audio_track_id_for_now() {
        let json = r#"{
            "inputUrl":"https://example.com/whep/live",
            "outputVideoTrackId":"video-main",
            "outputAudioTrackId":"audio-main"
        }"#;
        let result: orfail::Result<WhepSubscriber> = crate::json::parse_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn whep_subscriber_accepts_params() {
        let json = r#"{
            "inputUrl":"https://example.com/whep/live",
            "outputVideoTrackId":"video-main",
            "bearerToken":"  test-token  "
        }"#;
        let subscriber: WhepSubscriber = crate::json::parse_str(json).expect("parse");
        assert_eq!(subscriber.input_url, "https://example.com/whep/live");
        assert_eq!(
            subscriber.output_video_track_id.as_ref().map(|id| id.get()),
            Some("video-main")
        );
        assert!(subscriber.output_audio_track_id.is_none());
        assert_eq!(subscriber.bearer_token.as_deref(), Some("test-token"));
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
    fn resolve_resource_url_supports_relative_location() {
        let resolved = resolve_resource_url(
            "https://example.com/whep/live/channel",
            "/resource/abc?token=xyz",
        )
        .expect("resolve");
        assert_eq!(resolved, "https://example.com/resource/abc?token=xyz");
    }

    #[test]
    fn resolve_resource_url_supports_absolute_location() {
        let resolved = resolve_resource_url(
            "https://example.com/whep/live/channel",
            "https://resource.example.com/session/123",
        )
        .expect("resolve");
        assert_eq!(resolved, "https://resource.example.com/session/123");
    }
}
