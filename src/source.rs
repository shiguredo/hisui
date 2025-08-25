use orfail::OrFail;

use crate::{
    audio::AudioData,
    channel::{self, ErrorFlag},
    decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions},
    layout::AggregatedSourceInfo,
    media::MediaStreamIdGenerator,
    processor::{MediaProcessor, MediaProcessorOutput},
    reader::{AudioReader, VideoReader},
    stats::SharedStats,
    video::VideoFrame,
};

#[derive(Debug)]
pub struct AudioSourceThread {
    reader: AudioReader,
    decoder: AudioDecoder,
    tx: channel::SyncSender<AudioData>,
}

impl AudioSourceThread {
    pub fn start(
        error_flag: ErrorFlag,
        source_info: &AggregatedSourceInfo,
        stream_id_gen: &mut MediaStreamIdGenerator,
        stats: SharedStats,
    ) -> orfail::Result<channel::Receiver<AudioData>> {
        let read_stream_id = stream_id_gen.next_id();
        let decoded_stream_id = stream_id_gen.next_id();

        // 音声入力は Opus 前提
        let decoder = AudioDecoder::new_opus(read_stream_id, decoded_stream_id).or_fail()?;

        let mut input_files = source_info
            .media_paths
            .iter()
            .map(|(path, source)| (source.start_timestamp, path.clone()))
            .collect::<Vec<_>>();
        input_files.sort();
        let reader = AudioReader::new(
            read_stream_id,
            source_info.id.clone(),
            source_info.format,
            source_info.start_timestamp,
            input_files.into_iter().map(|(_, path)| path).collect(),
        )
        .or_fail()?;

        let (tx, rx) = channel::sync_channel();
        let mut this = Self {
            reader,
            decoder,
            tx,
        };
        std::thread::spawn(move || {
            if let Err(e) = this.run().or_fail() {
                error_flag.set();
                this.reader.spec().stats.set_error();
                this.decoder.set_error();
                log::error!("failed to load audio source: {e}");
            }

            stats.with_lock(|stats| {
                stats.processors.push(this.reader.spec().stats);
                stats.processors.push(this.decoder.spec().stats);
            });
        });
        Ok(rx)
    }

    fn run(&mut self) -> orfail::Result<()> {
        while let MediaProcessorOutput::Processed { sample, .. } =
            self.reader.process_output().or_fail()?
        {
            let data = sample.expect_audio_data().or_fail()?;
            let decoded = self.decoder.decode(data).or_fail()?;
            if !self.tx.send(decoded) {
                // 受信側がすでに閉じている場合にはこれ以上処理しても仕方がないので終了する
                log::info!("receiver of audio source has been closed");
                return Ok(());
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct VideoSourceThread {
    reader: VideoReader,
    decoder: VideoDecoder,
    tx: channel::SyncSender<VideoFrame>,
}

impl VideoSourceThread {
    pub fn start(
        error_flag: ErrorFlag,
        source_info: &AggregatedSourceInfo,
        options: VideoDecoderOptions,
        stream_id_gen: &mut MediaStreamIdGenerator,
        stats: SharedStats,
    ) -> orfail::Result<channel::Receiver<VideoFrame>> {
        let read_stream_id = stream_id_gen.next_id();
        let decoded_stream_id = stream_id_gen.next_id();
        let decoder = VideoDecoder::new(read_stream_id, decoded_stream_id, options);

        let mut input_files = source_info
            .media_paths
            .iter()
            .map(|(path, source)| (source.start_timestamp, path.clone()))
            .collect::<Vec<_>>();
        input_files.sort();
        let reader = VideoReader::new(
            read_stream_id,
            source_info.id.clone(),
            source_info.format,
            source_info.start_timestamp,
            input_files.into_iter().map(|(_, path)| path).collect(),
        )
        .or_fail()?;

        let (tx, rx) = channel::sync_channel();
        let mut this = Self {
            reader,
            decoder,
            tx,
        };
        std::thread::spawn(move || {
            if let Err(e) = this.run().or_fail() {
                error_flag.set();
                this.reader.spec().stats.set_error();
                this.decoder.set_error();
                log::error!("failed to load video source: {e}");
            }

            stats.with_lock(|stats| {
                stats.processors.push(this.reader.spec().stats);
                stats.processors.push(this.decoder.spec().stats);
            });
        });
        Ok(rx)
    }

    fn run(&mut self) -> orfail::Result<()> {
        while let MediaProcessorOutput::Processed { sample, .. } =
            self.reader.process_output().or_fail()?
        {
            let frame = sample.expect_video_frame().or_fail()?;
            self.decoder.decode(frame).or_fail()?;

            while let Some(decoded_frame) = self.decoder.next_decoded_frame() {
                if !self.tx.send(decoded_frame) {
                    // 受信側がすでに閉じている場合にはこれ以上処理しても仕方がないので終了する
                    log::info!("receiver of video source has been closed");
                    return Ok(());
                }
            }
        }

        // Finish decoding any remaining frames
        self.decoder.finish().or_fail()?;
        while let Some(decoded_frame) = self.decoder.next_decoded_frame() {
            if !self.tx.send(decoded_frame) {
                log::info!("receiver of video source has been closed");
                return Ok(());
            }
        }

        Ok(())
    }
}
