use std::path::PathBuf;
use std::time::Duration;

use orfail::OrFail;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::tcp::{ServerTcpOrTlsStream, create_server_tls_acceptor};
use crate::{
    media::{MediaSample, MediaStreamId},
    processor::{
        MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
        MediaProcessorWorkloadHint,
    },
    stats::{ProcessorStats, SharedAtomicCounter, SharedAtomicDuration, SharedAtomicFlag},
};

/// メディアフレーム用チャネルサイズ
const FRAME_CHANNEL_SIZE: usize = 500;

#[derive(Debug, Clone)]
pub struct RtmpInboundEndpointOptions {
    /// TLS接続時の証明書ファイルパス（オプション）
    pub cert_path: Option<PathBuf>,

    /// TLS接続時の秘密鍵ファイルパス（オプション）
    pub key_path: Option<PathBuf>,

    // サーバーの起動時間指定
    pub lifetime: Duration,
}

impl Default for RtmpInboundEndpointOptions {
    fn default() -> Self {
        Self {
            cert_path: None,
            key_path: None,

            // TODO: 暫定値（当面はこれでいいけど将来的には変更する）
            //       そもそも将来的には外部から停止できるようにするべきだが、今はそのための口が hisui にないのと、
            //       WriterMp4 が fmp4 に対応しておらず、finalize() を呼ばずにプロセスを停止すると再生できない
            //       MP4 ファイルができてしまうため、この設定を用意しているが、あくまでも暫定的なもの
            lifetime: Duration::from_secs(60),
        }
    }
}

/// クライアントからの publish リクエストを受け付けてメディアストリームを受信するサーバー
#[derive(Debug)]
pub struct RtmpInboundEndpoint {
    output_audio_stream_id: Option<MediaStreamId>,
    output_video_stream_id: Option<MediaStreamId>,
    rx: tokio::sync::mpsc::Receiver<MediaSample>,
    stats: RtmpInboundEndpointStats,
}

impl RtmpInboundEndpoint {
    pub fn start(
        runtime: &tokio::runtime::Runtime,
        output_audio_stream_id: Option<MediaStreamId>,
        output_video_stream_id: Option<MediaStreamId>,
        url: shiguredo_rtmp::RtmpUrl,
        options: RtmpInboundEndpointOptions,
    ) -> Self {
        let stats = RtmpInboundEndpointStats::default();
        let (tx, rx) = tokio::sync::mpsc::channel(FRAME_CHANNEL_SIZE);

        let stats_clone = stats.clone();

        // TODO: 二回目のクライアントではタイムスタンプを調整する（オフセットを入れる）
        runtime.spawn(async move {
            let mut server = RtmpPublishServer {
                url: url.clone(),
                tx,
                stats: stats_clone.clone(),
                options,
            };

            if let Err(e) = server.run().await.or_fail() {
                log::error!("RTMP publish server error: {e}");
                server.stats.error.set(true);
            }
        });

        Self {
            output_audio_stream_id,
            output_video_stream_id,
            rx,
            stats,
        }
    }
}

impl MediaProcessor for RtmpInboundEndpoint {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: Vec::new(),
            output_stream_ids: self
                .output_audio_stream_id
                .into_iter()
                .chain(self.output_video_stream_id)
                .collect(),
            stats: ProcessorStats::RtmpInboundEndpoint(self.stats.clone()),
            workload_hint: MediaProcessorWorkloadHint::ASYNC_IO,
        }
    }

    fn process_input(&mut self, _input: MediaProcessorInput) -> orfail::Result<()> {
        // Inbound endpoint には別のプロセッサからの入力は来ない
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        loop {
            match self.rx.try_recv() {
                Ok(sample) => {
                    let stream_id = match &sample {
                        MediaSample::Audio(_) => self.output_audio_stream_id.or_fail()?,
                        MediaSample::Video(_) => self.output_video_stream_id.or_fail()?,
                    };
                    return Ok(MediaProcessorOutput::Processed { stream_id, sample });
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    // 特に入力を待っている訳ではないけど、現状では他に適切なものがないので awaiting_any() を返しておく
                    // TODO: Ok(MediaProcessorOutput::awaiting_any())
                    std::thread::sleep(std::time::Duration::from_millis(10)); // TODO
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    return Ok(MediaProcessorOutput::Finished);
                }
            }
        }
    }
}

/// RTMP Publish サーバー
#[derive(Debug)]
struct RtmpPublishServer {
    url: shiguredo_rtmp::RtmpUrl,
    tx: tokio::sync::mpsc::Sender<MediaSample>,
    stats: RtmpInboundEndpointStats,
    options: RtmpInboundEndpointOptions,
}

impl RtmpPublishServer {
    async fn run(&mut self) -> orfail::Result<()> {
        let addr = format!("{}:{}", self.url.host, self.url.port);
        log::debug!("Starting RTMP inbound endpoint on {addr}");

        let listener = TcpListener::bind(&addr).await.or_fail()?;

        // URL スキームから TLS を判定（rtmps:// の場合は TLS を有効化）
        let tls_enabled = self.url.tls;
        log::debug!(
            "TLS is {}",
            if tls_enabled { "enabled" } else { "disabled" }
        );

        // TLS Acceptor を作成（共通化されたヘルパー関数を使用）
        let tls_acceptor = if tls_enabled {
            let (cert_path, key_path) = self.get_cert_and_key_paths().or_fail()?;
            Some(
                create_server_tls_acceptor(&cert_path, &key_path)
                    .await
                    .or_fail()?,
            )
        } else {
            None
        };

        // lifetime タイムアウトを設定
        let timeout = self.options.lifetime;
        let start_time = tokio::time::Instant::now();

        loop {
            // 残り時間を計算
            let elapsed = start_time.elapsed();
            if elapsed >= timeout {
                log::info!("RTMP server lifetime expired, shutting down");
                break;
            }
            let remaining = timeout - elapsed;

            // 残り時間でタイムアウトを設定して accept を待機
            match tokio::time::timeout(remaining, listener.accept()).await {
                Ok(Ok((stream, peer_addr))) => {
                    log::debug!("New RTMP client connection from: {peer_addr}");

                    let stats = self.stats.clone();

                    // 他のクライアントがすでに接続済みかどうかをチェックする
                    // （同じエンドポイントに一度に配信できるのは一人のみ）
                    if stats.is_client_connected() {
                        log::warn!(
                            "Another client is already connected, rejecting new connection from {peer_addr}"
                        );
                        drop(stream);
                        continue;
                    }

                    let tx = self.tx.clone();
                    let expected_app = self.url.app.clone();
                    let expected_stream_name = self.url.stream_name.clone();
                    let tls_acceptor = tls_acceptor.clone();

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
                                    log::debug!("TLS handshake successful with {peer_addr}");
                                }
                                let mut handler = RtmpPublisherHandler::new(
                                    tls_stream,
                                    tx,
                                    stats.clone(),
                                    expected_app,
                                    expected_stream_name,
                                    frame_handler_stats,
                                );

                                if let Err(e) = handler.run().await.or_fail() {
                                    log::error!("RTMP publisher handler error: {e}");
                                    handler.stats.total_error_disconnected_clients.increment();
                                }
                                handler.stats.total_disconnected_clients.increment();
                                log::debug!("RTMP publisher disconnected: {peer_addr}");
                            }
                            Err(e) => {
                                log::error!("Connection setup failed with {peer_addr}: {e}");
                                stats.total_error_disconnected_clients.increment();
                                stats.total_disconnected_clients.increment();
                            }
                        }
                    });
                }
                Ok(Err(e)) => return Err(e).or_fail(),
                Err(_) => {
                    log::info!("RTMP server lifetime expired, shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    /// 証明書と秘密鍵のパスを取得する
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
    tx: tokio::sync::mpsc::Sender<MediaSample>,
    recv_buf: Vec<u8>,
    stats: RtmpInboundEndpointStats,
    expected_app: String,
    expected_stream_name: String,
    frame_handler: crate::rtmp::RtmpIncomingFrameHandler,
}

impl RtmpPublisherHandler {
    fn new(
        stream: ServerTcpOrTlsStream,
        tx: tokio::sync::mpsc::Sender<MediaSample>,
        stats: RtmpInboundEndpointStats,
        expected_app: String,
        expected_stream_name: String,
        frame_handler_stats: crate::rtmp::RtmpIncomingFrameHandlerStats,
    ) -> Self {
        Self {
            stream,
            connection: shiguredo_rtmp::RtmpServerConnection::new(),
            tx,
            recv_buf: vec![0u8; 4096],
            stats,
            expected_app,
            expected_stream_name,
            frame_handler: crate::rtmp::RtmpIncomingFrameHandler::new(frame_handler_stats),
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
                    log::debug!("RTMP event: {:?}", event);
                }
                self.stats.total_event_count.increment();
                self.handle_event(event).or_fail()?;
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

        Ok(())
    }

    /// RTMP イベントを処理する
    fn handle_event(&mut self, event: shiguredo_rtmp::RtmpConnectionEvent) -> orfail::Result<()> {
        match event {
            shiguredo_rtmp::RtmpConnectionEvent::PublishRequested {
                app, stream_name, ..
            } => {
                if app == self.expected_app && stream_name == self.expected_stream_name {
                    self.connection.accept().or_fail()?;
                    log::debug!("Client started publishing stream: {}/{}", app, stream_name);
                } else {
                    self.connection
                        .reject(&format!(
                            "Stream not found: {}/{}. Expected: {}/{}",
                            app, stream_name, self.expected_app, self.expected_stream_name
                        ))
                        .or_fail()?;
                    log::warn!(
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
            shiguredo_rtmp::RtmpConnectionEvent::AudioReceived(frame) => {
                self.handle_audio_frame(frame).or_fail()?;
            }
            shiguredo_rtmp::RtmpConnectionEvent::VideoReceived(frame) => {
                self.handle_video_frame(frame).or_fail()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// オーディオフレームを処理する
    fn handle_audio_frame(&mut self, frame: shiguredo_rtmp::AudioFrame) -> orfail::Result<()> {
        let audio_data = self.frame_handler.process_audio_frame(frame)?;
        self.tx
            .try_send(MediaSample::Audio(std::sync::Arc::new(audio_data)))
            .or_fail()?;
        Ok(())
    }

    /// ビデオフレームを処理する
    fn handle_video_frame(&mut self, frame: shiguredo_rtmp::VideoFrame) -> orfail::Result<()> {
        if let Some(video_frame) = self.frame_handler.process_video_frame(frame).or_fail()? {
            self.tx
                .try_send(MediaSample::Video(std::sync::Arc::new(video_frame)))
                .or_fail()?;
        }
        Ok(())
    }

    /// TCP/TLS ストリームからデータを読み込み、RTMP 接続に供給する
    async fn handle_stream_read(&mut self, result: std::io::Result<usize>) -> orfail::Result<bool> {
        match result {
            Ok(0) => {
                log::debug!("Connection closed by publisher");
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
                log::debug!("Connection closed by publisher");
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
    /// クライアントが接続中かどうかを判定する
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
