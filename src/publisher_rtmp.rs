use orfail::OrFail;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{
    audio::AudioFormat,
    media::{MediaSample, MediaStreamId},
    processor::{
        MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
        MediaProcessorWorkloadHint,
    },
    stats::ProcessorStats,
    video::VideoFormat,
};

#[derive(Debug, Clone)]
pub struct RtmpStreamUrl {
    pub host: String,
    pub port: u16,
    pub app: String,
    pub stream_name: String,
    pub tls: bool,
}

impl std::fmt::Display for RtmpStreamUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let scheme = if self.tls { "rtmps" } else { "rtmp" };
        write!(f, "{}://{}:{}/{}", scheme, self.host, self.port, self.app)
    }
}

#[derive(Debug)]
pub struct RtmpPublisher {
    input_audio_stream_id: Option<MediaStreamId>,
    input_video_stream_id: Option<MediaStreamId>,
    tx: Option<tokio::sync::mpsc::Sender<MediaSample>>,
}

impl RtmpPublisher {
    pub fn start(
        runtime: &tokio::runtime::Runtime,
        input_audio_stream_id: Option<MediaStreamId>,
        input_video_stream_id: Option<MediaStreamId>,
        url: RtmpStreamUrl,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(100); // TODO: サイズは変更できるようにする
        runtime.spawn(async move {
            let connection = shiguredo_rtmp::RtmpPublishClientConnection::new(
                &url.to_string(),
                &url.stream_name,
            );
            let runner = RtmpPublishRunner {
                url,
                rx,
                recv_buf: vec![0u8; 8192],
                connection,
            };
            if let Err(e) = runner.run().await.or_fail() {
                log::error!("RTMP publish error: {e}");
                // TODO: stats 更新
            }
        });
        Self {
            input_audio_stream_id,
            input_video_stream_id,
            tx: Some(tx),
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
            stats: ProcessorStats::other("rtmp-publisher"), // TODO: 専用の構造体を用意する
            workload_hint: MediaProcessorWorkloadHint::WRITER, // TODO: 非同期 I/O 用の値を追加する
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

                // TODO: ちゃんとエラーハンドリングする（一時的に詰まっているだけならエラーにしない）
                tx.try_send(MediaSample::Audio(sample)).or_fail()?;
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

                // TODO: ちゃんとエラーハンドリングする（一時的に詰まっているだけならエラーにしない）
                tx.try_send(MediaSample::Video(sample)).or_fail()?;
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

        if self.input_audio_stream_id.is_none() && self.input_video_stream_id.is_none() {
            Ok(MediaProcessorOutput::awaiting_any())
        } else {
            self.tx = None;
            Ok(MediaProcessorOutput::Finished)
        }
    }
}

#[derive(Debug)]
struct RtmpPublishRunner {
    url: RtmpStreamUrl,
    rx: tokio::sync::mpsc::Receiver<MediaSample>,
    recv_buf: Vec<u8>,
    connection: shiguredo_rtmp::RtmpPublishClientConnection,
}

impl RtmpPublishRunner {
    async fn run(mut self) -> orfail::Result<()> {
        let tc_url = self.url.to_string();
        log::debug!("Starting RTMP publisher: {tc_url}");

        // TCP または TLS 接続を確立
        let mut stream =
            crate::tcp::TcpOrTlsStream::connect(&self.url.host, self.url.port, self.url.tls)
                .await
                .or_fail()?;

        // RTMP パブリッシュクライアント接続を作成
        let mut connection =
            shiguredo_rtmp::RtmpPublishClientConnection::new(&tc_url, &self.url.stream_name);

        // イベント処理ループ
        loop {
            // イベント処理
            while let Some(event) = connection.next_event() {
                log::debug!("RTMP event: {:?}", event);
            }

            // 送信バッファをストリームに書き込む
            let send_data = connection.send_buf();
            if !send_data.is_empty() {
                stream.write_all(send_data).await.or_fail()?;
                connection.advance_send_buf(send_data.len());
            }

            // select! マクロでストリーム受信とメディアサンプル受信を並行処理
            tokio::select! {
                result = stream.read(&mut self.recv_buf) => {
                    // TCP / TLS ストリームからデータを受信
                    self.handle_read_result(result).or_fail()?;
                }
                Some(sample) = self.rx.recv() => {
                    // サーバーに配信すべき入力メディアサンプルを受信
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

        self.connection
            .feed_recv_buf(&self.recv_buf[..n])
            .or_fail()?;
        Ok(())
    }

    fn handle_media_sample(&mut self, sample: crate::media::MediaSample) -> orfail::Result<()> {
        match sample {
            crate::media::MediaSample::Audio(audio) => {
                let frame = shiguredo_rtmp::AudioFrame {
                    timestamp: shiguredo_rtmp::RtmpTimestamp::from_millis(
                        audio.timestamp.as_millis() as u32,
                    ),
                    format: shiguredo_rtmp::AudioFormat::Aac,
                    sample_rate: shiguredo_rtmp::AudioSampleRate::Khz44,
                    is_8bit_sample: false,
                    is_stereo: true,
                    is_aac_sequence_header: false,
                    data: audio.data.clone(),
                };
                self.connection.send_audio(frame).or_fail()?;
            }
            crate::media::MediaSample::Video(video) => {
                let frame = shiguredo_rtmp::VideoFrame {
                    timestamp: shiguredo_rtmp::RtmpTimestamp::from_millis(
                        video.timestamp.as_millis() as u32,
                    ),
                    composition_timestamp_offset: shiguredo_rtmp::RtmpTimestampDelta::ZERO,
                    frame_type: if video.keyframe {
                        shiguredo_rtmp::VideoFrameType::KeyFrame
                    } else {
                        shiguredo_rtmp::VideoFrameType::InterFrame
                    },
                    codec: shiguredo_rtmp::VideoCodec::Avc,
                    avc_packet_type: Some(shiguredo_rtmp::AvcPacketType::NalUnit),
                    data: video.data.clone(),
                };
                self.connection.send_video(frame).or_fail()?;
            }
        }
        Ok(())
    }
}
