// TODO: マージ前に削除する
#![expect(clippy::too_many_arguments)]

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
pub struct RtmpStreamEndpoint {
    pub host: String,
    pub port: u16,
    pub app: String,
    pub stream_name: String,
    pub tls: bool,
}

impl RtmpStreamEndpoint {
    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn tc_url(&self) -> String {
        let scheme = if self.tls { "rtmps" } else { "rtmp" };
        format!("{}://{}:{}/{}", scheme, self.host, self.port, self.app)
    }
}

#[derive(Debug)]
pub struct RtmpPublisher {
    input_audio_stream_id: Option<MediaStreamId>,
    input_video_stream_id: Option<MediaStreamId>,
    tx: tokio::sync::mpsc::Sender<MediaSample>,
}

impl RtmpPublisher {
    pub fn start(
        runtime: &tokio::runtime::Runtime,
        input_audio_stream_id: Option<MediaStreamId>,
        input_video_stream_id: Option<MediaStreamId>,
        endpoint: RtmpStreamEndpoint,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(100); // TODO: サイズは変更できるようにする
        runtime.spawn(async move {
            let runner = RtmpPublishRunner { endpoint, rx };
            if let Err(e) = runner.run().await.or_fail() {
                log::error!("RTMP publish error: {e}");
                // TODO: stats 更新
            }
        });
        Self {
            input_audio_stream_id,
            input_video_stream_id,
            tx,
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

                // TODO: ちゃんとエラーハンドリングする（一時的に詰まっているだけならエラーにしない）
                self.tx.try_send(MediaSample::Audio(sample)).or_fail()?;
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

                // TODO: ちゃんとエラーハンドリングする（一時的に詰まっているだけならエラーにしない）
                self.tx.try_send(MediaSample::Video(sample)).or_fail()?;
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
            Ok(MediaProcessorOutput::Finished)
        }
    }
}

#[derive(Debug)]
struct RtmpPublishRunner {
    endpoint: RtmpStreamEndpoint,
    rx: tokio::sync::mpsc::Receiver<MediaSample>,
}

impl RtmpPublishRunner {
    async fn run(mut self) -> orfail::Result<()> {
        let tc_url = self.endpoint.tc_url();
        log::debug!("Starting RTMP publisher: {tc_url}");

        // TCP または TLS 接続を確立
        let mut stream = if self.endpoint.tls {
            crate::tcp::TcpOrTlsStream::connect_tls(self.endpoint.addr(), &self.endpoint.host)
                .await
                .or_fail()?
        } else {
            crate::tcp::TcpOrTlsStream::connect_tcp(self.endpoint.addr())
                .await
                .or_fail()?
        };

        // RTMP パブリッシュクライアント接続を作成
        let mut connection =
            shiguredo_rtmp::RtmpPublishClientConnection::new(&tc_url, &self.endpoint.stream_name);
        let mut recv_buf = vec![0u8; 8192];

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

            // ストリームからデータを受信 (タイムアウト付き)
            match tokio::time::timeout(
                std::time::Duration::from_millis(5),
                stream.read(&mut recv_buf),
            )
            .await
            {
                Ok(Ok(0)) => break, // 接続が切断された
                Ok(Ok(n)) => connection.feed_recv_buf(&recv_buf[..n]).or_fail()?,
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionReset => break,
                Ok(Err(e)) => Err(e).or_fail()?,
                Err(_) => {} // タイムアウト
            }

            // メディアサンプルを送信
            if let Ok(sample) = self.rx.try_recv() {
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
                        connection.send_audio(frame).or_fail()?;
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
                        connection.send_video(frame).or_fail()?;
                    }
                }
            } else {
                // チャンネルが閉じている可能性をチェック
                if self.rx.is_closed() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        }

        log::debug!("RTMP publisher finished");
        Ok(())
    }
}
