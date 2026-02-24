use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::tcp::{ServerTcpOrTlsStream, create_server_tls_acceptor};

#[derive(Debug, Clone, Default)]
pub struct RtmpInboundEndpointOptions {
    /// TLS接続時の証明書ファイルパス（オプション）
    pub cert_path: Option<PathBuf>,

    /// TLS接続時の秘密鍵ファイルパス（オプション）
    pub key_path: Option<PathBuf>,
}

/// RTMP Inbound Endpoint
pub struct RtmpInboundEndpoint {
    pub input_url: String,
    pub stream_name: Option<String>,
    pub output_audio_track_id: Option<crate::TrackId>,
    pub output_video_track_id: Option<crate::TrackId>,
    pub options: RtmpInboundEndpointOptions,
}

impl RtmpInboundEndpoint {
    /// Start the RTMP Inbound Endpoint
    pub async fn run(self, handle: crate::ProcessorHandle) -> crate::Result<()> {
        let url = parse_rtmp_url(&self.input_url, self.stream_name.as_deref())
            .map_err(|e| crate::Error::new(format!("invalid inputUrl: {e}")))?;
        let addr = format!("{}:{}", url.host, url.port);
        tracing::debug!("Starting RTMP inbound endpoint on {addr}");

        let listener = TcpListener::bind(&addr).await?;

        let tls_enabled = url.tls;
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

        let output_audio_track_id = self.output_audio_track_id.clone();
        let output_video_track_id = self.output_video_track_id.clone();
        let client_slots = Arc::new(tokio::sync::Semaphore::new(1));
        let server_started_at = tokio::time::Instant::now();
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    tracing::debug!("New RTMP client connection from: {peer_addr}");

                    let permit = match client_slots.clone().try_acquire_owned() {
                        Ok(permit) => permit,
                        Err(_) => {
                            tracing::warn!(
                                "Another client is already connected, rejecting new connection from {peer_addr}"
                            );
                            drop(stream);
                            continue;
                        }
                    };

                    let expected_app = url.app.clone();
                    let expected_stream_name = url.stream_name.clone();
                    let tls_acceptor = tls_acceptor.clone();
                    let timestamp_offset = server_started_at.elapsed();

                    let video_track_tx = if let Some(track_id) = &output_video_track_id {
                        Some(handle.publish_track(track_id.clone()).await?)
                    } else {
                        None
                    };

                    let audio_track_tx = if let Some(track_id) = &output_audio_track_id {
                        Some(handle.publish_track(track_id.clone()).await?)
                    } else {
                        None
                    };

                    tokio::spawn(async move {
                        let _permit = permit;

                        match ServerTcpOrTlsStream::accept_with_tls(stream, tls_acceptor.as_ref())
                            .await
                        {
                            Ok(tls_stream) => {
                                if tls_acceptor.is_some() {
                                    tracing::debug!("TLS handshake successful with {peer_addr}");
                                }
                                let mut handler = RtmpPublisherHandler::new(
                                    tls_stream,
                                    expected_app,
                                    expected_stream_name,
                                    timestamp_offset,
                                    video_track_tx,
                                    audio_track_tx,
                                );

                                if let Err(e) = handler.run().await {
                                    tracing::error!(
                                        "RTMP publisher handler error: {}",
                                        e.display()
                                    );
                                }
                                tracing::debug!("RTMP publisher disconnected: {peer_addr}");
                            }
                            Err(e) => {
                                tracing::error!("Connection setup failed with {peer_addr}: {e}");
                            }
                        }
                    });
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn get_cert_and_key_paths(&self) -> crate::Result<(PathBuf, PathBuf)> {
        let cert_path = self
            .options
            .cert_path
            .clone()
            .ok_or_else(|| crate::Error::new("Certificate path not specified"))?;
        let key_path = self
            .options
            .key_path
            .clone()
            .ok_or_else(|| crate::Error::new("Private key path not specified"))?;
        Ok((cert_path, key_path))
    }
}

impl nojson::DisplayJson for RtmpInboundEndpoint {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("inputUrl", &self.input_url)?;
            if let Some(stream_name) = &self.stream_name {
                f.member("streamName", stream_name)?;
            }
            if let Some(track_id) = &self.output_audio_track_id {
                f.member("outputAudioTrackId", track_id)?;
            }
            if let Some(track_id) = &self.output_video_track_id {
                f.member("outputVideoTrackId", track_id)?;
            }
            Ok(())
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for RtmpInboundEndpoint {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let input_url: String = value.to_member("inputUrl")?.required()?.try_into()?;
        let stream_name: Option<String> = value.to_member("streamName")?.try_into()?;
        let output_audio_track_id: Option<crate::TrackId> =
            value.to_member("outputAudioTrackId")?.try_into()?;
        let output_video_track_id: Option<crate::TrackId> =
            value.to_member("outputVideoTrackId")?.try_into()?;

        if output_audio_track_id.is_none() && output_video_track_id.is_none() {
            return Err(value.invalid("outputAudioTrackId or outputVideoTrackId is required"));
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

        if let Err(e) = parse_rtmp_url(&input_url, stream_name.as_deref()) {
            return Err(value.to_member("inputUrl")?.required()?.invalid(e));
        }

        Ok(Self {
            input_url,
            stream_name,
            output_audio_track_id,
            output_video_track_id,
            options: RtmpInboundEndpointOptions::default(),
        })
    }
}

fn parse_rtmp_url(
    input_url: &str,
    stream_name: Option<&str>,
) -> std::result::Result<shiguredo_rtmp::RtmpUrl, String> {
    match stream_name {
        Some(stream_name) => {
            shiguredo_rtmp::RtmpUrl::parse_with_stream_name(input_url, stream_name)
                .map_err(|e| e.to_string())
        }
        None => shiguredo_rtmp::RtmpUrl::parse(input_url).map_err(|e| e.to_string()),
    }
}

/// 個別のクライアント（パブリッシャー）接続を処理する
#[derive(Debug)]
struct RtmpPublisherHandler {
    stream: ServerTcpOrTlsStream,
    connection: shiguredo_rtmp::RtmpServerConnection,
    recv_buf: Vec<u8>,
    expected_app: String,
    expected_stream_name: String,
    frame_handler: crate::rtmp::RtmpIncomingFrameHandler,
    video_track_tx: Option<crate::MessageSender>,
    audio_track_tx: Option<crate::MessageSender>,
}

impl RtmpPublisherHandler {
    fn new(
        stream: ServerTcpOrTlsStream,
        expected_app: String,
        expected_stream_name: String,
        timestamp_offset: std::time::Duration,
        video_track_tx: Option<crate::MessageSender>,
        audio_track_tx: Option<crate::MessageSender>,
    ) -> Self {
        Self {
            stream,
            connection: shiguredo_rtmp::RtmpServerConnection::new(),
            recv_buf: vec![0u8; 4096],
            expected_app,
            expected_stream_name,
            frame_handler: crate::rtmp::RtmpIncomingFrameHandler::new(timestamp_offset),
            video_track_tx,
            audio_track_tx,
        }
    }

    async fn run(&mut self) -> crate::Result<()> {
        loop {
            while let Some(event) = self.connection.next_event() {
                if !matches!(
                    event,
                    shiguredo_rtmp::RtmpConnectionEvent::AudioReceived(_)
                        | shiguredo_rtmp::RtmpConnectionEvent::VideoReceived(_)
                ) {
                    tracing::debug!("RTMP event: {:?}", event);
                }
                self.handle_event(&event)?;
                self.process_event(event).await?;
            }

            self.flush_send_buf().await?;

            tokio::select! {
                read_result = self.stream.read(&mut self.recv_buf) => {
                    if !self.handle_stream_read(read_result).await? {
                        break;
                    }
                }
            }
        }

        if let Some(tx) = &mut self.video_track_tx {
            tx.send_eos();
        }
        if let Some(tx) = &mut self.audio_track_tx {
            tx.send_eos();
        }

        Ok(())
    }

    /// RTMP イベントを処理する
    async fn process_event(
        &mut self,
        event: shiguredo_rtmp::RtmpConnectionEvent,
    ) -> crate::Result<()> {
        match event {
            shiguredo_rtmp::RtmpConnectionEvent::AudioReceived(frame) => {
                self.handle_audio_frame(frame).await?;
            }
            shiguredo_rtmp::RtmpConnectionEvent::VideoReceived(frame) => {
                self.handle_video_frame(frame).await?;
            }
            _ => {}
        }
        Ok(())
    }

    /// RTMP イベントハンドラ（接続制御）
    fn handle_event(&mut self, event: &shiguredo_rtmp::RtmpConnectionEvent) -> crate::Result<()> {
        match event {
            shiguredo_rtmp::RtmpConnectionEvent::PublishRequested {
                app, stream_name, ..
            } => {
                if app == &self.expected_app && stream_name == &self.expected_stream_name {
                    self.connection.accept()?;
                    tracing::debug!("Client started publishing stream: {}/{}", app, stream_name);
                } else {
                    self.connection.reject(&format!(
                        "Stream not found: {}/{}. Expected: {}/{}",
                        app, stream_name, self.expected_app, self.expected_stream_name
                    ))?;
                    tracing::warn!(
                        "Client requested invalid stream: {}/{}, expected: {}/{}",
                        app,
                        stream_name,
                        self.expected_app,
                        self.expected_stream_name
                    );
                }
            }
            shiguredo_rtmp::RtmpConnectionEvent::PlayRequested { .. } => {
                self.connection
                    .reject("Playing is not supported by this server")?;
            }
            _ => {}
        }
        Ok(())
    }

    /// オーディオフレームを処理する
    async fn handle_audio_frame(&mut self, frame: shiguredo_rtmp::AudioFrame) -> crate::Result<()> {
        let audio_data = self.frame_handler.process_audio_frame(frame)?;
        if let Some(tx) = &mut self.audio_track_tx {
            tx.send_media(crate::MediaSample::Audio(std::sync::Arc::new(audio_data)));
        }
        Ok(())
    }

    /// ビデオフレームを処理する
    async fn handle_video_frame(&mut self, frame: shiguredo_rtmp::VideoFrame) -> crate::Result<()> {
        if let Some(video_frame) = self.frame_handler.process_video_frame(frame)?
            && let Some(tx) = &mut self.video_track_tx
        {
            tx.send_media(crate::MediaSample::Video(std::sync::Arc::new(video_frame)));
        }
        Ok(())
    }

    /// TCP/TLS ストリームからデータを読み込む
    async fn handle_stream_read(&mut self, result: std::io::Result<usize>) -> crate::Result<bool> {
        match result {
            Ok(0) => {
                tracing::debug!("Connection closed by publisher");
                Ok(false)
            }
            Ok(n) => {
                self.connection.feed_recv_buf(&self.recv_buf[..n])?;
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {
                tracing::debug!("Connection closed by publisher");
                Ok(false)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// 送信バッファをストリームにフラッシュする
    async fn flush_send_buf(&mut self) -> crate::Result<()> {
        while !self.connection.send_buf().is_empty() {
            let send_data = self.connection.send_buf();
            self.stream.write_all(send_data).await?;
            self.connection.advance_send_buf(send_data.len());
        }
        Ok(())
    }
}
