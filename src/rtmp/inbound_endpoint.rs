use std::path::PathBuf;

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
///
/// フィールドの不変条件は `Self::new()` で eager 検証される。
pub struct RtmpInboundEndpoint {
    pub(crate) input_url: String,
    pub(crate) stream_name: Option<String>,
    pub(crate) output_audio_track_id: Option<crate::TrackId>,
    pub(crate) output_video_track_id: Option<crate::TrackId>,
    pub(crate) options: RtmpInboundEndpointOptions,
}

/// `RtmpInboundEndpoint::new()` が返す検証エラー。
#[derive(Debug)]
pub enum RtmpInboundEndpointBuildError {
    EmptyInputUrl,
    EmptyStreamName,
    NoTrackId,
}

impl std::fmt::Display for RtmpInboundEndpointBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyInputUrl => write!(f, "input_url must not be empty"),
            Self::EmptyStreamName => {
                write!(f, "stream_name must not be empty when specified")
            }
            Self::NoTrackId => write!(
                f,
                "at least one of output_audio_track_id / output_video_track_id must be set"
            ),
        }
    }
}

impl RtmpInboundEndpoint {
    /// `RtmpInboundEndpoint` を構築する。
    pub fn new(
        input_url: String,
        stream_name: Option<String>,
        output_audio_track_id: Option<crate::TrackId>,
        output_video_track_id: Option<crate::TrackId>,
        options: RtmpInboundEndpointOptions,
    ) -> Result<Self, RtmpInboundEndpointBuildError> {
        if input_url.is_empty() {
            return Err(RtmpInboundEndpointBuildError::EmptyInputUrl);
        }
        if let Some(name) = &stream_name
            && name.is_empty()
        {
            return Err(RtmpInboundEndpointBuildError::EmptyStreamName);
        }
        if output_audio_track_id.is_none() && output_video_track_id.is_none() {
            return Err(RtmpInboundEndpointBuildError::NoTrackId);
        }
        Ok(Self {
            input_url,
            stream_name,
            output_audio_track_id,
            output_video_track_id,
            options,
        })
    }
}

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
        let url = parse_rtmp_url(&self.input_url, self.stream_name.as_deref())
            .map_err(|e| crate::Error::new(format!("invalid input_url: {e}")))?;
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
        let endpoint_stats = RtmpInboundEndpointStats::new(handle.stats());
        endpoint_stats.set_listening(true);
        let server_started_at = tokio::time::Instant::now();

        // video decoder task を endpoint 寿命で保持し、 accept ループから input_tx を clone して
        // handler に渡す。 現状同期版が接続跨ぎで decoder を保持する挙動を踏襲するため、
        // publish_track で得た TrackPublisher を task 内に move する形にする。
        let video_decoder_task = if let Some(track_id) = &output_video_track_id {
            let output_tx = handle.publish_track(track_id.clone()).await?;
            let options = crate::decoder::VideoDecoderOptions {
                openh264_lib: handle.config().openh264_lib.clone(),
                ..Default::default()
            };
            Some(spawn_video_decoder_task(options, handle.stats(), output_tx))
        } else {
            None
        };

        let mut audio_track_tx = if let Some(track_id) = &output_audio_track_id {
            Some(handle.publish_track(track_id.clone()).await?)
        } else {
            None
        };

        let mut audio_decoder = if output_audio_track_id.is_some() {
            let mut decoder_stats = handle.stats();
            decoder_stats.set_default_label("component", "audio_decoder");
            Some(crate::decoder::AudioDecoder::new(
                #[cfg(feature = "fdk-aac")]
                handle.config().fdk_aac_lib.clone(),
                decoder_stats,
            )?)
        } else {
            None
        };

        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    tracing::debug!("New RTMP client connection from: {peer_addr}");
                    let expected_app = url.app.clone();
                    let expected_stream_name = url.stream_name.clone();
                    let tls_acceptor = tls_acceptor.clone();
                    let timestamp_offset = server_started_at.elapsed();
                    let endpoint_stats = endpoint_stats.clone();

                    match ServerTcpOrTlsStream::accept_with_tls(stream, tls_acceptor.as_ref()).await
                    {
                        Ok(tls_stream) => {
                            if tls_acceptor.is_some() {
                                tracing::debug!("TLS handshake successful with {peer_addr}");
                            }
                            let video_decoder_input_tx =
                                video_decoder_task.as_ref().map(|t| t.input_tx.clone());
                            let Ok(mut handler) = RtmpPublisherHandler::new(
                                tls_stream,
                                expected_app,
                                expected_stream_name,
                                timestamp_offset,
                                video_decoder_input_tx,
                                audio_track_tx.take(),
                                audio_decoder.take(),
                                endpoint_stats,
                            )
                            .inspect_err(|e| {
                                tracing::error!(
                                    "Failed to initialize RTMP publisher handler: {}",
                                    e.display()
                                );
                            }) else {
                                continue;
                            };

                            if let Err(e) = handler.run().await {
                                tracing::error!("RTMP publisher handler error: {}", e.display());
                            }
                            let RtmpPublisherHandlerAudioParts {
                                audio_track_tx: restored_audio_track_tx,
                                audio_decoder: restored_audio_decoder,
                            } = handler.into_parts();
                            audio_track_tx = restored_audio_track_tx;
                            audio_decoder = restored_audio_decoder;
                            tracing::debug!("RTMP publisher disconnected: {peer_addr}");
                        }
                        Err(e) => {
                            tracing::error!("Connection setup failed with {peer_addr}: {e}");
                        }
                    }
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
    frame_handler: crate::rtmp::frame::RtmpIncomingFrameHandler,
    video_decoder_input_tx: Option<tokio::sync::mpsc::UnboundedSender<DecoderInput>>,
    audio_track_tx: Option<crate::TrackPublisher>,
    audio_decoder: Option<crate::decoder::AudioDecoder>,
    stats: RtmpInboundEndpointStats,
}

/// `RtmpPublisherHandler::into_parts` が返す audio 側の回収要素。
/// tuple 順序ミスによる audio / video 誤配線を避けるため named struct にする。
struct RtmpPublisherHandlerAudioParts {
    audio_track_tx: Option<crate::TrackPublisher>,
    audio_decoder: Option<crate::decoder::AudioDecoder>,
}

impl RtmpPublisherHandler {
    #[expect(
        clippy::too_many_arguments,
        reason = "handler の内部生成関数であり、呼び出し元は run() の 1 箇所のみ"
    )]
    fn new(
        stream: ServerTcpOrTlsStream,
        expected_app: String,
        expected_stream_name: String,
        timestamp_offset: std::time::Duration,
        video_decoder_input_tx: Option<tokio::sync::mpsc::UnboundedSender<DecoderInput>>,
        audio_track_tx: Option<crate::TrackPublisher>,
        audio_decoder: Option<crate::decoder::AudioDecoder>,
        stats: RtmpInboundEndpointStats,
    ) -> crate::Result<Self> {
        Ok(Self {
            stream,
            connection: shiguredo_rtmp::RtmpServerConnection::new(),
            recv_buf: vec![0u8; 4096],
            expected_app,
            expected_stream_name,
            frame_handler: crate::rtmp::frame::RtmpIncomingFrameHandler::new(timestamp_offset)?,
            video_decoder_input_tx,
            audio_track_tx,
            audio_decoder,
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

    fn into_parts(self) -> RtmpPublisherHandlerAudioParts {
        RtmpPublisherHandlerAudioParts {
            audio_track_tx: self.audio_track_tx,
            audio_decoder: self.audio_decoder,
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
    ///
    /// エンコード済みフレームをデコードし、raw フレームを出力トラックに送信する。
    async fn handle_audio_frame(&mut self, frame: shiguredo_rtmp::AudioFrame) -> crate::Result<()> {
        if let Some(audio_data) = self.frame_handler.process_audio_frame(frame)?
            && let Some(decoder) = &mut self.audio_decoder
            && let Some(tx) = &mut self.audio_track_tx
        {
            if let Some(codec) = audio_data.format.codec_name() {
                self.stats.set_audio_codec(codec);
            }
            self.stats.add_input_audio_data_count();
            self.stats
                .set_last_input_audio_timestamp(audio_data.timestamp);
            decoder.handle_input_sample(Some(crate::MediaFrame::Audio(std::sync::Arc::new(
                audio_data,
            ))))?;
            // Finished は EOS 入力時にしか発生しないため、通常フレーム処理中は Pending のみ返る
            if crate::decoder::drain_audio_decoder_output(decoder, tx)?
                == crate::decoder::DrainResult::PipelineClosed
            {
                return Err(crate::Error::new("audio track pipeline closed"));
            }
        }
        Ok(())
    }

    /// ビデオフレームを処理する
    ///
    /// エンコード済みフレームを decoder task の入力チャネルに投入する。
    async fn handle_video_frame(&mut self, frame: shiguredo_rtmp::VideoFrame) -> crate::Result<()> {
        if let Some(video_frame) = self.frame_handler.process_video_frame(frame)?
            && let Some(tx) = self.video_decoder_input_tx.as_ref()
        {
            if let Some(codec) = video_frame.format.codec_name() {
                self.stats.set_video_codec(codec);
            }
            self.stats.add_input_video_frame_count();
            self.stats
                .set_last_input_video_timestamp(video_frame.timestamp);
            tx.send(DecoderInput::Media(crate::MediaFrame::new_video(
                video_frame,
            )))
            .map_err(|_| crate::Error::new("video decoder task terminated unexpectedly"))?;
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

// video decoder task の spawn pattern。

enum DecoderInput {
    Media(crate::MediaFrame),
    Eos,
}

// Drop trait と shutdown(self) を共存させるため join_handle を Option で保持し
// take() で move する。 直接 JoinHandle を持つと Drop 実装型の partial move が
// E0509 で禁止される。
#[derive(Debug)]
struct VideoDecoderTask {
    input_tx: tokio::sync::mpsc::UnboundedSender<DecoderInput>,
    join_handle: Option<tokio::task::JoinHandle<crate::Result<()>>>,
}

impl VideoDecoderTask {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "shutdown はテストからのみ呼ばれる (本番経路は Drop 経由 abort)"
        )
    )]
    async fn shutdown(mut self) -> crate::Result<()> {
        let _ = self.input_tx.send(DecoderInput::Eos);
        let handle = self
            .join_handle
            .take()
            .expect("join_handle is Some until shutdown/Drop consumes it");
        match handle.await {
            Ok(result) => result,
            Err(e) if e.is_panic() => {
                tracing::error!("video decoder task panicked: {e}");
                Err(crate::Error::new(format!(
                    "video decoder task panicked: {e}"
                )))
            }
            Err(e) => Err(crate::Error::new(format!(
                "video decoder task join failed: {e}"
            ))),
        }
    }
}

impl Drop for VideoDecoderTask {
    fn drop(&mut self) {
        // 早期 return / panic unwind 経路で task が leak しないよう abort する。
        // shutdown() が先に呼ばれていれば take 済みで None のため何もしない。
        if let Some(handle) = self.join_handle.take() {
            handle.abort();
        }
    }
}

fn spawn_video_decoder_task(
    options: crate::decoder::VideoDecoderOptions,
    mut stats: crate::stats::Stats,
    output_tx: crate::TrackPublisher,
) -> VideoDecoderTask {
    let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();
    stats.set_default_label("component", "video_decoder");
    let join_handle =
        tokio::spawn(async move { video_decoder_loop(options, stats, input_rx, output_tx).await });
    VideoDecoderTask {
        input_tx,
        join_handle: Some(join_handle),
    }
}

async fn video_decoder_loop(
    options: crate::decoder::VideoDecoderOptions,
    stats: crate::stats::Stats,
    mut input_rx: tokio::sync::mpsc::UnboundedReceiver<DecoderInput>,
    mut output_tx: crate::TrackPublisher,
) -> crate::Result<()> {
    let mut decoder = crate::decoder::AsyncVideoDecoder::new(options, stats);
    loop {
        let input = match input_rx.recv().await {
            Some(input) => input,
            // main が VideoDecoderTask を drop する経路 (通常は shutdown 経由で EOS 送信済み)。
            None => return Ok(()),
        };
        let is_eos = matches!(input, DecoderInput::Eos);
        match input {
            DecoderInput::Media(sample) => decoder.handle_input_sample_sync(Some(sample))?,
            DecoderInput::Eos => decoder.handle_input_sample_sync(None)?,
        }
        // 1 サンプル入力で 0 個以上のフレームが出力されうるため Pending / Finished まで drain する。
        loop {
            match decoder.poll_output_sync()? {
                crate::decoder::DecoderRunOutput::Processed(sample) => {
                    if !output_tx.send_media(sample) {
                        return Ok(());
                    }
                }
                crate::decoder::DecoderRunOutput::Pending => break,
                crate::decoder::DecoderRunOutput::Finished => {
                    let _ = output_tx.send_eos();
                    return Ok(());
                }
            }
        }
        // AsyncVideoDecoder::poll_output_sync が Empty + eos==true を Finished に射影するため到達不能。
        if is_eos {
            unreachable!("video decoder still pending after EOS");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// spawn 直後に shutdown().await が Ok(()) を返すことを検証する smoke test。
    #[tokio::test]
    async fn spawn_then_shutdown_returns_ok() -> crate::Result<()> {
        let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())?;
        let pipeline_handle = pipeline.handle();
        let _pipeline_task = tokio::spawn(async move { pipeline.run().await });

        let processor_handle = pipeline_handle
            .register_processor(
                crate::ProcessorId::new("rtmp_task_smoke_test"),
                crate::ProcessorMetadata::new("rtmp_task_smoke_test"),
            )
            .await
            .expect("register processor");
        let track_id = crate::TrackId::new("rtmp_task_smoke_test_video");
        let output_tx = processor_handle.publish_track(track_id).await?;

        let task = spawn_video_decoder_task(
            crate::decoder::VideoDecoderOptions::default(),
            processor_handle.stats(),
            output_tx,
        );

        task.shutdown().await
    }
}
