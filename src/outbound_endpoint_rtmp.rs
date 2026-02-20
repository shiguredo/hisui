use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::tcp::{ServerTcpOrTlsStream, create_server_tls_acceptor};
use crate::{
    Error, MediaSample, Message, ProcessorHandle, TrackId,
    audio::{AudioData, AudioFormat},
    video::{VideoFormat, VideoFrame},
};

/// メディアフレーム用チャネルサイズ
///
/// こっちは基本的に詰まらないので比較的小さくていい
const FRAME_CHANNEL_SIZE: usize = 100;

/// クライアント配信用チャネルサイズ（各クライアント接続ごと）
///
/// こっちはクライアントとの接続処理に時間が掛かると少し詰まることがあるので大きめにしておく
const CLIENT_FRAME_CHANNEL_SIZE: usize = 500;

#[derive(Debug, Clone, Default)]
pub struct RtmpOutboundEndpointOptions {
    /// TLS接続時の証明書ファイルパス（オプション）
    pub cert_path: Option<PathBuf>,

    /// TLS接続時の秘密鍵ファイルパス（オプション）
    pub key_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct RtmpOutboundEndpoint {
    pub output_url: String,
    pub stream_name: Option<String>,
    pub input_audio_track_id: Option<TrackId>,
    pub input_video_track_id: Option<TrackId>,
    pub options: RtmpOutboundEndpointOptions,
}

impl nojson::DisplayJson for RtmpOutboundEndpoint {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("outputUrl", &self.output_url)?;
            if let Some(stream_name) = &self.stream_name {
                f.member("streamName", stream_name)?;
            }
            if let Some(track_id) = &self.input_audio_track_id {
                f.member("inputAudioTrackId", track_id)?;
            }
            if let Some(track_id) = &self.input_video_track_id {
                f.member("inputVideoTrackId", track_id)?;
            }
            if let Some(cert_path) = &self.options.cert_path {
                let cert_path = cert_path.display().to_string();
                f.member("certPath", &cert_path)?;
            }
            if let Some(key_path) = &self.options.key_path {
                let key_path = key_path.display().to_string();
                f.member("keyPath", &key_path)?;
            }
            Ok(())
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for RtmpOutboundEndpoint {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let output_url: String = value.to_member("outputUrl")?.required()?.try_into()?;
        let stream_name: Option<String> = value.to_member("streamName")?.try_into()?;
        let input_audio_track_id: Option<TrackId> =
            value.to_member("inputAudioTrackId")?.try_into()?;
        let input_video_track_id: Option<TrackId> =
            value.to_member("inputVideoTrackId")?.try_into()?;
        let cert_path: Option<String> = value.to_member("certPath")?.try_into()?;
        let key_path: Option<String> = value.to_member("keyPath")?.try_into()?;

        if input_audio_track_id.is_none() && input_video_track_id.is_none() {
            return Err(value.invalid("inputAudioTrackId or inputVideoTrackId is required"));
        }

        let stream_name = match stream_name {
            Some(stream_name) => {
                let trimmed = stream_name.trim();
                if trimmed.is_empty() {
                    return Err(value
                        .to_member("streamName")?
                        .required()?
                        .invalid("streamName must not be empty"));
                }
                Some(trimmed.to_owned())
            }
            None => None,
        };

        if cert_path.is_some() != key_path.is_some() {
            return Err(value.invalid("certPath and keyPath must be specified together"));
        }

        let url = match parse_rtmp_url(&output_url, stream_name.as_deref()) {
            Ok(url) => url,
            Err(e) => return Err(value.to_member("outputUrl")?.required()?.invalid(e)),
        };

        if url.tls && cert_path.is_none() {
            return Err(value.invalid("certPath and keyPath are required for rtmps"));
        }

        Ok(Self {
            output_url,
            stream_name,
            input_audio_track_id,
            input_video_track_id,
            options: RtmpOutboundEndpointOptions {
                cert_path: cert_path.map(PathBuf::from),
                key_path: key_path.map(PathBuf::from),
            },
        })
    }
}

impl RtmpOutboundEndpoint {
    pub async fn run(self, handle: ProcessorHandle) -> crate::Result<()> {
        let mut audio_rx = self
            .input_audio_track_id
            .clone()
            .map(|track_id| handle.subscribe_track(track_id));
        let mut video_rx = self
            .input_video_track_id
            .clone()
            .map(|track_id| handle.subscribe_track(track_id));

        handle.notify_ready();

        let url = parse_rtmp_url(&self.output_url, self.stream_name.as_deref())
            .map_err(|e| Error::new(format!("invalid outputUrl: {e}")))?;
        let (tx, rx) = tokio::sync::mpsc::channel(FRAME_CHANNEL_SIZE);

        let server_options = self.options.clone();
        let server_task = tokio::spawn(async move {
            let mut server = RtmpPlayServer {
                url,
                rx,
                clients: Vec::new(),
                options: server_options,
            };

            if let Err(e) = server.run().await {
                tracing::error!("RTMP play server error: {}", e.display());
                return Err(e);
            }
            Ok(())
        });

        loop {
            let mut close_audio = false;
            let mut close_video = false;
            match (audio_rx.as_mut(), video_rx.as_mut()) {
                (Some(audio_rx), Some(video_rx)) => {
                    tokio::select! {
                        message = audio_rx.recv() => {
                            if handle_audio_message(&self.input_audio_track_id, message, &tx)? {
                                close_audio = true;
                            }
                        }
                        message = video_rx.recv() => {
                            if handle_video_message(&self.input_video_track_id, message, &tx)? {
                                close_video = true;
                            }
                        }
                    }
                }
                (Some(audio_rx), None) => {
                    if handle_audio_message(&self.input_audio_track_id, audio_rx.recv().await, &tx)?
                    {
                        close_audio = true;
                    }
                }
                (None, Some(video_rx)) => {
                    if handle_video_message(&self.input_video_track_id, video_rx.recv().await, &tx)?
                    {
                        close_video = true;
                    }
                }
                (None, None) => break,
            }

            if close_audio {
                audio_rx = None;
            }
            if close_video {
                video_rx = None;
            }
        }

        drop(tx);
        match server_task.await {
            Ok(result) => result,
            Err(e) => Err(Error::new(format!(
                "rtmp outbound endpoint task failed: {e}"
            ))),
        }
    }
}

fn parse_rtmp_url(
    output_url: &str,
    stream_name: Option<&str>,
) -> std::result::Result<shiguredo_rtmp::RtmpUrl, String> {
    match stream_name {
        Some(stream_name) => {
            shiguredo_rtmp::RtmpUrl::parse_with_stream_name(output_url, stream_name)
                .map_err(|e| e.to_string())
        }
        None => shiguredo_rtmp::RtmpUrl::parse(output_url).map_err(|e| e.to_string()),
    }
}

fn handle_audio_message(
    track_id: &Option<TrackId>,
    message: Message,
    tx: &tokio::sync::mpsc::Sender<MediaSample>,
) -> crate::Result<bool> {
    match message {
        Message::Media(MediaSample::Audio(sample)) => {
            if sample.format != AudioFormat::Aac {
                return Err(Error::new(format!(
                    "unsupported audio codec: {}",
                    sample.format
                )));
            }
            tx.try_send(MediaSample::Audio(sample))
                .map_err(|e| Error::new(format!("failed to send audio frame: {e}")))?;
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

fn handle_video_message(
    track_id: &Option<TrackId>,
    message: Message,
    tx: &tokio::sync::mpsc::Sender<MediaSample>,
) -> crate::Result<bool> {
    match message {
        Message::Media(MediaSample::Video(sample)) => {
            if !matches!(sample.format, VideoFormat::H264 | VideoFormat::H264AnnexB) {
                return Err(Error::new(format!(
                    "unsupported video codec: {}",
                    sample.format
                )));
            }
            tx.try_send(MediaSample::Video(sample))
                .map_err(|e| Error::new(format!("failed to send video frame: {e}")))?;
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

/// クライアント配信用の内部メディアフレーム表現
#[derive(Debug, Clone)]
enum ClientMediaFrame {
    Audio(Arc<AudioData>),
    Video(Arc<VideoFrame>),
}

/// RTMP Play サーバー
#[derive(Debug)]
struct RtmpPlayServer {
    url: shiguredo_rtmp::RtmpUrl,
    rx: tokio::sync::mpsc::Receiver<MediaSample>,
    clients: Vec<tokio::sync::mpsc::Sender<ClientMediaFrame>>,
    options: RtmpOutboundEndpointOptions,
}

impl RtmpPlayServer {
    async fn run(&mut self) -> crate::Result<()> {
        let addr = format!("{}:{}", self.url.host, self.url.port);
        tracing::debug!("Starting RTMP outbound endpoint on {addr}");

        let listener = TcpListener::bind(&addr).await?;

        // URL スキームから TLS を判定（rtmps:// の場合は TLS を有効化）
        let tls_enabled = self.url.tls;
        tracing::debug!(
            "TLS is {}",
            if tls_enabled { "enabled" } else { "disabled" }
        );

        let tls_acceptor = if tls_enabled {
            let (cert_path, key_path) = self.get_cert_and_key_paths()?;
            Some(create_server_tls_acceptor(&cert_path, &key_path).await?)
        } else {
            None
        };

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    self.handle_new_client(accept_result, tls_acceptor.clone()).await?;
                }
                Some(sample) = self.rx.recv() => {
                    self.handle_media_sample(sample).await?;
                }
                else => {
                    break;
                }
            }
        }

        tracing::debug!("RTMP outbound endpoint finished");
        Ok(())
    }

    fn get_cert_and_key_paths(&self) -> crate::Result<(PathBuf, PathBuf)> {
        let cert_path = self
            .options
            .cert_path
            .clone()
            .ok_or_else(|| Error::new("Certificate path not specified"))?;
        let key_path = self
            .options
            .key_path
            .clone()
            .ok_or_else(|| Error::new("Private key path not specified"))?;
        Ok((cert_path, key_path))
    }

    async fn handle_new_client(
        &mut self,
        accept_result: std::io::Result<(TcpStream, std::net::SocketAddr)>,
        tls_acceptor: Option<Arc<tokio_rustls::TlsAcceptor>>,
    ) -> crate::Result<()> {
        let (stream, peer_addr) = accept_result?;
        tracing::debug!("New RTMP client connection from: {peer_addr}");

        let (client_tx, client_rx) = tokio::sync::mpsc::channel(CLIENT_FRAME_CHANNEL_SIZE);
        self.clients.push(client_tx);

        let expected_app = self.url.app.clone();
        let expected_stream_name = self.url.stream_name.clone();

        tokio::spawn(async move {
            match ServerTcpOrTlsStream::accept_with_tls(stream, tls_acceptor.as_ref()).await {
                Ok(tls_stream) => {
                    if tls_acceptor.is_some() {
                        tracing::debug!("TLS handshake successful with {peer_addr}");
                    }
                    let mut handler = RtmpClientHandler::new(
                        tls_stream,
                        client_rx,
                        expected_app,
                        expected_stream_name,
                    );

                    if let Err(e) = handler.run().await {
                        tracing::error!("RTMP client handler error: {}", e.display());
                    }
                    tracing::debug!("RTMP client disconnected: {peer_addr}");
                }
                Err(e) => {
                    tracing::error!("Connection setup failed with {peer_addr}: {e}");
                }
            }
        });

        Ok(())
    }

    async fn handle_media_sample(&mut self, sample: MediaSample) -> crate::Result<()> {
        let frame = match sample {
            MediaSample::Audio(audio) => ClientMediaFrame::Audio(audio),
            MediaSample::Video(video) => ClientMediaFrame::Video(video),
        };

        // NOTE: RtmpClientHandler が閉じたら削除したいので retain を使っている
        self.clients.retain(|tx| tx.try_send(frame.clone()).is_ok());
        Ok(())
    }
}

/// 個別のクライアント接続を処理する
#[derive(Debug)]
struct RtmpClientHandler {
    stream: ServerTcpOrTlsStream,
    connection: shiguredo_rtmp::RtmpServerConnection,
    rx: tokio::sync::mpsc::Receiver<ClientMediaFrame>,
    recv_buf: Vec<u8>,
    expected_app: String,
    expected_stream_name: String,
    frame_handler: crate::rtmp::RtmpOutgoingFrameHandler,
}

impl RtmpClientHandler {
    fn new(
        stream: ServerTcpOrTlsStream,
        rx: tokio::sync::mpsc::Receiver<ClientMediaFrame>,
        expected_app: String,
        expected_stream_name: String,
    ) -> Self {
        Self {
            stream,
            connection: shiguredo_rtmp::RtmpServerConnection::new(),
            rx,
            recv_buf: vec![0u8; 4096],
            expected_app,
            expected_stream_name,
            frame_handler: crate::rtmp::RtmpOutgoingFrameHandler::new(),
        }
    }

    async fn run(&mut self) -> crate::Result<()> {
        loop {
            while let Some(event) = self.connection.next_event() {
                tracing::debug!("RTMP event: {:?}", event);
                self.handle_event(event)?;
            }

            self.flush_send_buf().await?;

            tokio::select! {
                frame = self.rx.recv(), if self.connection.state() == shiguredo_rtmp::RtmpConnectionState::Playing => {
                    if !self.handle_media_frame_or_close(frame).await? {
                        break;
                    }
                }
                read_result = self.stream.read(&mut self.recv_buf) => {
                    if !self.handle_stream_read(read_result).await? {
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    fn handle_event(&mut self, event: shiguredo_rtmp::RtmpConnectionEvent) -> crate::Result<()> {
        match event {
            shiguredo_rtmp::RtmpConnectionEvent::PlayRequested {
                app, stream_name, ..
            } => {
                if app == self.expected_app && stream_name == self.expected_stream_name {
                    self.connection.accept().map_err(|e| {
                        Error::new(format!("failed to accept RTMP play request: {e}"))
                    })?;
                    tracing::debug!("Client started playing stream: {}/{}", app, stream_name);
                } else {
                    self.connection
                        .reject(&format!(
                            "Stream not found: {}/{}. Expected: {}/{}",
                            app, stream_name, self.expected_app, self.expected_stream_name
                        ))
                        .map_err(|e| {
                            Error::new(format!("failed to reject RTMP play request: {e}"))
                        })?;
                    tracing::warn!(
                        "Client requested invalid stream: {}/{}, expected: {}/{}",
                        app,
                        stream_name,
                        self.expected_app,
                        self.expected_stream_name
                    );
                }
            }
            shiguredo_rtmp::RtmpConnectionEvent::PublishRequested { .. } => {
                self.connection
                    .reject("Publishing is not supported by this server")
                    .map_err(|e| {
                        Error::new(format!(
                            "failed to reject RTMP publish request on play server: {e}"
                        ))
                    })?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_media_frame_or_close(
        &mut self,
        frame: Option<ClientMediaFrame>,
    ) -> crate::Result<bool> {
        match frame {
            Some(f) => {
                self.handle_client_media_frame(f)?;
                Ok(true)
            }
            None => {
                tracing::debug!("Media feed ended for client");
                Ok(false)
            }
        }
    }

    fn handle_client_media_frame(&mut self, frame: ClientMediaFrame) -> crate::Result<()> {
        match frame {
            ClientMediaFrame::Audio(audio) => {
                let (seq_frame, audio_frame) =
                    self.frame_handler.prepare_audio_frame(audio).map_err(|e| {
                        Error::new(format!("failed to prepare audio frame: {}", e.display()))
                    })?;
                if let Some(seq) = seq_frame {
                    self.connection.send_audio(seq).map_err(|e| {
                        Error::new(format!("failed to send audio sequence header: {e}"))
                    })?;
                }
                self.connection
                    .send_audio(audio_frame)
                    .map_err(|e| Error::new(format!("failed to send audio frame: {e}")))?;
            }
            ClientMediaFrame::Video(video) => {
                if let Some((seq_frame, video_frame)) =
                    self.frame_handler.prepare_video_frame(video).map_err(|e| {
                        Error::new(format!("failed to prepare video frame: {}", e.display()))
                    })?
                {
                    if let Some(seq) = seq_frame {
                        self.connection.send_video(seq).map_err(|e| {
                            Error::new(format!("failed to send video sequence header: {e}"))
                        })?;
                    }
                    self.connection
                        .send_video(video_frame)
                        .map_err(|e| Error::new(format!("failed to send video frame: {e}")))?;
                }
            }
        }
        Ok(())
    }

    async fn handle_stream_read(&mut self, result: std::io::Result<usize>) -> crate::Result<bool> {
        match result {
            Ok(0) => {
                tracing::debug!("Connection closed by client");
                Ok(false)
            }
            Ok(n) => {
                self.connection
                    .feed_recv_buf(&self.recv_buf[..n])
                    .map_err(|e| {
                        Error::new(format!(
                            "failed to feed received bytes to RTMP connection: {e}"
                        ))
                    })?;
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {
                // 正常終了扱い
                tracing::debug!("Connection closed by client");
                Ok(false)
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn flush_send_buf(&mut self) -> crate::Result<()> {
        while !self.connection.send_buf().is_empty() {
            let send_data = self.connection.send_buf();
            self.stream.write_all(send_data).await?;
            self.connection.advance_send_buf(send_data.len());
        }
        Ok(())
    }
}
