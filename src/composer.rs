use std::{num::NonZeroUsize, path::PathBuf, time::Duration};

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{
    decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions},
    encoder::{AudioEncoder, VideoEncoder},
    layout::Layout,
    media::MediaStreamId,
    mixer_audio::AudioMixer,
    mixer_video::{VideoMixer, VideoMixerSpec},
    processor::{MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec},
    reader::{AudioReader, VideoReader},
    scheduler::Scheduler,
    stats::{ProcessorStats, Stats},
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
    pub stats: Stats,
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
            self.layout.trim_spans.clone(),
            audio_mixer_input_stream_ids,
            audio_mixer_output_stream_id,
        );
        scheduler.register(audio_mixer).or_fail()?;

        let video_mixer_output_stream_id = next_stream_id.fetch_add(1);
        let video_mixer = VideoMixer::new(
            VideoMixerSpec::from_layout(&self.layout),
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
            return Err(orfail::Failure::new("composition process failed"));
        }

        if let Some(path) = &self.stats_file_path {
            stats.save(path);
        }

        Ok(ComposeResult {
            success: !stats.error.get(),
            stats,
        })
    }
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
