use std::{num::NonZeroUsize, path::PathBuf, time::Duration};

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{
    decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions},
    encoder::{AudioEncoder, VideoEncoder, VideoEncoderOptions},
    layout::Layout,
    media::MediaStreamId,
    mixer_audio::AudioMixer,
    mixer_video::{VideoMixer, VideoMixerSpec},
    processor::{
        MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
        MediaProcessorWorkloadHint,
    },
    reader::{AudioReader, VideoReader},
    scheduler::Scheduler,
    writer_mp4::{Mp4Writer, Mp4WriterOptions},
};

#[derive(Debug)]
pub struct Composer {
    pub layout: Layout,
    pub openh264_lib: Option<Openh264Library>,
    pub show_progress_bar: bool,
    pub worker_threads: NonZeroUsize,
    pub stats_file_path: Option<PathBuf>,
}

#[derive(Debug)]
pub struct ComposeResult {
    pub stats: crate::stats::Stats,
    pub elapsed_duration: Duration,
    pub success: bool,
}

impl Composer {
    pub fn new(layout: Layout) -> Self {
        Self {
            layout,
            openh264_lib: None,
            show_progress_bar: false,
            worker_threads: NonZeroUsize::MIN,
            stats_file_path: None,
        }
    }

    pub fn compose(&self, out_file_path: &std::path::Path) -> orfail::Result<ComposeResult> {
        // プロセッサを準備
        let mut scheduler = Scheduler::with_thread_count(self.worker_threads);
        let mut next_stream_id = MediaStreamId::new(0);
        let stats = crate::stats::Stats::new();
        let mut next_processor_index = 0;
        let mut scoped_stats = |processor_type: &'static str| {
            let mut scoped = stats.clone();
            scoped.set_default_label(
                "processor_id",
                &format!("{processor_type}:{next_processor_index}"),
            );
            scoped.set_default_label("processor_type", processor_type);
            next_processor_index += 1;
            scoped
        };

        // リーダーとデコーダーを登録
        let mut audio_mixer_input_stream_ids = Vec::new();
        for source_id in self.layout.audio_source_ids() {
            let source_info = self.layout.sources.get(source_id).or_fail()?;
            let reader_output_stream_id = next_stream_id.fetch_add(1);
            let reader = AudioReader::new_with_stats(
                reader_output_stream_id,
                source_info.id.clone(),
                source_info.format,
                source_info.start_timestamp,
                source_info.timestamp_sorted_media_paths(),
                scoped_stats(match source_info.format {
                    crate::metadata::ContainerFormat::Mp4 => "mp4_audio_reader",
                    crate::metadata::ContainerFormat::Webm => "webm_audio_reader",
                }),
            )
            .or_fail()?;
            scheduler.register(reader).or_fail()?;

            let decoder_output_stream_id = next_stream_id.fetch_add(1);
            let decoder = AudioDecoder::new_with_stats(
                reader_output_stream_id,
                decoder_output_stream_id,
                scoped_stats("audio_decoder"),
            )
            .or_fail()?;
            scheduler.register(decoder).or_fail()?;

            audio_mixer_input_stream_ids.push(decoder_output_stream_id);
        }

        let mut video_mixer_input_stream_ids = Vec::new();
        let video_decoder_options = VideoDecoderOptions {
            openh264_lib: self.openh264_lib.clone(),
            decode_params: self.layout.decode_params.clone(),
            engines: self.layout.video_decode_engines.clone(),
        };
        for source_id in self.layout.video_source_ids() {
            let source_info = self.layout.sources.get(source_id).or_fail()?;
            let reader_output_stream_id = next_stream_id.fetch_add(1);
            let reader = VideoReader::new_with_stats(
                reader_output_stream_id,
                source_info.id.clone(),
                source_info.format,
                source_info.start_timestamp,
                source_info.timestamp_sorted_media_paths(),
                scoped_stats(match source_info.format {
                    crate::metadata::ContainerFormat::Mp4 => "mp4_video_reader",
                    crate::metadata::ContainerFormat::Webm => "webm_video_reader",
                }),
            )
            .or_fail()?;
            scheduler.register(reader).or_fail()?;

            let decoder_output_stream_id = next_stream_id.fetch_add(1);
            let decoder = VideoDecoder::new_with_stats(
                reader_output_stream_id,
                decoder_output_stream_id,
                video_decoder_options.clone(),
                scoped_stats("video_decoder"),
            );
            scheduler.register(decoder).or_fail()?;

            video_mixer_input_stream_ids.push(decoder_output_stream_id);
        }

        // ミキサーを登録
        let audio_mixer_output_stream_id = next_stream_id.fetch_add(1);
        let audio_mixer = AudioMixer::new_with_stats(
            self.layout.trim_spans.clone(),
            audio_mixer_input_stream_ids,
            audio_mixer_output_stream_id,
            scoped_stats("audio_mixer"),
        );
        scheduler.register(audio_mixer).or_fail()?;

        let video_mixer_output_stream_id = next_stream_id.fetch_add(1);
        let video_mixer = VideoMixer::new_with_stats(
            VideoMixerSpec::from_layout(&self.layout),
            video_mixer_input_stream_ids,
            video_mixer_output_stream_id,
            scoped_stats("video_mixer"),
        );
        scheduler.register(video_mixer).or_fail()?;

        // エンコーダーを登録
        let audio_encoder_output_stream_id = next_stream_id.fetch_add(1);
        let audio_encoder = AudioEncoder::new_with_stats(
            self.layout.audio_codec,
            self.layout.audio_bitrate_bps(),
            audio_mixer_output_stream_id,
            audio_encoder_output_stream_id,
            scoped_stats("audio_encoder"),
        )
        .or_fail()?;
        scheduler.register(audio_encoder).or_fail()?;

        let video_encoder_output_stream_id = next_stream_id.fetch_add(1);
        let video_encoder = VideoEncoder::new_with_stats(
            &VideoEncoderOptions::from_layout(&self.layout),
            video_mixer_output_stream_id,
            video_encoder_output_stream_id,
            self.openh264_lib.clone(),
            scoped_stats("video_encoder"),
        )
        .or_fail()?;
        scheduler.register(video_encoder).or_fail()?;

        // ライターを登録
        let writer = Mp4Writer::new_with_stats(
            out_file_path,
            Some(Mp4WriterOptions::from_layout(&self.layout)),
            self.layout
                .has_audio()
                .then_some(audio_encoder_output_stream_id),
            self.layout
                .has_video()
                .then_some(video_encoder_output_stream_id),
            scoped_stats("mp4_writer"),
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
        let scheduler_result = scheduler.run().or_fail()?;

        if let Some(path) = &self.stats_file_path {
            match crate::stats_legacy_json::to_legacy_stats_json(
                &stats,
                scheduler_result.elapsed_duration.as_secs_f64(),
            ) {
                Ok(json) => {
                    if let Err(e) = std::fs::write(path, json.to_string()) {
                        // 統計が出力できなくても全体を失敗扱いにはしない
                        tracing::warn!(
                            "failed to write stats JSON: path={}, reason={e}",
                            path.display()
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "failed to build stats JSON: path={}, reason={e}",
                        path.display()
                    );
                }
            }
        }

        Ok(ComposeResult {
            stats,
            elapsed_duration: scheduler_result.elapsed_duration,
            success: !scheduler_result.error,
        })
    }
}

#[derive(Debug)]
struct ProgressBar {
    input_stream_ids: Vec<MediaStreamId>,
    bar: crate::progress::ProgressBar,
    max_timestamp: Duration,
}

impl ProgressBar {
    fn new(input_stream_ids: Vec<MediaStreamId>, output_duration: Duration) -> Self {
        Self {
            input_stream_ids,
            bar: crate::progress::ProgressBar::new(
                output_duration.as_secs(),
                crate::progress::ProgressKind::Time,
            ),
            max_timestamp: Duration::ZERO,
        }
    }
}

impl MediaProcessor for ProgressBar {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: self.input_stream_ids.clone(),
            output_stream_ids: Vec::new(),
            workload_hint: MediaProcessorWorkloadHint::WRITER,
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
