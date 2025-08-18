use std::{path::PathBuf, time::Duration};

use orfail::OrFail;

use crate::{
    audio::AudioData,
    channel::{self, ErrorFlag},
    decoder::{AudioDecoder, VideoDecoder},
    layout::AggregatedSourceInfo,
    media::{MediaStreamId, MediaStreamIdGenerator},
    metadata::{ContainerFormat, SourceId},
    reader::{AudioReader, VideoReader},
    reader_mp4::{Mp4AudioReader, Mp4VideoReader},
    reader_webm::{WebmAudioReader, WebmVideoReader},
    stats::{
        AudioDecoderStats, ProcessorStats, Seconds, SharedStats, VideoDecoderStats, VideoResolution,
    },
    types::{CodecName, EngineName},
    video::VideoFrame,
};

#[derive(Debug)]
pub struct AudioSourceThread {
    reader: AudioReader,
    decoder: AudioDecoder,
    decoder_stats: AudioDecoderStats,
    tx: channel::SyncSender<AudioData>,
    read_stream_id: MediaStreamId,
    start_timestamp: Duration,
    media_file_queue: MediaFileQueue,
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

        let mut media_file_queue = MediaFileQueue::new(source_info);
        let reader = media_file_queue
            .next_audio_reader(read_stream_id)
            .or_fail()?
            .or_fail()?;
        let start_timestamp = source_info.start_timestamp;

        let (tx, rx) = channel::sync_channel();
        let mut this = Self {
            reader,
            decoder,
            decoder_stats: AudioDecoderStats {
                engine: Some(EngineName::Opus),
                codec: Some(CodecName::Opus),
                ..Default::default()
            },
            tx,
            read_stream_id,
            start_timestamp,
            media_file_queue,
        };
        std::thread::spawn(move || {
            if let Err(e) = this.run(stats.clone()).or_fail() {
                error_flag.set();
                this.decoder_stats.error.set(true);
                log::error!("failed to load audio source: {e}");
            }

            stats.with_lock(|stats| {
                stats.processors.push(this.reader.stats());
                stats
                    .processors
                    .push(ProcessorStats::AudioDecoder(this.decoder_stats));
            });
        });
        Ok(rx)
    }

    fn run(&mut self, stats: SharedStats) -> orfail::Result<()> {
        loop {
            let mut next_timestamp = self.start_timestamp;
            while let Some(mut data) = self.reader.next().transpose().or_fail()? {
                // コンテナ自体の開始タイムスタンプを考慮する
                data.timestamp += self.start_timestamp;
                next_timestamp = data.timestamp + data.duration;

                let (decoded, elapsed) =
                    Seconds::try_elapsed(|| self.decoder.decode(&data).or_fail())?;
                self.decoder_stats.total_audio_data_count.add(1);
                self.decoder_stats.total_processing_seconds.add(elapsed);
                self.decoder_stats.source_id = data.source_id;
                if !self.tx.send(decoded) {
                    // 受信側がすでに閉じている場合にはこれ以上処理しても仕方がないので終了する
                    log::info!("receiver of audio source has been closed");
                    return Ok(());
                }
            }

            if let Some(reader) = self
                .media_file_queue
                .next_audio_reader(self.read_stream_id)
                .or_fail()?
            {
                // 次の分割録画ファイルがある
                stats.with_lock(|stats| {
                    stats.processors.push(self.reader.stats());
                });
                self.reader = reader;

                // 分割録画ファイルのタイムスタンプは連続している前提
                self.start_timestamp = next_timestamp;
            } else {
                return Ok(());
            }
        }
    }
}

#[derive(Debug)]
pub struct VideoSourceThread {
    reader: VideoReader,
    read_stream_id: MediaStreamId,
    tx: channel::SyncSender<VideoFrame>,
    start_timestamp: Duration,
    decoder: VideoDecoder,
    decoder_stats: VideoDecoderStats,
    media_file_queue: MediaFileQueue,
}

impl VideoSourceThread {
    pub fn start(
        error_flag: ErrorFlag,
        source_info: &AggregatedSourceInfo,
        decoder: VideoDecoder,
        stream_id_gen: &mut MediaStreamIdGenerator,
        stats: SharedStats,
    ) -> orfail::Result<channel::Receiver<VideoFrame>> {
        let start_timestamp = source_info.start_timestamp;

        let read_stream_id = stream_id_gen.next_id();
        let mut media_file_queue = MediaFileQueue::new(source_info);
        let reader = media_file_queue
            .next_video_reader(read_stream_id)
            .or_fail()?
            .or_fail()?;

        let (tx, rx) = channel::sync_channel();
        let mut this = Self {
            reader,
            read_stream_id,
            tx,
            start_timestamp,
            decoder,
            decoder_stats: VideoDecoderStats::default(),
            media_file_queue,
        };
        std::thread::spawn(move || {
            if let Err(e) = this.run(stats.clone()).or_fail() {
                error_flag.set();
                this.decoder_stats.error.set(true);
                log::error!("failed to load video source: {e}");
            }
            stats.with_lock(|stats| {
                stats.processors.push(this.reader.stats());
                stats
                    .processors
                    .push(ProcessorStats::VideoDecoder(this.decoder_stats));
            });
        });
        Ok(rx)
    }

    fn run(&mut self, stats: SharedStats) -> orfail::Result<()> {
        loop {
            let next_timestamp = self.run_one_reader().or_fail()?;

            if let Some(reader) = self
                .media_file_queue
                .next_video_reader(self.read_stream_id)
                .or_fail()?
            {
                // 次の分割録画ファイルがある
                stats.with_lock(|stats| {
                    stats.processors.push(self.reader.stats());
                });
                self.reader = reader;

                // 分割録画ファイルのタイムスタンプは連続している前提
                self.start_timestamp = next_timestamp;
            } else {
                return Ok(());
            }
        }
    }

    fn run_one_reader(&mut self) -> orfail::Result<Duration> {
        let mut next_timestamp = self.start_timestamp;
        loop {
            while let Some(frame) = self.decoder.next_decoded_frame() {
                self.decoder_stats.total_output_video_frame_count.add(1);
                self.decoder_stats
                    .resolutions
                    .insert(VideoResolution::new(&frame));
                if !self.tx.send(frame) {
                    // 受信側がすでに閉じている場合にはこれ以上処理しても仕方がないので終了する
                    log::info!("receiver of video source has been closed");
                    return Ok(next_timestamp);
                }
            }

            if let Some(mut frame) = self.reader.next().transpose().or_fail()? {
                // コンテナ自体の開始タイムスタンプを考慮する
                frame.timestamp += self.start_timestamp;
                next_timestamp = frame.timestamp + frame.duration;

                self.decoder_stats.total_input_video_frame_count.add(1);
                if let Some(id) = &frame.source_id
                    && self.decoder_stats.source_id.get().is_none()
                {
                    self.decoder_stats.source_id.set(id.clone());
                }

                let (_, elapsed) = Seconds::try_elapsed(|| {
                    self.decoder
                        .decode(frame, &mut self.decoder_stats)
                        .or_fail()
                })?;
                self.decoder_stats.total_processing_seconds.add(elapsed);
            } else {
                break;
            }
        }

        let (_, elapsed) = Seconds::try_elapsed(|| self.decoder.finish().or_fail())?;
        self.decoder_stats.total_processing_seconds.add(elapsed);

        while let Some(frame) = self.decoder.next_decoded_frame() {
            self.decoder_stats.total_output_video_frame_count.add(1);
            self.decoder_stats
                .resolutions
                .insert(VideoResolution::new(&frame));
            if !self.tx.send(frame) {
                // 受信側がすでに閉じている場合にはこれ以上処理しても仕方がないので終了する
                log::info!("receiver of video source has been closed");
                return Ok(next_timestamp);
            }
        }

        Ok(next_timestamp)
    }
}

// 読み込み対象のメディアファイルパスのキュー
// 分割録画ではない場合には、要素は常に一つとなる
#[derive(Debug)]
struct MediaFileQueue {
    source_id: SourceId,
    format: ContainerFormat,
    reverse_queue: Vec<MediaFileInfo>,
}

impl MediaFileQueue {
    fn new(info: &AggregatedSourceInfo) -> Self {
        let mut queue = info
            .media_paths
            .iter()
            .map(|(path, info)| MediaFileInfo {
                path: path.to_path_buf(),
                start_timestamp: info.start_timestamp,
            })
            .collect::<Vec<_>>();

        // 時刻順でソートする
        queue.sort_by_key(|x| x.start_timestamp);

        queue.reverse();
        Self {
            source_id: info.id.clone(),
            format: info.format,
            reverse_queue: queue,
        }
    }

    fn next_audio_reader(
        &mut self,
        read_stream_id: MediaStreamId,
    ) -> orfail::Result<Option<AudioReader>> {
        let Some(info) = self.reverse_queue.pop() else {
            return Ok(None);
        };

        let reader = if self.format == ContainerFormat::Webm {
            AudioReader::Webm(
                WebmAudioReader::new(self.source_id.clone(), read_stream_id, info.path)
                    .or_fail()?,
            )
        } else {
            AudioReader::Mp4(
                Mp4AudioReader::new(self.source_id.clone(), read_stream_id, info.path).or_fail()?,
            )
        };
        Ok(Some(reader))
    }

    fn next_video_reader(
        &mut self,
        read_stream_id: MediaStreamId,
    ) -> orfail::Result<Option<VideoReader>> {
        let Some(info) = self.reverse_queue.pop() else {
            return Ok(None);
        };

        let reader = if self.format == ContainerFormat::Webm {
            VideoReader::Webm(
                WebmVideoReader::new(self.source_id.clone(), read_stream_id, info.path)
                    .or_fail()?,
            )
        } else {
            VideoReader::Mp4(
                Mp4VideoReader::new(self.source_id.clone(), read_stream_id, info.path).or_fail()?,
            )
        };
        Ok(Some(reader))
    }
}

#[derive(Debug)]
struct MediaFileInfo {
    path: PathBuf,
    start_timestamp: Duration,
}
