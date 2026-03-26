use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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

/// 1 つのストリームキーに対応する設定
pub struct RtmpInboundStream {
    pub stream_name: String,
    pub output_audio_track_id: Option<crate::TrackId>,
    pub output_video_track_id: Option<crate::TrackId>,
}

/// RTMP Inbound Endpoint（1 ポートで複数ストリームを受信）
pub struct RtmpInboundEndpoint {
    pub input_url: String,
    pub streams: Vec<RtmpInboundStream>,
    pub options: RtmpInboundEndpointOptions,
}

/// ストリームごとのランタイム状態
struct StreamSlot {
    video_track_tx: Option<crate::MessageSender>,
    audio_track_tx: Option<crate::MessageSender>,
}

/// 共有状態（accept ループと各 handler タスクで共有）
type StreamSlots = Mutex<HashMap<String, StreamSlot>>;

#[derive(Debug, Clone)]
struct RtmpInboundEndpointStats {
    is_listening_metric: crate::stats::StatsFlag,
    audio_codec_metric: crate::stats::StatsString,
    total_input_audio_data_count_metric: crate::stats::StatsCounter,
    last_input_audio_timestamp_metric: crate::stats::StatsDuration,
    video_codec_metric: crate::stats::StatsString,
    total_input_video_frame_count_metric: crate::stats::StatsCounter,
    last_input_video_timestamp_metric: crate::stats::StatsDuration,
}

impl RtmpInboundEndpointStats {
    fn new(mut stats: crate::stats::Stats) -> Self {
        Self {
            is_listening_metric: stats.flag("is_listening"),
            audio_codec_metric: stats.string("audio_codec"),
            total_input_audio_data_count_metric: stats.counter("total_input_audio_data_count"),
            last_input_audio_timestamp_metric: stats.duration("last_input_audio_timestamp"),
            video_codec_metric: stats.string("video_codec"),
            total_input_video_frame_count_metric: stats.counter("total_input_video_frame_count"),
            last_input_video_timestamp_metric: stats.duration("last_input_video_timestamp"),
        }
    }

    fn set_audio_codec(&self, codec: crate::types::CodecName) {
        self.audio_codec_metric.set(codec.as_str());
    }

    fn add_input_audio_data_count(&self) {
        self.total_input_audio_data_count_metric.inc();
    }

    fn set_last_input_audio_timestamp(&self, timestamp: std::time::Duration) {
        self.last_input_audio_timestamp_metric.set(timestamp);
    }

    fn set_video_codec(&self, codec: crate::types::CodecName) {
        self.video_codec_metric.set(codec.as_str());
    }

    fn add_input_video_frame_count(&self) {
        self.total_input_video_frame_count_metric.inc();
    }

    fn set_last_input_video_timestamp(&self, timestamp: std::time::Duration) {
        self.last_input_video_timestamp_metric.set(timestamp);
    }

    fn set_listening(&self, value: bool) {
        self.is_listening_metric.set(value);
    }
}

impl RtmpInboundEndpoint {
    /// Start the RTMP Inbound Endpoint
    pub async fn run(self, handle: crate::ProcessorHandle) -> crate::Result<()> {
        // input_url を stream_name なしでパース（バインドアドレス取得用）
        let url = shiguredo_rtmp::RtmpUrl::parse(&self.input_url)
            .map_err(|e| crate::Error::new(format!("invalid inputUrl: {e}")))?;
        let addr = format!("{}:{}", url.host, url.port);
        tracing::debug!("Starting RTMP inbound endpoint on {addr}");

        // 各ストリームの stream_name でバリデーション
        for stream in &self.streams {
            shiguredo_rtmp::RtmpUrl::parse_with_stream_name(&self.input_url, &stream.stream_name)
                .map_err(|e| {
                crate::Error::new(format!(
                    "invalid inputUrl with streamName '{}': {e}",
                    stream.stream_name
                ))
            })?;
        }

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

        let stats = RtmpInboundEndpointStats::new(handle.stats());
        stats.set_listening(true);
        let server_started_at = tokio::time::Instant::now();
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        // 各ストリームのトラック送信者を初期化して StreamSlot に格納
        let mut slots = HashMap::new();
        for stream in self.streams {
            let video_track_tx = if let Some(track_id) = stream.output_video_track_id {
                Some(handle.publish_track(track_id).await?)
            } else {
                None
            };
            let audio_track_tx = if let Some(track_id) = stream.output_audio_track_id {
                Some(handle.publish_track(track_id).await?)
            } else {
                None
            };
            slots.insert(
                stream.stream_name,
                StreamSlot {
                    video_track_tx,
                    audio_track_tx,
                },
            );
        }
        let stream_slots: Arc<StreamSlots> = Arc::new(Mutex::new(slots));

        let mut join_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();

        let expected_app = url.app.clone();

        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    tracing::debug!("New RTMP client connection from: {peer_addr}");
                    let tls_acceptor = tls_acceptor.clone();
                    let timestamp_offset = server_started_at.elapsed();
                    let stats = stats.clone();
                    let stream_slots = Arc::clone(&stream_slots);
                    let expected_app = expected_app.clone();

                    // 完了済みタスクを除去
                    join_handles.retain(|h| !h.is_finished());

                    let join_handle = tokio::spawn(async move {
                        match ServerTcpOrTlsStream::accept_with_tls(stream, tls_acceptor.as_ref())
                            .await
                        {
                            Ok(tls_stream) => {
                                if tls_acceptor.is_some() {
                                    tracing::debug!("TLS handshake successful with {peer_addr}");
                                }
                                let Ok(mut handler) = RtmpPublisherHandler::new(
                                    tls_stream,
                                    expected_app,
                                    stream_slots,
                                    timestamp_offset,
                                    stats,
                                )
                                .inspect_err(|e| {
                                    tracing::error!(
                                        "Failed to initialize RTMP publisher handler: {}",
                                        e.display()
                                    );
                                }) else {
                                    return;
                                };

                                if let Err(e) = handler.run().await {
                                    tracing::error!(
                                        "RTMP publisher handler error: {}",
                                        e.display()
                                    );
                                }
                                handler.finalize();
                                tracing::debug!("RTMP publisher disconnected: {peer_addr}");
                            }
                            Err(e) => {
                                tracing::error!("Connection setup failed with {peer_addr}: {e}");
                            }
                        }
                    });
                    join_handles.push(join_handle);
                }
                Err(e) => {
                    // リスナーエラー時は全タスクを停止
                    for h in &join_handles {
                        h.abort();
                    }
                    return Err(e.into());
                }
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

impl nojson::DisplayJson for RtmpInboundStream {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("streamName", &self.stream_name)?;
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

impl nojson::DisplayJson for RtmpInboundEndpoint {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("inputUrl", &self.input_url)?;
            f.member("streams", &self.streams)?;
            Ok(())
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for RtmpInboundStream {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let stream_name: String = value.to_member("streamName")?.required()?.try_into()?;
        let output_audio_track_id: Option<crate::TrackId> =
            value.to_member("outputAudioTrackId")?.try_into()?;
        let output_video_track_id: Option<crate::TrackId> =
            value.to_member("outputVideoTrackId")?.try_into()?;

        if output_audio_track_id.is_none() && output_video_track_id.is_none() {
            return Err(value.invalid("outputAudioTrackId or outputVideoTrackId is required"));
        }

        let trimmed = stream_name.trim();
        if trimmed.is_empty() {
            return Err(value
                .to_member("streamName")?
                .required()?
                .invalid("streamName must not be empty"));
        }

        Ok(Self {
            stream_name: trimmed.to_owned(),
            output_audio_track_id,
            output_video_track_id,
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for RtmpInboundEndpoint {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let input_url: String = value.to_member("inputUrl")?.required()?.try_into()?;
        let streams: Vec<RtmpInboundStream> = value.to_member("streams")?.required()?.try_into()?;

        if streams.is_empty() {
            return Err(value
                .to_member("streams")?
                .required()?
                .invalid("streams must not be empty"));
        }

        // streamName の重複チェック
        let mut seen = std::collections::HashSet::new();
        for stream in &streams {
            if !seen.insert(&stream.stream_name) {
                return Err(value
                    .to_member("streams")?
                    .required()?
                    .invalid(format!("duplicate streamName: {}", stream.stream_name)));
            }
        }

        // 各ストリームの URL バリデーション
        for stream in &streams {
            if let Err(e) =
                shiguredo_rtmp::RtmpUrl::parse_with_stream_name(&input_url, &stream.stream_name)
            {
                return Err(value
                    .to_member("inputUrl")?
                    .required()?
                    .invalid(e.to_string()));
            }
        }

        Ok(Self {
            input_url,
            streams,
            options: RtmpInboundEndpointOptions::default(),
        })
    }
}

/// 個別のクライアント（パブリッシャー）接続を処理する
struct RtmpPublisherHandler {
    stream: ServerTcpOrTlsStream,
    connection: shiguredo_rtmp::RtmpServerConnection,
    recv_buf: Vec<u8>,
    expected_app: String,
    stream_slots: Arc<StreamSlots>,
    active_stream_name: Option<String>,
    frame_handler: crate::rtmp::frame::RtmpIncomingFrameHandler,
    video_track_tx: Option<crate::MessageSender>,
    audio_track_tx: Option<crate::MessageSender>,
    stats: RtmpInboundEndpointStats,
}

impl RtmpPublisherHandler {
    fn new(
        stream: ServerTcpOrTlsStream,
        expected_app: String,
        stream_slots: Arc<StreamSlots>,
        timestamp_offset: std::time::Duration,
        stats: RtmpInboundEndpointStats,
    ) -> crate::Result<Self> {
        Ok(Self {
            stream,
            connection: shiguredo_rtmp::RtmpServerConnection::new(),
            recv_buf: vec![0u8; 4096],
            expected_app,
            stream_slots,
            active_stream_name: None,
            frame_handler: crate::rtmp::frame::RtmpIncomingFrameHandler::new(timestamp_offset)?,
            video_track_tx: None,
            audio_track_tx: None,
            stats,
        })
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
        Ok(())
    }

    /// トラック送信者を StreamSlot に返却する
    fn finalize(&mut self) {
        if let Some(stream_name) = self.active_stream_name.take() {
            let mut slots = self.stream_slots.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(slot) = slots.get_mut(&stream_name) {
                slot.video_track_tx = self.video_track_tx.take();
                slot.audio_track_tx = self.audio_track_tx.take();
            }
        }
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
                if app != &self.expected_app {
                    self.connection.reject(&format!(
                        "Unknown app: {app}. Expected: {}",
                        self.expected_app
                    ))?;
                    tracing::warn!(
                        "Client requested unknown app: {app}, expected: {}",
                        self.expected_app
                    );
                    return Ok(());
                }

                let mut slots = self.stream_slots.lock().unwrap_or_else(|e| e.into_inner());
                match slots.get_mut(stream_name.as_str()) {
                    Some(slot) => {
                        // トラック送信者が利用可能か確認（他の配信者が使用中でないか）
                        if slot.video_track_tx.is_none() && slot.audio_track_tx.is_none() {
                            self.connection
                                .reject(&format!("Stream already in use: {app}/{stream_name}"))?;
                            tracing::warn!("Stream already in use: {app}/{stream_name}");
                        } else {
                            self.video_track_tx = slot.video_track_tx.take();
                            self.audio_track_tx = slot.audio_track_tx.take();
                            self.active_stream_name = Some(stream_name.clone());
                            self.connection.accept()?;
                            tracing::debug!(
                                "Client started publishing stream: {app}/{stream_name}"
                            );
                        }
                    }
                    None => {
                        self.connection
                            .reject(&format!("Stream not found: {app}/{stream_name}"))?;
                        tracing::warn!("Client requested unknown stream: {app}/{stream_name}");
                    }
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
        if let Some(audio_data) = self.frame_handler.process_audio_frame(frame)?
            && let Some(tx) = &mut self.audio_track_tx
        {
            if let Some(codec) = audio_data.format.codec_name() {
                self.stats.set_audio_codec(codec);
            }
            self.stats.add_input_audio_data_count();
            self.stats
                .set_last_input_audio_timestamp(audio_data.timestamp);
            tx.send_media(crate::MediaFrame::Audio(std::sync::Arc::new(audio_data)));
        }
        Ok(())
    }

    /// ビデオフレームを処理する
    async fn handle_video_frame(&mut self, frame: shiguredo_rtmp::VideoFrame) -> crate::Result<()> {
        if let Some(video_frame) = self.frame_handler.process_video_frame(frame)?
            && let Some(tx) = &mut self.video_track_tx
        {
            if let Some(codec) = video_frame.format.codec_name() {
                self.stats.set_video_codec(codec);
            }
            self.stats.add_input_video_frame_count();
            self.stats
                .set_last_input_video_timestamp(video_frame.timestamp);
            tx.send_media(crate::MediaFrame::Video(std::sync::Arc::new(video_frame)));
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
