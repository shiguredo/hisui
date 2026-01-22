// TODO: マージ前に削除する
#![expect(dead_code)]
#![expect(clippy::too_many_arguments)]

use orfail::OrFail;

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

#[derive(Debug, Default, Clone)]
pub struct RtmpPublisherOptions {
    pub tls: bool,
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
        server_host: String,
        server_port: u16,
        app: String,
        stream_name: String,
        options: RtmpPublisherOptions,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(100); // TODO: サイズは変更できるようにする
        runtime.spawn(async move {
            let runner = RtmpPublishRunner {
                server_host,
                server_port,
                app,
                stream_name,
                options,
                rx,
            };
            runner.run().await;
            todo!()
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
                // 映像は H.264 （AVC 形式） のみ許可する
                (sample.format == VideoFormat::H264)
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
    server_host: String,
    server_port: u16,
    app: String,
    stream_name: String,
    options: RtmpPublisherOptions,
    rx: tokio::sync::mpsc::Receiver<MediaSample>,
}

impl RtmpPublishRunner {
    async fn run(self) {
        log::debug!(
            "Starting RTMP publisher: {}://{}:{}/{}/{}",
            if self.options.tls { "rtmps" } else { "rtmp" },
            self.server_host,
            self.server_port,
            self.app,
            self.stream_name
        );
    }
}
