use std::path::PathBuf;
use std::time::Duration;

use orfail::OrFail;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::stats::{SharedAtomicCounter, SharedAtomicDuration, SharedAtomicFlag};
use crate::tcp::{ServerTcpOrTlsStream, create_server_tls_acceptor};

#[derive(Debug, Clone)]
pub struct RtmpInboundEndpointOptions {
    /// TLS接続時の証明書ファイルパス（オプション）
    pub cert_path: Option<PathBuf>,

    /// TLS接続時の秘密鍵ファイルパス（オプション）
    pub key_path: Option<PathBuf>,

    // サーバーの起動時間指定
    //
    // TODO: 暫定値（当面はこれでいいけど将来的には変更する）
    //       そもそも将来的には外部から停止できるようにするべきだが、今はそのための口が hisui にないのと、
    //       WriterMp4 が fmp4 に対応しておらず、finalize() を呼ばずにプロセスを停止すると再生できない
    //       MP4 ファイルができてしまうため、この設定を用意しているが、あくまでも暫定的なもの
    pub lifetime: Duration,
}

impl Default for RtmpInboundEndpointOptions {
    fn default() -> Self {
        Self {
            cert_path: None,
            key_path: None,
            lifetime: Duration::from_secs(60),
        }
    }
}

/// RTMP Inbound Endpoint
pub struct RtmpInboundEndpoint {
    url: shiguredo_rtmp::RtmpUrl,
    stats: RtmpInboundEndpointStats,
    options: RtmpInboundEndpointOptions,
}

impl RtmpInboundEndpoint {
    /// Create a new RTMP Inbound Endpoint
    pub fn new(url: shiguredo_rtmp::RtmpUrl, options: RtmpInboundEndpointOptions) -> Self {
        Self {
            url,
            stats: RtmpInboundEndpointStats::default(),
            options,
        }
    }

    /// Start the RTMP Inbound Endpoint
    pub async fn run(self, handle: crate::ProcessorHandle) -> crate::Result<()> {
        let addr = format!("{}:{}", self.url.host, self.url.port);
        tracing::debug!("Starting RTMP inbound endpoint on {addr}");

        let listener = TcpListener::bind(&addr).await?;

        let tls_enabled = self.url.tls;
        tracing::debug!(
            "TLS is {}",
            if tls_enabled { "enabled" } else { "disabled" }
        );

        let tls_acceptor = if tls_enabled {
            let (cert_path, key_path) = self
                .get_cert_and_key_paths()
                .map_err(|e| crate::Error::new(e.to_string()))?;
            Some(create_server_tls_acceptor(&cert_path, &key_path).await?)
        } else {
            None
        };

        let timeout = self.options.lifetime;
        let start_time = tokio::time::Instant::now();
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        loop {
            let elapsed = start_time.elapsed();
            if elapsed >= timeout {
                tracing::info!("RTMP server lifetime expired, shutting down");
                break;
            }
            let remaining = timeout - elapsed;

            match tokio::time::timeout(remaining, listener.accept()).await {
                Ok(Ok((stream, peer_addr))) => {
                    tracing::debug!("New RTMP client connection from: {peer_addr}");

                    let stats = self.stats.clone();

                    if stats.is_client_connected() {
                        tracing::warn!(
                            "Another client is already connected, rejecting new connection from {peer_addr}"
                        );
                        drop(stream);
                        continue;
                    }

                    let expected_app = self.url.app.clone();
                    let expected_stream_name = self.url.stream_name.clone();
                    let tls_acceptor = tls_acceptor.clone();
                    let timestamp_offset = start_time.elapsed();

                    let video_track_tx = handle
                        .publish_track(crate::TrackId::new(format!(
                            "{}_video",
                            self.url.stream_name
                        )))
                        .await?;

                    let audio_track_tx = handle
                        .publish_track(crate::TrackId::new(format!(
                            "{}_audio",
                            self.url.stream_name
                        )))
                        .await?;

                    tokio::spawn(async move {
                        let frame_handler_stats = crate::rtmp::RtmpIncomingFrameHandlerStats {
                            total_audio_frame_count: stats.total_audio_frame_count.clone(),
                            total_video_frame_count: stats.total_video_frame_count.clone(),
                            total_video_keyframe_count: stats.total_video_keyframe_count.clone(),
                            total_audio_sequence_header_count: stats
                                .total_audio_sequence_header_count
                                .clone(),
                            total_video_sequence_header_count: stats
                                .total_video_sequence_header_count
                                .clone(),
                        };

                        stats.total_connected_clients.increment();

                        match ServerTcpOrTlsStream::accept_with_tls(stream, tls_acceptor.as_ref())
                            .await
                        {
                            Ok(tls_stream) => {
                                if tls_acceptor.is_some() {
                                    tracing::debug!("TLS handshake successful with {peer_addr}");
                                }
                                let mut handler = RtmpPublisherHandler::new(
                                    tls_stream,
                                    stats.clone(),
                                    expected_app,
                                    expected_stream_name,
                                    frame_handler_stats,
                                    timestamp_offset,
                                    video_track_tx,
                                    audio_track_tx,
                                );

                                if let Err(e) = handler.run().await.or_fail() {
                                    tracing::error!("RTMP publisher handler error: {e}");
                                    handler.stats.total_error_disconnected_clients.increment();
                                }
                                handler.stats.total_disconnected_clients.increment();
                                tracing::debug!("RTMP publisher disconnected: {peer_addr}");
                            }
                            Err(e) => {
                                tracing::error!("Connection setup failed with {peer_addr}: {e}");
                                stats.total_error_disconnected_clients.increment();
                                stats.total_disconnected_clients.increment();
                            }
                        }
                    });
                }
                Ok(Err(e)) => return Err(e.into()),
                Err(_) => {
                    tracing::info!("RTMP server lifetime expired, shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    fn get_cert_and_key_paths(&self) -> orfail::Result<(PathBuf, PathBuf)> {
        let cert_path = self
            .options
            .cert_path
            .clone()
            .or_fail_with(|()| "Certificate path not specified".to_owned())?;
        let key_path = self
            .options
            .key_path
            .clone()
            .or_fail_with(|()| "Private key path not specified".to_owned())?;
        Ok((cert_path, key_path))
    }
}

/// 個別のクライアント（パブリッシャー）接続を処理する
#[derive(Debug)]
struct RtmpPublisherHandler {
    stream: ServerTcpOrTlsStream,
    connection: shiguredo_rtmp::RtmpServerConnection,
    recv_buf: Vec<u8>,
    stats: RtmpInboundEndpointStats,
    expected_app: String,
    expected_stream_name: String,
    frame_handler: crate::rtmp::RtmpIncomingFrameHandler,
    video_track_tx: crate::MessageSender,
    audio_track_tx: crate::MessageSender,
}

impl RtmpPublisherHandler {
    #[expect(clippy::too_many_arguments)]
    fn new(
        stream: ServerTcpOrTlsStream,
        stats: RtmpInboundEndpointStats,
        expected_app: String,
        expected_stream_name: String,
        frame_handler_stats: crate::rtmp::RtmpIncomingFrameHandlerStats,
        timestamp_offset: std::time::Duration,
        video_track_tx: crate::MessageSender,
        audio_track_tx: crate::MessageSender,
    ) -> Self {
        Self {
            stream,
            connection: shiguredo_rtmp::RtmpServerConnection::new(),
            recv_buf: vec![0u8; 4096],
            stats,
            expected_app,
            expected_stream_name,
            frame_handler: crate::rtmp::RtmpIncomingFrameHandler::new(
                timestamp_offset,
                frame_handler_stats,
            ),
            video_track_tx,
            audio_track_tx,
        }
    }

    async fn run(&mut self) -> orfail::Result<()> {
        loop {
            while let Some(event) = self.connection.next_event() {
                if !matches!(
                    event,
                    shiguredo_rtmp::RtmpConnectionEvent::AudioReceived(_)
                        | shiguredo_rtmp::RtmpConnectionEvent::VideoReceived(_)
                ) {
                    tracing::debug!("RTMP event: {:?}", event);
                }
                self.stats.total_event_count.increment();
                self.handle_event(&event).or_fail()?;
                self.process_event(event).await.or_fail()?;
            }

            self.flush_send_buf().await.or_fail()?;

            tokio::select! {
                read_result = self.stream.read(&mut self.recv_buf) => {
                    if !self.handle_stream_read(read_result).await.or_fail()? {
                        break;
                    }
                }
            }
        }

        self.video_track_tx.send_eos();
        self.audio_track_tx.send_eos();

        Ok(())
    }

    /// RTMP イベントを処理する
    async fn process_event(
        &mut self,
        event: shiguredo_rtmp::RtmpConnectionEvent,
    ) -> orfail::Result<()> {
        match event {
            shiguredo_rtmp::RtmpConnectionEvent::AudioReceived(frame) => {
                self.handle_audio_frame(frame).await.or_fail()?;
            }
            shiguredo_rtmp::RtmpConnectionEvent::VideoReceived(frame) => {
                self.handle_video_frame(frame).await.or_fail()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// RTMP イベントハンドラ（接続制御）
    fn handle_event(&mut self, event: &shiguredo_rtmp::RtmpConnectionEvent) -> orfail::Result<()> {
        match event {
            shiguredo_rtmp::RtmpConnectionEvent::PublishRequested {
                app, stream_name, ..
            } => {
                if app == &self.expected_app && stream_name == &self.expected_stream_name {
                    self.connection.accept().or_fail()?;
                    tracing::debug!("Client started publishing stream: {}/{}", app, stream_name);
                } else {
                    self.connection
                        .reject(&format!(
                            "Stream not found: {}/{}. Expected: {}/{}",
                            app, stream_name, self.expected_app, self.expected_stream_name
                        ))
                        .or_fail()?;
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
                    .reject("Playing is not supported by this server")
                    .or_fail()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// オーディオフレームを処理する
    async fn handle_audio_frame(
        &mut self,
        frame: shiguredo_rtmp::AudioFrame,
    ) -> orfail::Result<()> {
        let audio_data = self.frame_handler.process_audio_frame(frame)?;
        self.audio_track_tx
            .send_media(crate::MediaSample::Audio(std::sync::Arc::new(audio_data)));
        Ok(())
    }

    /// ビデオフレームを処理する
    async fn handle_video_frame(
        &mut self,
        frame: shiguredo_rtmp::VideoFrame,
    ) -> orfail::Result<()> {
        if let Some(video_frame) = self.frame_handler.process_video_frame(frame).or_fail()? {
            self.video_track_tx
                .send_media(crate::MediaSample::Video(std::sync::Arc::new(video_frame)));
        }
        Ok(())
    }

    /// TCP/TLS ストリームからデータを読み込む
    async fn handle_stream_read(&mut self, result: std::io::Result<usize>) -> orfail::Result<bool> {
        match result {
            Ok(0) => {
                tracing::debug!("Connection closed by publisher");
                Ok(false)
            }
            Ok(n) => {
                self.stats.total_received_bytes.add(n as u64);
                self.connection
                    .feed_recv_buf(&self.recv_buf[..n])
                    .or_fail()?;
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {
                tracing::debug!("Connection closed by publisher");
                Ok(false)
            }
            Err(e) => Err(e).or_fail(),
        }
    }

    /// 送信バッファをストリームにフラッシュする
    async fn flush_send_buf(&mut self) -> orfail::Result<()> {
        while !self.connection.send_buf().is_empty() {
            let send_data = self.connection.send_buf();
            self.stream.write_all(send_data).await.or_fail()?;
            self.stats.total_sent_bytes.add(send_data.len() as u64);
            self.connection.advance_send_buf(send_data.len());
        }
        Ok(())
    }
}

/// [`RtmpInboundEndpoint`] 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct RtmpInboundEndpointStats {
    pub total_audio_frame_count: SharedAtomicCounter,
    pub total_video_frame_count: SharedAtomicCounter,
    pub total_event_count: SharedAtomicCounter,
    pub total_sent_bytes: SharedAtomicCounter,
    pub total_received_bytes: SharedAtomicCounter,
    pub total_video_keyframe_count: SharedAtomicCounter,
    pub total_audio_sequence_header_count: SharedAtomicCounter,
    pub total_video_sequence_header_count: SharedAtomicCounter,
    pub total_processing_duration: SharedAtomicDuration,
    pub total_connected_clients: SharedAtomicCounter,
    pub total_disconnected_clients: SharedAtomicCounter,
    pub total_error_disconnected_clients: SharedAtomicCounter,
    pub error: SharedAtomicFlag,
}

impl RtmpInboundEndpointStats {
    fn is_client_connected(&self) -> bool {
        self.total_connected_clients.get() > self.total_disconnected_clients.get()
    }
}

impl nojson::DisplayJson for RtmpInboundEndpointStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "rtmp_inbound_endpoint")?;
            f.member("total_audio_frame_count", &self.total_audio_frame_count)?;
            f.member("total_video_frame_count", &self.total_video_frame_count)?;
            f.member("total_event_count", &self.total_event_count)?;
            f.member("total_sent_bytes", &self.total_sent_bytes)?;
            f.member("total_received_bytes", &self.total_received_bytes)?;
            f.member(
                "total_video_keyframe_count",
                &self.total_video_keyframe_count,
            )?;
            f.member(
                "total_audio_sequence_header_count",
                &self.total_audio_sequence_header_count,
            )?;
            f.member(
                "total_video_sequence_header_count",
                &self.total_video_sequence_header_count,
            )?;
            f.member("total_processing_seconds", &self.total_processing_duration)?;
            f.member("total_connected_clients", &self.total_connected_clients)?;
            f.member(
                "total_disconnected_clients",
                &self.total_disconnected_clients,
            )?;
            f.member(
                "total_error_disconnected_clients",
                &self.total_error_disconnected_clients,
            )?;
            f.member("error", self.error.get())?;
            Ok(())
        })
    }
}
