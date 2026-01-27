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
const FRAME_CHANNEL_SIZE: usize = 100;

#[derive(Debug, Default, Clone)]
pub struct RtmpOutboundEndpointOptions {}

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
        _options: RtmpOutboundEndpointOptions,
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
}

impl RtmpPlayServer {
    async fn run(&mut self) -> orfail::Result<()> {
        log::debug!(
            "Starting RTMP play server on {}:{}",
            self.url.host,
            self.url.port
        );

        let addr = format!("{}:{}", self.url.host, self.url.port);
        let listener = TcpListener::bind(&addr).await.or_fail()?;

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    let (stream, peer_addr) = accept_result.or_fail()?;
                    log::debug!("New RTMP client connection from: {}", peer_addr);

                    let (client_tx, client_rx) = tokio::sync::mpsc::channel(FRAME_CHANNEL_SIZE);
                    self.clients.push(client_tx);

                    let stats = self.stats.clone();
                    let expected_app = self.url.app.clone();
                    let expected_stream_name = self.url.stream_name.clone();

                    tokio::spawn(async move {
                        let frame_handler_stats = crate::rtmp::RtmpFrameHandlerStats {
                            total_audio_frame_count: stats.total_audio_frame_count.clone(),
                            total_video_frame_count: stats.total_video_frame_count.clone(),
                            total_video_keyframe_count: stats.total_video_keyframe_count.clone(),
                            total_audio_sequence_header_count: stats.total_audio_sequence_header_count.clone(),
                            total_video_sequence_header_count: stats.total_video_sequence_header_count.clone(),
                        };

                        let mut handler = RtmpClientHandler {
                            stream,
                            connection: shiguredo_rtmp::RtmpServerConnection::new(),
                            rx: client_rx,
                            recv_buf: vec![0u8; 4096],
                            received_keyframe: false,
                            stats,
                            expected_app,
                            expected_stream_name,
                            frame_handler: crate::rtmp::RtmpFrameHandler::new(4, frame_handler_stats),
                        };

                        if let Err(e) = handler.run().await.or_fail() {
                            log::error!("RTMP client handler error: {e}");
                        }
                        log::debug!("RTMP client disconnected: {}", peer_addr);
                    });
                }

                Some(sample) = self.rx.recv() => {
                    self.handle_media_sample(sample).await.or_fail()?;
                }
                else => {
                    break;
                }
            }
        }

        log::debug!("RTMP play server finished");
        Ok(())
    }

    /// メディアサンプルを受け取り、すべてのプレイヤーに配信する
    async fn handle_media_sample(&mut self, sample: MediaSample) -> orfail::Result<()> {
        let frame = match sample {
            MediaSample::Audio(audio) => ClientMediaFrame::Audio(audio),
            MediaSample::Video(video) => ClientMediaFrame::Video(video),
        };

        self.clients.retain(|tx| tx.try_send(frame.clone()).is_ok());
        Ok(())
    }
}

/// 個別のクライアント接続を処理する
#[derive(Debug)]
struct RtmpClientHandler {
    stream: TcpStream,
    connection: shiguredo_rtmp::RtmpServerConnection,
    rx: tokio::sync::mpsc::Receiver<ClientMediaFrame>,
    recv_buf: Vec<u8>,
    received_keyframe: bool,
    stats: RtmpOutboundEndpointStats,
    expected_app: String,
    expected_stream_name: String,
    frame_handler: crate::rtmp::RtmpFrameHandler,
}

impl RtmpClientHandler {
    async fn run(&mut self) -> orfail::Result<()> {
        loop {
            while let Some(event) = self.connection.next_event() {
                log::debug!("RTMP event: {:?}", event);
                self.stats.total_event_count.increment();
                self.handle_event(event).await.or_fail()?;
            }

            self.flush_send_buf().await.or_fail()?;

            tokio::select! {
                Some(frame) = self.rx.recv(), if self.connection.state() == shiguredo_rtmp::RtmpConnectionState::Playing => {
                    self.handle_client_media_frame(frame).or_fail()?;
                }

                read_result = self.stream.read(&mut self.recv_buf) => {
                    let n = read_result.or_fail()?;
                    if n == 0 {
                        break;
                    }

                    self.stats.total_received_bytes.add(n as u64);
                    self.connection.feed_recv_buf(&self.recv_buf[..n]).or_fail()?;
                }
            }
        }

        Ok(())
    }

    /// RTMP イベントを処理する
    async fn handle_event(
        &mut self,
        event: shiguredo_rtmp::RtmpConnectionEvent,
    ) -> orfail::Result<()> {
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
                // キーフレームを待っている場合
                if !self.received_keyframe && !video.keyframe {
                    return Ok(());
                }
                if !self.received_keyframe {
                    self.received_keyframe = true;
                }

                let (seq_frame, video_frame) = self.frame_handler.prepare_video_frame(video)?;

                if let Some(seq) = seq_frame {
                    self.connection.send_video(seq).or_fail()?;
                }
                self.connection.send_video(video_frame).or_fail()?;
            }
        }
        Ok(())
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
            f.member("error", self.error.get())?;
            Ok(())
        })
    }
}
