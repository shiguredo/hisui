use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{
    Error, MediaFrame, Message, ProcessorHandle, TrackId,
    audio::{AudioFormat, AudioFrame},
    video::{VideoFormat, VideoFrame},
};

#[derive(Debug, Clone)]
pub struct RtmpPublisherOptions {
    /// 未送信の音声・映像フレームを保持するバッファの最大フレーム数
    ///
    /// このサイズを超えてフレームが溜まった場合には、
    /// 出力先のネットワークないしサーバーが過負荷に陥っていると判断して、
    /// 接続を強制終了する（エラー扱い）
    ///
    /// デフォルト値は 1000
    pub max_buffered_frame_count: usize,
}

impl Default for RtmpPublisherOptions {
    fn default() -> Self {
        Self {
            max_buffered_frame_count: 1000, // FPS にもよるけど概ね 10 秒分くらい
        }
    }
}

#[derive(Debug, Clone)]
pub struct RtmpPublisher {
    pub output_url: String,
    pub stream_name: Option<String>,
    pub input_audio_track_id: Option<TrackId>,
    pub input_video_track_id: Option<TrackId>,
    pub options: RtmpPublisherOptions,
}

#[derive(Debug, Clone)]
struct RtmpPublisherStats {
    total_sent_bytes: crate::stats::StatsCounter,
    total_waiting_keyframe_dropped_video_frame_count: crate::stats::StatsCounter,
}

impl RtmpPublisherStats {
    fn new(stats: &mut crate::stats::Stats) -> Self {
        Self {
            total_sent_bytes: stats.counter("total_sent_bytes"),
            total_waiting_keyframe_dropped_video_frame_count: stats
                .counter("total_waiting_keyframe_dropped_video_frame_count"),
        }
    }

    fn add_sent_bytes(&self, value: usize) {
        self.total_sent_bytes.add(value as u64);
    }

    fn add_waiting_keyframe_dropped_video_frame(&self) {
        self.total_waiting_keyframe_dropped_video_frame_count.inc();
    }
}

impl nojson::DisplayJson for RtmpPublisher {
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
            if self.options.max_buffered_frame_count
                != RtmpPublisherOptions::default().max_buffered_frame_count
            {
                f.member(
                    "maxBufferedFrameCount",
                    self.options.max_buffered_frame_count,
                )?;
            }
            Ok(())
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for RtmpPublisher {
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
        let max_buffered_frame_count: Option<usize> =
            value.to_member("maxBufferedFrameCount")?.try_into()?;

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

        let max_buffered_frame_count = max_buffered_frame_count
            .unwrap_or(RtmpPublisherOptions::default().max_buffered_frame_count);
        if max_buffered_frame_count == 0 {
            return Err(value
                .to_member("maxBufferedFrameCount")?
                .required()?
                .invalid("maxBufferedFrameCount must be greater than 0"));
        }

        if let Err(e) = parse_rtmp_url(&output_url, stream_name.as_deref()) {
            return Err(value.to_member("outputUrl")?.required()?.invalid(e));
        }

        Ok(Self {
            output_url,
            stream_name,
            input_audio_track_id,
            input_video_track_id,
            options: RtmpPublisherOptions {
                max_buffered_frame_count,
            },
        })
    }
}

impl RtmpPublisher {
    pub async fn run(self, handle: ProcessorHandle) -> crate::Result<()> {
        let mut stats = handle.stats();
        let publisher_stats = RtmpPublisherStats::new(&mut stats);
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

        let (tx, rx) = tokio::sync::mpsc::channel(self.options.max_buffered_frame_count);

        let mut runner = RtmpPublishRunner {
            url,
            rx,
            recv_buf: vec![0u8; 4096],
            connection: shiguredo_rtmp::RtmpPublishClientConnection::new(
                parse_rtmp_url(&self.output_url, self.stream_name.as_deref())
                    .map_err(|e| Error::new(format!("invalid outputUrl: {e}")))?,
            ),
            ready: false,
            frame_handler: crate::rtmp::RtmpOutgoingFrameHandler::new(),
            stats: publisher_stats,
        };

        let runner_task = tokio::spawn(async move {
            if let Err(e) = runner.run().await {
                tracing::error!("RTMP publish error: {}", e.display());
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
        match runner_task.await {
            Ok(result) => result,
            Err(e) => Err(Error::new(format!("rtmp publisher task failed: {e}"))),
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
    tx: &tokio::sync::mpsc::Sender<MediaFrame>,
) -> crate::Result<bool> {
    match message {
        Message::Media(MediaFrame::Audio(sample)) => {
            if sample.format != AudioFormat::Aac {
                return Err(Error::new(format!(
                    "unsupported audio codec: {}",
                    sample.format
                )));
            }
            tx.try_send(MediaFrame::Audio(sample))
                .map_err(|e| Error::new(format!("failed to send audio frame: {e}")))?;
            Ok(false)
        }
        Message::Media(MediaFrame::Video(_)) => Err(Error::new(format!(
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
    tx: &tokio::sync::mpsc::Sender<MediaFrame>,
) -> crate::Result<bool> {
    match message {
        Message::Media(MediaFrame::Video(sample)) => {
            if !matches!(sample.format, VideoFormat::H264 | VideoFormat::H264AnnexB) {
                return Err(Error::new(format!(
                    "unsupported video codec: {}",
                    sample.format
                )));
            }
            tx.try_send(MediaFrame::Video(sample))
                .map_err(|e| Error::new(format!("failed to send video frame: {e}")))?;
            Ok(false)
        }
        Message::Media(MediaFrame::Audio(_)) => Err(Error::new(format!(
            "expected a video sample on track {}, but got an audio sample",
            track_id.as_ref().map(|id| id.get()).unwrap_or("<none>")
        ))),
        Message::Eos => Ok(true),
        Message::Syn(_) => Ok(false),
    }
}

#[derive(Debug)]
struct RtmpPublishRunner {
    url: shiguredo_rtmp::RtmpUrl,
    rx: tokio::sync::mpsc::Receiver<MediaFrame>,
    recv_buf: Vec<u8>,
    connection: shiguredo_rtmp::RtmpPublishClientConnection,
    ready: bool,
    frame_handler: crate::rtmp::RtmpOutgoingFrameHandler,
    stats: RtmpPublisherStats,
}

impl RtmpPublishRunner {
    async fn run(&mut self) -> crate::Result<()> {
        tracing::debug!("Starting RTMP publisher: {}", self.url);

        let mut stream =
            crate::tcp::TcpOrTlsStream::connect(&self.url.host, self.url.port, self.url.tls)
                .await?;

        loop {
            while let Some(event) = self.connection.next_event() {
                tracing::debug!("RTMP event: {:?}", event);
                if matches!(
                    event,
                    shiguredo_rtmp::RtmpConnectionEvent::StateChanged(
                        shiguredo_rtmp::RtmpConnectionState::Publishing
                    )
                ) {
                    self.ready = true;
                }
            }

            // 送信バッファをストリームに書き込む
            while !self.connection.send_buf().is_empty() {
                let send_data = self.connection.send_buf();
                stream.write_all(send_data).await?;
                self.stats.add_sent_bytes(send_data.len());
                self.connection.advance_send_buf(send_data.len());
            }

            tokio::select! {
                result = stream.read(&mut self.recv_buf) => {
                    // TCP / TLS ストリームからデータを受信
                    self.handle_read_result(result)?;
                }
                sample = self.rx.recv(), if self.ready => {
                    let Some(sample) = sample else {
                        // 配信すべき入力サンプルがなくなった（正常終了）
                        break;
                    };
                    // RTMP サーバーとの接続処理が完了したら、メディアサンプルを受信処理
                    self.handle_media_sample(sample)?;
                }
            }
        }

        tracing::debug!("RTMP publisher finished");
        Ok(())
    }

    fn handle_read_result(&mut self, result: std::io::Result<usize>) -> crate::Result<()> {
        let n = result?;

        // サーバーから切断されるのは想定外なのでエラー扱いにする
        if n == 0 {
            return Err(Error::new("connection reset by server"));
        }

        self.connection
            .feed_recv_buf(&self.recv_buf[..n])
            .map_err(|e| {
                Error::new(format!(
                    "failed to feed received bytes to RTMP connection: {e}"
                ))
            })?;
        Ok(())
    }

    fn handle_media_sample(&mut self, sample: MediaFrame) -> crate::Result<()> {
        match sample {
            MediaFrame::Audio(audio) => self.handle_audio_sample(audio),
            MediaFrame::Video(video) => self.handle_video_sample(video),
        }
    }

    fn handle_audio_sample(&mut self, audio: Arc<AudioFrame>) -> crate::Result<()> {
        let (seq_frame, audio_frame) = self
            .frame_handler
            .prepare_audio_frame(audio)
            .map_err(|e| e.with_context("failed to prepare audio frame"))?;
        if let Some(seq) = seq_frame {
            self.connection
                .send_audio(seq)
                .map_err(|e| Error::new(format!("failed to send audio sequence header: {e}")))?;
        }
        self.connection
            .send_audio(audio_frame)
            .map_err(|e| Error::new(format!("failed to send audio frame: {e}")))?;
        Ok(())
    }

    fn handle_video_sample(&mut self, video: Arc<VideoFrame>) -> crate::Result<()> {
        let waiting_for_keyframe = self.frame_handler.is_waiting_for_keyframe();
        if let Some((seq_frame, video_frame)) = self
            .frame_handler
            .prepare_video_frame(video)
            .map_err(|e| e.with_context("failed to prepare video frame"))?
        {
            if let Some(seq) = seq_frame {
                self.connection.send_video(seq).map_err(|e| {
                    Error::new(format!("failed to send video sequence header: {e}"))
                })?;
            }
            self.connection
                .send_video(video_frame)
                .map_err(|e| Error::new(format!("failed to send video frame: {e}")))?;
        } else if waiting_for_keyframe {
            self.stats.add_waiting_keyframe_dropped_video_frame();
        }
        Ok(())
    }
}
