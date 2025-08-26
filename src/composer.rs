use std::{
    collections::HashSet,
    num::NonZeroUsize,
    path::PathBuf,
    time::{Duration, Instant},
};

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{
    audio::AudioDataReceiver,
    channel::{self, ErrorFlag},
    decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions},
    encoder::{AudioEncoder, AudioEncoderThread, VideoEncoder, VideoEncoderThread},
    layout::Layout,
    media::{MediaSample, MediaStreamId, MediaStreamIdGenerator},
    mixer_audio::{AudioMixer, AudioMixerThread},
    mixer_video::{VideoMixer, VideoMixerThread},
    processor::{MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec},
    reader::{AudioReader, VideoReader},
    scheduler::Scheduler,
    source::{AudioSourceThread, VideoSourceThread},
    stats::{ProcessorStats, Seconds, SharedStats},
    video::VideoFrameReceiver,
    writer_mp4::Mp4Writer,
};

#[derive(Debug)]
pub struct Composer {
    pub layout: Layout,
    pub openh264_lib: Option<Openh264Library>,
    pub show_progress_bar: bool,
    pub max_cpu_cores: Option<NonZeroUsize>,
    pub stats_file_path: Option<PathBuf>,
}

#[derive(Debug)]
pub struct ComposeResult {
    pub stats: SharedStats,
    pub success: bool,
}

impl Composer {
    pub fn new(layout: Layout) -> Self {
        Self {
            layout,
            openh264_lib: None,
            show_progress_bar: false,
            max_cpu_cores: None,
            stats_file_path: None,
        }
    }

    pub fn compose(&self, out_file_path: &std::path::Path) -> orfail::Result<ComposeResult> {
        // 利用する CPU コア数を制限する
        crate::arg_utils::maybe_limit_cpu_cores(self.max_cpu_cores).or_fail()?;

        // プロセッサを準備
        let mut scheduler = Scheduler::new();
        let mut next_stream_id = MediaStreamId::new(0);

        // リーダーとデコーダーを登録
        let mut audio_mixer_input_stream_ids = Vec::new();
        for source_id in self.layout.audio_source_ids() {
            let source_info = self.layout.sources.get(source_id).or_fail()?;
            let reader_output_stream_id = next_stream_id.fetch_add(1);
            let reader =
                AudioReader::from_source_info(reader_output_stream_id, source_info).or_fail()?;
            scheduler.register(reader).or_fail()?;

            let decoder_output_stream_id = next_stream_id.fetch_add(1);
            let decoder = AudioDecoder::new_opus(reader_output_stream_id, decoder_output_stream_id)
                .or_fail()?;
            scheduler.register(decoder).or_fail()?;

            audio_mixer_input_stream_ids.push(decoder_output_stream_id);
        }

        let mut video_mixer_input_stream_ids = Vec::new();
        let video_decoder_options = VideoDecoderOptions {
            openh264_lib: self.openh264_lib.clone(),
        };
        for source_id in self.layout.video_source_ids() {
            let source_info = self.layout.sources.get(source_id).or_fail()?;
            let reader_output_stream_id = next_stream_id.fetch_add(1);
            let reader =
                VideoReader::from_source_info(reader_output_stream_id, source_info).or_fail()?;
            scheduler.register(reader).or_fail()?;

            let decoder_output_stream_id = next_stream_id.fetch_add(1);
            let decoder = VideoDecoder::new(
                reader_output_stream_id,
                decoder_output_stream_id,
                video_decoder_options.clone(),
            );
            scheduler.register(decoder).or_fail()?;

            video_mixer_input_stream_ids.push(decoder_output_stream_id);
        }

        // ミキサーを登録
        let audio_mixer_output_stream_id = next_stream_id.fetch_add(1);
        let audio_mixer = AudioMixer::new(
            self.layout.clone(),
            audio_mixer_input_stream_ids,
            audio_mixer_output_stream_id,
        );
        scheduler.register(audio_mixer).or_fail()?;

        let video_mixer_output_stream_id = next_stream_id.fetch_add(1);
        let video_mixer = VideoMixer::new(
            self.layout.clone(),
            video_mixer_input_stream_ids,
            video_mixer_output_stream_id,
        );
        scheduler.register(video_mixer).or_fail()?;

        // エンコーダーを登録
        let audio_encoder_output_stream_id = next_stream_id.fetch_add(1);
        let audio_encoder = AudioEncoder::new(
            &self.layout,
            audio_mixer_output_stream_id,
            audio_encoder_output_stream_id,
        )
        .or_fail()?;
        scheduler.register(audio_encoder).or_fail()?;

        let video_encoder_output_stream_id = next_stream_id.fetch_add(1);
        let video_encoder = VideoEncoder::new(
            &self.layout,
            video_mixer_output_stream_id,
            video_encoder_output_stream_id,
            self.openh264_lib.clone(),
        )
        .or_fail()?;
        scheduler.register(video_encoder).or_fail()?;

        // ライターを登録
        let writer = Mp4Writer::new(
            out_file_path,
            &self.layout,
            self.layout
                .has_audio()
                .then_some(audio_encoder_output_stream_id),
            self.layout
                .has_video()
                .then_some(video_encoder_output_stream_id),
        )
        .or_fail()?;
        scheduler.register(writer).or_fail()?;

        // プログレスバーを登録
        if self.show_progress_bar {
            let progress = ProgressBar::new(
                vec![
                    audio_encoder_output_stream_id,
                    video_encoder_output_stream_id,
                ],
                self.layout.output_duration(),
            );
            scheduler.register(progress).or_fail()?;
        }

        // 合成を実行
        let stats = scheduler.run().or_fail()?;
        if stats.error.get() {
            return Err(orfail::Failure::new("composition process failed").into());
        }

        // 統計情報の準備（実際にファイル出力するかどうかに関わらず、集計自体は常に行う）
        let stats = SharedStats::new();
        let start_time = Instant::now();

        // 映像および音声ソースの準備
        let error_flag = ErrorFlag::new();
        let (audio_source_rxs, video_source_rxs) = create_audio_and_video_sources(
            &self.layout,
            error_flag.clone(),
            stats.clone(),
            self.openh264_lib.clone(),
        )
        .or_fail()?;

        // プログレスバーを準備
        let progress_bar =
            crate::arg_utils::create_time_progress_bar(self.layout.output_duration());

        // 映像ミキサーとエンコーダーを準備
        let mut encoded_video_rx = self
            .create_video_mixer_and_encoder(error_flag.clone(), stats.clone(), video_source_rxs)
            .or_fail()?;

        // 音声ミキサーとエンコーダーを準備
        let mut encoded_audio_rx = self
            .create_audio_mixer_and_encoder(error_flag.clone(), stats.clone(), audio_source_rxs)
            .or_fail()?;

        // 合成後の映像と音声への MP4 への書き出しを行う（この処理は現在のスレッドで行う）
        let writer_input_audio_stream_id = MediaStreamId::new(1000); // audio / video で値が異なっていればなんでもいい
        let writer_input_video_stream_id = MediaStreamId::new(1001);

        let mut mp4_writer = Mp4Writer::new(
            out_file_path,
            &self.layout,
            self.layout
                .has_audio()
                .then_some(writer_input_audio_stream_id),
            self.layout
                .has_video()
                .then_some(writer_input_video_stream_id),
        )
        .or_fail()?;

        loop {
            match mp4_writer.process_output().or_fail()? {
                MediaProcessorOutput::Finished => break,
                MediaProcessorOutput::Pending { awaiting_stream_id }
                    if awaiting_stream_id == Some(writer_input_audio_stream_id) =>
                {
                    let input = MediaProcessorInput {
                        stream_id: writer_input_audio_stream_id,
                        sample: encoded_audio_rx.recv().map(MediaSample::audio_data),
                    };
                    mp4_writer.process_input(input).or_fail()?;
                }
                MediaProcessorOutput::Pending { awaiting_stream_id } => {
                    let input = MediaProcessorInput {
                        stream_id: awaiting_stream_id.expect("infallible"),
                        sample: encoded_video_rx.recv().map(MediaSample::video_frame),
                    };
                    mp4_writer.process_input(input).or_fail()?;
                }
                MediaProcessorOutput::Processed { .. } => unreachable!(),
            }

            let timestamp = mp4_writer.current_duration();
            progress_bar.set_position(timestamp.as_secs());
            if error_flag.get() {
                // ファイル読み込み、デコード、合成、エンコード、のいずれかで失敗したものがあるとここに来る
                log::error!("The composition process was aborted");
                break;
            }
        }

        // 全ての処理が完了したので、プログレスバーと統計処理の後始末を行う
        progress_bar.finish();
        self.finish_stats(stats.clone(), &mp4_writer, start_time);

        Ok(ComposeResult {
            stats,
            success: !error_flag.get(),
        })
    }

    fn create_video_mixer_and_encoder(
        &self,
        error_flag: ErrorFlag,
        stats: SharedStats,
        video_source_rxs: Vec<VideoFrameReceiver>,
    ) -> orfail::Result<VideoFrameReceiver> {
        if !self.layout.has_video() {
            // 映像が処理対象外の場合には、ダミーのレシーバーを返す
            let (_, rx) = channel::sync_channel();
            return Ok(rx);
        }

        let mixed_video_rx = VideoMixerThread::start(
            error_flag.clone(),
            self.layout.clone(),
            video_source_rxs,
            stats.clone(),
        );

        let encoder = VideoEncoder::new(
            &self.layout,
            MediaStreamId::new(1000),
            MediaStreamId::new(1001),
            self.openh264_lib.clone(),
        )
        .or_fail()?;
        let encoded_video_rx =
            VideoEncoderThread::start(error_flag.clone(), mixed_video_rx, encoder, stats.clone());
        Ok(encoded_video_rx)
    }

    fn create_audio_mixer_and_encoder(
        &self,
        error_flag: ErrorFlag,
        stats: SharedStats,
        audio_source_rxs: Vec<AudioDataReceiver>,
    ) -> orfail::Result<AudioDataReceiver> {
        if !self.layout.has_audio() {
            // 音声が処理対象外の場合には、ダミーのレシーバーを返す
            let (_, rx) = channel::sync_channel();
            return Ok(rx);
        }

        let mixed_audio_rx = AudioMixerThread::start(
            error_flag.clone(),
            self.layout.clone(),
            audio_source_rxs,
            stats.clone(),
        );

        let audio_encoder =
            AudioEncoder::new(&self.layout, MediaStreamId::new(0), MediaStreamId::new(1))
                .or_fail()?;
        let encoded_audio_rx = AudioEncoderThread::start(
            error_flag.clone(),
            mixed_audio_rx,
            audio_encoder,
            stats.clone(),
        );
        Ok(encoded_audio_rx)
    }

    fn finish_stats(&self, stats: SharedStats, mp4_writer: &Mp4Writer, start_time: Instant) {
        stats.with_lock(|stats| {
            stats
                .processors
                .push(ProcessorStats::Mp4Writer(mp4_writer.stats().clone()));
            log::debug!("stats: {}", nojson::Json(&stats));

            stats.elapsed_seconds = Seconds::new(start_time.elapsed());
        });

        if let Some(path) = &self.stats_file_path {
            stats.save(path);
        }
    }
}

pub fn create_audio_and_video_sources(
    layout: &Layout,
    error_flag: ErrorFlag,
    stats: SharedStats,
    openh264_lib: Option<Openh264Library>,
) -> orfail::Result<(Vec<AudioDataReceiver>, Vec<VideoFrameReceiver>)> {
    let mut stream_id_gen = MediaStreamIdGenerator::new();

    let audio_source_ids = layout.audio_source_ids().collect::<HashSet<_>>();
    let video_source_ids = layout.video_source_ids().collect::<HashSet<_>>();

    let mut audio_source_rxs = Vec::new();
    let mut video_source_rxs = Vec::new();
    for (source_id, source_info) in &layout.sources {
        if audio_source_ids.contains(source_id) && source_info.audio {
            let source_rx = AudioSourceThread::start(
                error_flag.clone(),
                source_info,
                &mut stream_id_gen,
                stats.clone(),
            )
            .or_fail()?;
            audio_source_rxs.push(source_rx);
        }
        if video_source_ids.contains(source_id) && source_info.video {
            let options = VideoDecoderOptions {
                openh264_lib: openh264_lib.clone(),
            };
            let source_rx = VideoSourceThread::start(
                error_flag.clone(),
                source_info,
                options,
                &mut stream_id_gen,
                stats.clone(),
            )
            .or_fail()?;
            video_source_rxs.push(source_rx);
        }
    }
    Ok((audio_source_rxs, video_source_rxs))
}

#[derive(Debug)]
struct ProgressBar {
    input_stream_ids: Vec<MediaStreamId>,
    bar: indicatif::ProgressBar,
    max_timestamp: Duration,
}

impl ProgressBar {
    fn new(input_stream_ids: Vec<MediaStreamId>, output_duration: Duration) -> Self {
        Self {
            input_stream_ids,
            bar: crate::arg_utils::create_time_progress_bar(output_duration),
            max_timestamp: Duration::ZERO,
        }
    }
}

impl MediaProcessor for ProgressBar {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: self.input_stream_ids.clone(),
            output_stream_ids: Vec::new(),
            stats: ProcessorStats::other("progress-bar"),
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        if let Some(sample) = input.sample {
            self.max_timestamp = self.max_timestamp.max(sample.timestamp());
            self.bar.set_position(self.max_timestamp.as_secs());
        } else {
            self.input_stream_ids.retain(|id| *id != input.stream_id);
        };
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        if self.input_stream_ids.is_empty() {
            self.bar.finish();
            Ok(MediaProcessorOutput::Finished)
        } else {
            Ok(MediaProcessorOutput::awaiting_any())
        }
    }
}
