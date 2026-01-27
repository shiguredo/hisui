use orfail::OrFail;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::{
    audio::{AudioData, AudioFormat},
    media::{MediaSample, MediaStreamId},
    processor::{
        MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
        MediaProcessorWorkloadHint,
    },
    stats::{ProcessorStats, SharedAtomicCounter, SharedAtomicDuration, SharedAtomicFlag},
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

#[derive(Debug, Clone)]
pub struct RtmpOutboundEndpointOptions {
    /// TLSを使用するかどうか
    ///
    /// デフォルト値は false（平文のRTMP接続）
    pub tls: bool,

    /// TLS接続時の証明書検証を行うかどうか
    ///
    /// true（デフォルト）の場合、システムの証明書ストアを使用して
    /// サーバー証明書を検証します。
    ///
    /// false の場合、自己署名証明書などを許可します（開発環境向け）
    pub verify_certificate: bool,
}

impl Default for RtmpOutboundEndpointOptions {
    fn default() -> Self {
        Self {
            tls: false,
            verify_certificate: true,
        }
    }
}

/// クライアントからの play リクエストを受け付けてメディアストリームを配信するサーバー
#[derive(Debug)]
pub struct RtmpOutboundEndpoint {
    input_audio_stream_id: Option<MediaStreamId>,
    input_video_stream_id: Option<MediaStreamId>,
    tx: Option<tokio::sync::mpsc::Sender<MediaSample>>,
    stats: RtmpOutboundEndpointStats,
}

impl RtmpOutboundEndpoint {
    pub fn start(
        runtime: &tokio::runtime::Runtime,
        input_audio_stream_id: Option<MediaStreamId>,
        input_video_stream_id: Option<MediaStreamId>,
        url: shiguredo_rtmp::RtmpUrl,
        options: RtmpOutboundEndpointOptions,
    ) -> Self {
        let stats = RtmpOutboundEndpointStats::default();
        let (tx, rx) = tokio::sync::mpsc::channel(FRAME_CHANNEL_SIZE);

        let stats_clone = stats.clone();

        runtime.spawn(async move {
            let mut server = RtmpPlayServer {
                url: url.clone(),
                rx,
                clients: Vec::new(),
                stats: stats_clone.clone(),
                options,
            };

            if let Err(e) = server.run().await.or_fail() {
                log::error!("RTMP play server error: {e}");
                server.stats.error.set(true);
            }
        });

        Self {
            input_audio_stream_id,
            input_video_stream_id,
            tx: Some(tx),
            stats,
        }
    }
}

impl MediaProcessor for RtmpOutboundEndpoint {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: self
                .input_audio_stream_id
                .into_iter()
                .chain(self.input_video_stream_id)
                .collect(),
            output_stream_ids: Vec::new(),
            stats: ProcessorStats::RtmpOutboundEndpoint(self.stats.clone()),
            workload_hint: MediaProcessorWorkloadHint::ASYNC_IO,
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        match input.sample {
            Some(MediaSample::Audio(sample))
                if Some(input.stream_id) == self.input_audio_stream_id =>
            {
                (sample.format == AudioFormat::Aac)
                    .or_fail_with(|()| format!("unsupported audio codec: {}", sample.format))?;

                let tx = self.tx.as_ref().or_fail()?;
                tx.try_send(MediaSample::Audio(sample))
                    .or_fail_with(|e| format!("failed to send audio frame: {e}"))?;
            }
            None if Some(input.stream_id) == self.input_audio_stream_id => {
                self.input_audio_stream_id = None;
            }
            Some(MediaSample::Video(sample))
                if Some(input.stream_id) == self.input_video_stream_id =>
            {
                matches!(sample.format, VideoFormat::H264 | VideoFormat::H264AnnexB)
                    .or_fail_with(|()| format!("unsupported video codec: {}", sample.format))?;

                let tx = self.tx.as_ref().or_fail()?;
                tx.try_send(MediaSample::Video(sample))
                    .or_fail_with(|e| format!("failed to send video frame: {e}"))?;
            }
            None if Some(input.stream_id) == self.input_video_stream_id => {
                self.input_video_stream_id = None;
            }
            _ => return Err(orfail::Failure::new("BUG: unexpected input stream")),
        }
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        if self.input_audio_stream_id.is_some() || self.input_video_stream_id.is_some() {
            Ok(MediaProcessorOutput::awaiting_any())
        } else {
            self.tx = None;
            Ok(MediaProcessorOutput::Finished)
        }
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
    stats: RtmpOutboundEndpointStats,
    options: RtmpOutboundEndpointOptions,
}

impl RtmpPlayServer {
    async fn run(&mut self) -> orfail::Result<()> {
        let addr = format!("{}:{}", self.url.host, self.url.port);
        log::debug!("Starting RTMP outbound endpoint on {addr}");

        let listener = TcpListener::bind(&addr).await.or_fail()?;

        // TLS用の設定をここで準備
        let tls_acceptor = if self.options.tls {
            Some(self.create_tls_acceptor().await.or_fail()?)
        } else {
            None
        };

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    self.handle_new_client(accept_result, tls_acceptor.clone()).await.or_fail()?;
                }
                Some(sample) = self.rx.recv() => {
                    self.handle_media_sample(sample).await.or_fail()?;
                }
                else => {
                    break;
                }
            }
        }

        log::debug!("RTMP outbound endpoint finished");
        Ok(())
    }

    /// TLS Acceptor を作成する
    async fn create_tls_acceptor(&self) -> orfail::Result<Arc<tokio_rustls::TlsAcceptor>> {
        if !self.options.tls {
            return Err(orfail::Failure::new("TLS is not enabled"));
        }

        // TLS設定をプラットフォームの証明書ストアを使用して作成
        let config = if self.options.verify_certificate {
            // 証明書検証を有効にする場合
            log::debug!("Creating TLS config with certificate verification enabled");

            // NOTE: 実装では、実際のサーバー証明書と秘密鍵を読み込む必要があります
            // ここではプレースホルダーとして示しています
            // 実際の実装例は、examples/server/src/main.rs を参照してください
            self.load_server_config_with_verification()
                .await
                .or_fail()?
        } else {
            // 証明書検証を無効にする場合（自己署名証明書対応）
            log::debug!("Creating TLS config with certificate verification disabled");
            self.load_server_config_without_verification()
                .await
                .or_fail()?
        };

        Ok(Arc::new(tokio_rustls::TlsAcceptor::from(Arc::new(config))))
    }

    /// 証明書検証を有効にしてサーバー設定を読み込む
    async fn load_server_config_with_verification(&self) -> orfail::Result<rustls::ServerConfig> {
        // 実装例：環境変数から証明書パスを読み込む
        let cert_path =
            std::env::var("RTMP_CERT_PATH").unwrap_or_else(|_| "./cert.pem".to_string());
        let key_path = std::env::var("RTMP_KEY_PATH").unwrap_or_else(|_| "./key.pem".to_string());

        self.load_server_config(&cert_path, &key_path).await
    }

    /// 証明書検証を無効にしてサーバー設定を読み込む
    async fn load_server_config_without_verification(
        &self,
    ) -> orfail::Result<rustls::ServerConfig> {
        // 実装例：環境変数から証明書パスを読み込む
        let cert_path =
            std::env::var("RTMP_CERT_PATH").unwrap_or_else(|_| "./cert.pem".to_string());
        let key_path = std::env::var("RTMP_KEY_PATH").unwrap_or_else(|_| "./key.pem".to_string());

        self.load_server_config(&cert_path, &key_path).await
    }

    /// サーバー設定を読み込む
    async fn load_server_config(
        &self,
        cert_path: &str,
        key_path: &str,
    ) -> orfail::Result<rustls::ServerConfig> {
        use rustls::pki_types::pem::PemObject;

        log::debug!("Loading TLS certificates from {}", cert_path);

        // 証明書ファイルを読み込む
        let certs = rustls::pki_types::CertificateDer::pem_file_iter(cert_path)
            .or_fail_with(|e| format!("Failed to open certificate file: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .or_fail_with(|e| format!("Failed to parse certificate file: {e}"))?;

        if certs.is_empty() {
            return Err(orfail::Failure::new("No certificates found in cert file"));
        }

        log::debug!("Loaded {} certificate(s)", certs.len());

        // 秘密鍵ファイルを読み込む
        log::debug!("Loading private key from {}", key_path);
        let key = rustls::pki_types::PrivateKeyDer::from_pem_file(key_path)
            .or_fail_with(|e| format!("Failed to load private key: {e}"))?;

        // ServerConfig を作成
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .or_fail_with(|e| format!("Failed to create server config: {e}"))?;

        Ok(config)
    }

    /// クライアント接続を受け付ける
    async fn handle_new_client(
        &mut self,
        accept_result: std::io::Result<(TcpStream, std::net::SocketAddr)>,
        tls_acceptor: Option<Arc<tokio_rustls::TlsAcceptor>>,
    ) -> orfail::Result<()> {
        let (stream, peer_addr) = accept_result.or_fail()?;
        log::debug!("New RTMP client connection from: {peer_addr}");

        let (client_tx, client_rx) = tokio::sync::mpsc::channel(CLIENT_FRAME_CHANNEL_SIZE);
        self.clients.push(client_tx);

        let stats = self.stats.clone();
        let expected_app = self.url.app.clone();
        let expected_stream_name = self.url.stream_name.clone();
        let tls_enabled = self.options.tls;

        tokio::spawn(async move {
            // [NOTE]
            // 統計の各フィールドはアトミック変数で共有されているので、
            // frame handler の中で更新されれば、大元にも反映される
            let frame_handler_stats = crate::rtmp::RtmpOutgoingFrameHandlerStats {
                total_audio_frame_count: stats.total_audio_frame_count.clone(),
                total_video_frame_count: stats.total_video_frame_count.clone(),
                total_video_keyframe_count: stats.total_video_keyframe_count.clone(),
                total_audio_sequence_header_count: stats.total_audio_sequence_header_count.clone(),
                total_video_sequence_header_count: stats.total_video_sequence_header_count.clone(),
            };

            stats.total_connected_clients.increment();

            if tls_enabled {
                // TLS接続の処理
                if let Some(acceptor) = tls_acceptor {
                    match acceptor.accept(stream).await {
                        Ok(tls_stream) => {
                            log::debug!("TLS handshake successful with {peer_addr}");
                            let mut handler = RtmpClientHandler::new_tls(
                                tls_stream,
                                client_rx,
                                stats.clone(),
                                expected_app,
                                expected_stream_name,
                                frame_handler_stats,
                            );

                            if let Err(e) = handler.run().await.or_fail() {
                                log::error!("RTMP client handler error: {e}");
                                handler.stats.total_error_disconnected_clients.increment();
                            }
                            handler.stats.total_disconnected_clients.increment();
                            log::debug!("RTMP client disconnected: {peer_addr}");
                        }
                        Err(e) => {
                            log::error!("TLS handshake failed with {peer_addr}: {e}");
                            stats.total_error_disconnected_clients.increment();
                            stats.total_disconnected_clients.increment();
                        }
                    }
                } else {
                    log::error!("TLS requested but acceptor not available for {peer_addr}");
                    stats.total_error_disconnected_clients.increment();
                    stats.total_disconnected_clients.increment();
                }
            } else {
                // 平文接続の処理
                let mut handler = RtmpClientHandler::new_plain(
                    stream,
                    client_rx,
                    stats.clone(),
                    expected_app,
                    expected_stream_name,
                    frame_handler_stats,
                );

                if let Err(e) = handler.run().await.or_fail() {
                    log::error!("RTMP client handler error: {e}");
                    handler.stats.total_error_disconnected_clients.increment();
                }
                handler.stats.total_disconnected_clients.increment();
                log::debug!("RTMP client disconnected: {peer_addr}");
            }
        });

        Ok(())
    }

    /// メディアサンプルを受け取り、すべてのプレイヤーに配信する
    async fn handle_media_sample(&mut self, sample: MediaSample) -> orfail::Result<()> {
        let frame = match sample {
            MediaSample::Audio(audio) => ClientMediaFrame::Audio(audio),
            MediaSample::Video(video) => ClientMediaFrame::Video(video),
        };

        // NOTE: RtmpClientHandler が閉じたら削除したいので retain を使っている
        self.clients.retain(|tx| tx.try_send(frame.clone()).is_ok());
        Ok(())
    }
}

/// TCP/TLS ストリーム抽象化
#[derive(Debug)]
enum RtmpClientStream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::server::TlsStream<TcpStream>>),
}

impl RtmpClientStream {
    async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            RtmpClientStream::Plain(s) => s.read(buf).await,
            RtmpClientStream::Tls(s) => s.read(buf).await,
        }
    }

    async fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        match self {
            RtmpClientStream::Plain(s) => s.write_all(buf).await,
            RtmpClientStream::Tls(s) => s.write_all(buf).await,
        }
    }
}

/// 個別のクライアント接続を処理する
#[derive(Debug)]
struct RtmpClientHandler {
    stream: RtmpClientStream,
    connection: shiguredo_rtmp::RtmpServerConnection,
    rx: tokio::sync::mpsc::Receiver<ClientMediaFrame>,
    recv_buf: Vec<u8>,
    stats: RtmpOutboundEndpointStats,
    expected_app: String,
    expected_stream_name: String,
    frame_handler: crate::rtmp::RtmpOutgoingFrameHandler,
}

impl RtmpClientHandler {
    fn new_plain(
        stream: TcpStream,
        rx: tokio::sync::mpsc::Receiver<ClientMediaFrame>,
        stats: RtmpOutboundEndpointStats,
        expected_app: String,
        expected_stream_name: String,
        frame_handler_stats: crate::rtmp::RtmpOutgoingFrameHandlerStats,
    ) -> Self {
        Self {
            stream: RtmpClientStream::Plain(stream),
            connection: shiguredo_rtmp::RtmpServerConnection::new(),
            rx,
            recv_buf: vec![0u8; 4096],
            stats,
            expected_app,
            expected_stream_name,
            frame_handler: crate::rtmp::RtmpOutgoingFrameHandler::new(frame_handler_stats),
        }
    }

    fn new_tls(
        stream: tokio_rustls::server::TlsStream<TcpStream>,
        rx: tokio::sync::mpsc::Receiver<ClientMediaFrame>,
        stats: RtmpOutboundEndpointStats,
        expected_app: String,
        expected_stream_name: String,
        frame_handler_stats: crate::rtmp::RtmpOutgoingFrameHandlerStats,
    ) -> Self {
        Self {
            stream: RtmpClientStream::Tls(Box::new(stream)),
            connection: shiguredo_rtmp::RtmpServerConnection::new(),
            rx,
            recv_buf: vec![0u8; 4096],
            stats,
            expected_app,
            expected_stream_name,
            frame_handler: crate::rtmp::RtmpOutgoingFrameHandler::new(frame_handler_stats),
        }
    }

    async fn run(&mut self) -> orfail::Result<()> {
        loop {
            while let Some(event) = self.connection.next_event() {
                log::debug!("RTMP event: {:?}", event);
                self.stats.total_event_count.increment();
                self.handle_event(event).or_fail()?;
            }

            self.flush_send_buf().await.or_fail()?;

            tokio::select! {
                frame = self.rx.recv(), if self.connection.state() == shiguredo_rtmp::RtmpConnectionState::Playing => {
                    if !self.handle_media_frame_or_close(frame).await.or_fail()? {
                        break;
                    }
                }
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
            shiguredo_rtmp::RtmpConnectionEvent::PlayRequested {
                app, stream_name, ..
            } => {
                if app == self.expected_app && stream_name == self.expected_stream_name {
                    self.connection.accept().or_fail()?;
                    log::debug!("Client started playing stream: {}/{}", app, stream_name);
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
            shiguredo_rtmp::RtmpConnectionEvent::PublishRequested { .. } => {
                self.connection
                    .reject("Publishing is not supported by this server")
                    .or_fail()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// メディアフレームを処理するか、閉じるかのいずれかを行う
    async fn handle_media_frame_or_close(
        &mut self,
        frame: Option<ClientMediaFrame>,
    ) -> orfail::Result<bool> {
        match frame {
            Some(f) => {
                self.handle_client_media_frame(f).or_fail()?;
                Ok(true)
            }
            None => {
                log::debug!("Media feed ended for client");
                Ok(false)
            }
        }
    }

    /// クライアント用メディアフレームを処理する
    fn handle_client_media_frame(&mut self, frame: ClientMediaFrame) -> orfail::Result<()> {
        match frame {
            ClientMediaFrame::Audio(audio) => {
                let (seq_frame, audio_frame) = self.frame_handler.prepare_audio_frame(audio)?;
                if let Some(seq) = seq_frame {
                    self.connection.send_audio(seq).or_fail()?;
                }
                self.connection.send_audio(audio_frame).or_fail()?;
            }
            ClientMediaFrame::Video(video) => {
                if let Some((seq_frame, video_frame)) =
                    self.frame_handler.prepare_video_frame(video)?
                {
                    if let Some(seq) = seq_frame {
                        self.connection.send_video(seq).or_fail()?;
                    }
                    self.connection.send_video(video_frame).or_fail()?;
                }
            }
        }
        Ok(())
    }

    /// TCP/TLS ストリームからデータを読み込み、RTMP 接続に供給する
    async fn handle_stream_read(&mut self, result: std::io::Result<usize>) -> orfail::Result<bool> {
        match result {
            Ok(0) => {
                log::debug!("Connection closed by client");
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
                // 正常終了扱い
                log::debug!("Connection closed by client");
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

/// [`RtmpOutboundEndpoint`] 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct RtmpOutboundEndpointStats {
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

impl nojson::DisplayJson for RtmpOutboundEndpointStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "rtmp_outbound_endpoint")?;
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
