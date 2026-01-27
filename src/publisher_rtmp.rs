use std::sync::Arc;

use orfail::OrFail;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

#[derive(Debug)]
pub struct RtmpPublisher {
    input_audio_stream_id: Option<MediaStreamId>,
    input_video_stream_id: Option<MediaStreamId>,
    tx: Option<tokio::sync::mpsc::Sender<MediaSample>>,
    stats: RtmpPublisherStats,
}

impl RtmpPublisher {
    pub fn start(
        runtime: &tokio::runtime::Runtime,
        input_audio_stream_id: Option<MediaStreamId>,
        input_video_stream_id: Option<MediaStreamId>,
        url: shiguredo_rtmp::RtmpUrl,
        options: RtmpPublisherOptions,
    ) -> Self {
        let stats = RtmpPublisherStats::default();
        let (tx, rx) = tokio::sync::mpsc::channel(options.max_buffered_frame_count);

        let connection = shiguredo_rtmp::RtmpPublishClientConnection::new(url.clone());

        // Frame handler stats を作成
        let frame_handler_stats = crate::rtmp::RtmpOutgoingFrameHandlerStats {
            total_audio_frame_count: stats.total_audio_frame_count.clone(),
            total_video_frame_count: stats.total_video_frame_count.clone(),
            total_video_keyframe_count: stats.total_video_keyframe_count.clone(),
            total_audio_sequence_header_count: stats.total_audio_sequence_header_count.clone(),
            total_video_sequence_header_count: stats.total_video_sequence_header_count.clone(),
        };

        let mut runner = RtmpPublishRunner {
            url,
            rx,
            recv_buf: vec![0u8; 4096],
            connection,
            ready: false,
            frame_handler: crate::rtmp::RtmpOutgoingFrameHandler::new(frame_handler_stats),
            stats: stats.clone(),
        };
        runtime.spawn(async move {
            if let Err(e) = runner.run().await.or_fail() {
                log::error!("RTMP publish error: {e}");
                runner.stats.error.set(true);
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

impl MediaProcessor for RtmpPublisher {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: self
                .input_audio_stream_id
                .into_iter()
                .chain(self.input_video_stream_id)
                .collect(),
            output_stream_ids: Vec::new(),
            stats: ProcessorStats::RtmpPublisher(self.stats.clone()),
            workload_hint: MediaProcessorWorkloadHint::ASYNC_IO,
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        match input.sample {
            Some(MediaSample::Audio(sample))
                if Some(input.stream_id) == self.input_audio_stream_id =>
            {
                // 音声は AAC のみ許可する
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
                // 映像は H.264 （AVC or Annex.B 形式） のみ許可する
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
        // TODO: ネットワークが詰まっている場合には、それを前段にフィードバックする

        if self.input_audio_stream_id.is_some() || self.input_video_stream_id.is_some() {
            Ok(MediaProcessorOutput::awaiting_any())
        } else {
            self.tx = None;
            Ok(MediaProcessorOutput::Finished)
        }
    }
}

#[derive(Debug)]
struct RtmpPublishRunner {
    url: shiguredo_rtmp::RtmpUrl,
    rx: tokio::sync::mpsc::Receiver<MediaSample>,
    recv_buf: Vec<u8>,
    connection: shiguredo_rtmp::RtmpPublishClientConnection,
    ready: bool,
    frame_handler: crate::rtmp::RtmpOutgoingFrameHandler,
    stats: RtmpPublisherStats,
}

impl RtmpPublishRunner {
    async fn run(&mut self) -> orfail::Result<()> {
        log::debug!("Starting RTMP publisher: {}", self.url);

        // TCP または TLS 接続を確立
        let mut stream =
            crate::tcp::TcpOrTlsStream::connect(&self.url.host, self.url.port, self.url.tls)
                .await
                .or_fail()?;

        // イベント処理ループ
        loop {
            // イベント処理
            while let Some(event) = self.connection.next_event() {
                log::debug!("RTMP event: {:?}", event);
                self.stats.total_event_count.increment();
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
                stream.write_all(send_data).await.or_fail()?;
                self.stats.total_sent_bytes.add(send_data.len() as u64);
                self.connection.advance_send_buf(send_data.len());
            }

            // select! マクロでストリーム受信とメディアサンプル受信を並行処理
            tokio::select! {
                result = stream.read(&mut self.recv_buf) => {
                    // TCP / TLS ストリームからデータを受信
                    self.handle_read_result(result).or_fail()?;
                }
                Some(sample) = self.rx.recv(), if self.ready => {
                    // RTMP サーバーとの接続処理が完了したら、メディアサンプルを受信処理
                    self.handle_media_sample(sample).or_fail()?;
                }
                else => {
                    // 配信すべき入力サンプルがなくなった（正常終了）
                    break;
                }
            }
        }

        log::debug!("RTMP publisher finished");
        Ok(())
    }

    fn handle_read_result(&mut self, result: std::io::Result<usize>) -> orfail::Result<()> {
        let n = result.or_fail()?;

        // サーバーから切断されるのは想定外なのでエラー扱いにする
        (n > 0).or_fail_with(|()| "connection reset by server".to_owned())?;

        self.stats.total_received_bytes.add(n as u64);
        self.connection
            .feed_recv_buf(&self.recv_buf[..n])
            .or_fail()?;
        Ok(())
    }

    fn handle_media_sample(&mut self, sample: MediaSample) -> orfail::Result<()> {
        match sample {
            MediaSample::Audio(audio) => self.handle_audio_sample(audio),
            MediaSample::Video(video) => self.handle_video_sample(video),
        }
    }

    fn handle_audio_sample(&mut self, audio: Arc<AudioData>) -> orfail::Result<()> {
        let (seq_frame, audio_frame) = self.frame_handler.prepare_audio_frame(audio)?;
        if let Some(seq) = seq_frame {
            self.connection.send_audio(seq).or_fail()?;
        }
        self.connection.send_audio(audio_frame).or_fail()?;
        Ok(())
    }

    fn handle_video_sample(&mut self, video: Arc<VideoFrame>) -> orfail::Result<()> {
        if let Some((seq_frame, video_frame)) = self.frame_handler.prepare_video_frame(video)? {
            if let Some(seq) = seq_frame {
                self.connection.send_video(seq).or_fail()?;
            }
            self.connection.send_video(video_frame).or_fail()?;
        }
        Ok(())
    }
}

/// [`RtmpPublisher`] 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct RtmpPublisherStats {
    /// 配信した音声フレームの数
    pub total_audio_frame_count: SharedAtomicCounter,

    /// 配信した映像フレームの数
    pub total_video_frame_count: SharedAtomicCounter,

    /// RTMP イベント処理の回数
    pub total_event_count: SharedAtomicCounter,

    /// RTMP で送信したバイト数
    pub total_sent_bytes: SharedAtomicCounter,

    /// RTMP で受信したバイト数
    pub total_received_bytes: SharedAtomicCounter,

    /// 配信したキーフレーム（映像）の数
    pub total_video_keyframe_count: SharedAtomicCounter,

    /// 送信した音声シーケンスヘッダの数
    pub total_audio_sequence_header_count: SharedAtomicCounter,

    /// 送信した映像シーケンスヘッダの数
    pub total_video_sequence_header_count: SharedAtomicCounter,

    /// 処理に掛かった時間
    pub total_processing_duration: SharedAtomicDuration,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl nojson::DisplayJson for RtmpPublisherStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "rtmp_publisher")?;
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
