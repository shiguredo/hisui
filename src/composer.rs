use std::{num::NonZeroUsize, path::PathBuf, time::Duration};

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{
    decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions},
    encoder::{AudioEncoder, VideoEncoder, VideoEncoderOptions},
    layout::Layout,
    legacy_processor_stats::ProcessorStats,
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

        // リーダーとデコーダーを登録
        let mut audio_mixer_input_stream_ids = Vec::new();
        for source_id in self.layout.audio_source_ids() {
            let source_info = self.layout.sources.get(source_id).or_fail()?;
            let reader_output_stream_id = next_stream_id.fetch_add(1);
            let reader =
                AudioReader::from_source_info(reader_output_stream_id, source_info).or_fail()?;
            scheduler.register(reader).or_fail()?;

            let decoder_output_stream_id = next_stream_id.fetch_add(1);
            let decoder =
                AudioDecoder::new(reader_output_stream_id, decoder_output_stream_id).or_fail()?;
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
            self.layout.audio_codec,
            self.layout.audio_bitrate_bps(),
            audio_mixer_output_stream_id,
            audio_encoder_output_stream_id,
        )
        .or_fail()?;
        scheduler.register(audio_encoder).or_fail()?;

        let video_encoder_output_stream_id = next_stream_id.fetch_add(1);
        let video_encoder = VideoEncoder::new(
            &VideoEncoderOptions::from_layout(&self.layout),
            video_mixer_output_stream_id,
            video_encoder_output_stream_id,
            self.openh264_lib.clone(),
        )
        .or_fail()?;
        scheduler.register(video_encoder).or_fail()?;

        // ライターを登録
        let writer = Mp4Writer::new(
            out_file_path,
            Some(Mp4WriterOptions::from_layout(&self.layout)),
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
        let scheduler_result = scheduler.run().or_fail()?;
        let stats = convert_legacy_stats_to_stats(&scheduler_result.processors);

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

fn convert_legacy_stats_to_stats(legacy_processors: &[ProcessorStats]) -> crate::stats::Stats {
    let stats = crate::stats::Stats::new();
    for (index, processor) in legacy_processors.iter().enumerate() {
        let (processor_type, error) = match processor {
            ProcessorStats::Mp4AudioReader(reader) => ("mp4_audio_reader", reader.error.get()),
            ProcessorStats::Mp4VideoReader(reader) => ("mp4_video_reader", reader.error.get()),
            ProcessorStats::WebmAudioReader(reader) => ("webm_audio_reader", reader.error.get()),
            ProcessorStats::WebmVideoReader(reader) => ("webm_video_reader", reader.error.get()),
            ProcessorStats::AudioDecoder(decoder) => ("audio_decoder", decoder.error.get()),
            ProcessorStats::VideoDecoder(decoder) => ("video_decoder", decoder.error.get()),
            ProcessorStats::AudioMixer(mixer) => ("audio_mixer", mixer.error.get()),
            ProcessorStats::VideoMixer(mixer) => ("video_mixer", mixer.error.get()),
            ProcessorStats::AudioEncoder(encoder) => ("audio_encoder", encoder.error.get()),
            ProcessorStats::VideoEncoder(encoder) => ("video_encoder", encoder.error.get()),
            ProcessorStats::Mp4Writer(writer) => ("mp4_writer", writer.error.get()),
        };

        let mut processor_stats = stats.clone();
        processor_stats.set_default_label("processor_id", &format!("legacy_processor_{index}"));
        processor_stats.set_default_label("processor_type", processor_type);
        processor_stats.flag("error").set(error);

        match processor {
            ProcessorStats::Mp4AudioReader(reader) => {
                if let Some(codec) = reader.codec {
                    processor_stats.string("codec").set(codec.as_str());
                }
                if let Some(path) = reader.current_input_file.get() {
                    processor_stats
                        .string("current_input_file")
                        .set(path.display().to_string());
                }
                processor_stats
                    .counter("total_sample_count")
                    .add(reader.total_sample_count.get());
                processor_stats.gauge_f64("total_track_seconds").set(
                    (reader.track_duration_offset.get() + reader.total_track_duration.get())
                        .as_secs_f64(),
                );
                processor_stats
                    .gauge_f64("start_time_seconds")
                    .set(reader.start_time.as_secs_f64());
            }
            ProcessorStats::Mp4VideoReader(reader) => {
                if let Some(codec) = reader.codec.get() {
                    processor_stats.string("codec").set(codec.as_str());
                }
                if let Some(path) = reader.current_input_file.get() {
                    processor_stats
                        .string("current_input_file")
                        .set(path.display().to_string());
                }
                processor_stats
                    .counter("total_sample_count")
                    .add(reader.total_sample_count.get());
                processor_stats.gauge_f64("total_track_seconds").set(
                    (reader.track_duration_offset.get() + reader.total_track_duration.get())
                        .as_secs_f64(),
                );
                processor_stats
                    .gauge_f64("start_time_seconds")
                    .set(reader.start_time.as_secs_f64());
            }
            ProcessorStats::WebmAudioReader(reader) => {
                if let Some(codec) = reader.codec {
                    processor_stats.string("codec").set(codec.as_str());
                }
                if let Some(path) = reader.current_input_file.get() {
                    processor_stats
                        .string("current_input_file")
                        .set(path.display().to_string());
                }
                processor_stats
                    .counter("total_cluster_count")
                    .add(reader.total_cluster_count.get());
                processor_stats
                    .counter("total_simple_block_count")
                    .add(reader.total_simple_block_count.get());
                processor_stats.gauge_f64("total_track_seconds").set(
                    (reader.track_duration_offset.get() + reader.total_track_duration.get())
                        .as_secs_f64(),
                );
                processor_stats
                    .gauge_f64("start_time_seconds")
                    .set(reader.start_time.as_secs_f64());
            }
            ProcessorStats::WebmVideoReader(reader) => {
                if let Some(codec) = reader.codec.get() {
                    processor_stats.string("codec").set(codec.as_str());
                }
                if let Some(path) = reader.current_input_file.get() {
                    processor_stats
                        .string("current_input_file")
                        .set(path.display().to_string());
                }
                processor_stats
                    .counter("total_cluster_count")
                    .add(reader.total_cluster_count.get());
                processor_stats
                    .counter("total_simple_block_count")
                    .add(reader.total_simple_block_count.get());
                processor_stats.gauge_f64("total_track_seconds").set(
                    (reader.track_duration_offset.get() + reader.total_track_duration.get())
                        .as_secs_f64(),
                );
                processor_stats
                    .gauge_f64("start_time_seconds")
                    .set(reader.start_time.as_secs_f64());
            }
            ProcessorStats::AudioEncoder(encoder) => {
                processor_stats
                    .string("engine")
                    .set(encoder.engine.as_str());
                processor_stats.string("codec").set(encoder.codec.as_str());
                processor_stats
                    .counter("total_audio_data_count")
                    .add(encoder.total_audio_data_count.get());
            }
            ProcessorStats::AudioDecoder(decoder) => {
                if let Some(engine) = decoder.engine {
                    processor_stats.string("engine").set(engine.as_str());
                }
                if let Some(codec) = decoder.codec {
                    processor_stats.string("codec").set(codec.as_str());
                }
                processor_stats
                    .counter("total_audio_data_count")
                    .add(decoder.total_audio_data_count.get());
            }
            ProcessorStats::VideoDecoder(decoder) => {
                if let Some(engine) = decoder.engine.get() {
                    processor_stats.string("engine").set(engine.as_str());
                }
                if let Some(codec) = decoder.codec.get() {
                    processor_stats.string("codec").set(codec.as_str());
                }
                processor_stats
                    .counter("total_input_video_frame_count")
                    .add(decoder.total_input_video_frame_count.get());
                processor_stats
                    .counter("total_output_video_frame_count")
                    .add(decoder.total_output_video_frame_count.get());
            }
            ProcessorStats::VideoEncoder(encoder) => {
                if let Some(engine) = encoder.engine.get() {
                    processor_stats.string("engine").set(engine.as_str());
                }
                if let Some(codec) = encoder.codec.get() {
                    processor_stats.string("codec").set(codec.as_str());
                }
                processor_stats
                    .counter("total_input_video_frame_count")
                    .add(encoder.total_input_video_frame_count.get());
                processor_stats
                    .counter("total_output_video_frame_count")
                    .add(encoder.total_output_video_frame_count.get());
            }
            ProcessorStats::AudioMixer(mixer) => {
                processor_stats
                    .counter("total_input_audio_data_count")
                    .add(mixer.total_input_audio_data_count.get());
                processor_stats
                    .counter("total_output_audio_data_count")
                    .add(mixer.total_output_audio_data_count.get());
                processor_stats
                    .gauge_f64("total_output_audio_data_seconds")
                    .set(mixer.total_output_audio_data_duration.get().as_secs_f64());
                processor_stats
                    .counter("total_output_sample_count")
                    .add(mixer.total_output_sample_count.get());
                processor_stats
                    .counter("total_output_filled_sample_count")
                    .add(mixer.total_output_filled_sample_count.get());
                processor_stats
                    .counter("total_trimmed_sample_count")
                    .add(mixer.total_trimmed_sample_count.get());
            }
            ProcessorStats::VideoMixer(mixer) => {
                processor_stats
                    .gauge("output_video_width")
                    .set(mixer.output_video_resolution.width as i64);
                processor_stats
                    .gauge("output_video_height")
                    .set(mixer.output_video_resolution.height as i64);
                processor_stats
                    .counter("total_input_video_frame_count")
                    .add(mixer.total_input_video_frame_count.get());
                processor_stats
                    .counter("total_output_video_frame_count")
                    .add(mixer.total_output_video_frame_count.get());
                processor_stats
                    .gauge_f64("total_output_video_frame_seconds")
                    .set(mixer.total_output_video_frame_duration.get().as_secs_f64());
                processor_stats
                    .counter("total_trimmed_video_frame_count")
                    .add(mixer.total_trimmed_video_frame_count.get());
                processor_stats
                    .counter("total_extended_video_frame_count")
                    .add(mixer.total_extended_video_frame_count.get());
            }
            ProcessorStats::Mp4Writer(writer) => {
                if let Some(audio_codec) = writer.audio_codec.get() {
                    processor_stats
                        .string("audio_codec")
                        .set(audio_codec.as_str());
                }
                if let Some(video_codec) = writer.video_codec.get() {
                    processor_stats
                        .string("video_codec")
                        .set(video_codec.as_str());
                }
                processor_stats
                    .counter("total_audio_sample_data_byte_size")
                    .add(writer.total_audio_sample_data_byte_size.get());
                processor_stats
                    .counter("total_video_sample_data_byte_size")
                    .add(writer.total_video_sample_data_byte_size.get());
                processor_stats
                    .counter("reserved_moov_box_size")
                    .add(writer.reserved_moov_box_size.get());
                processor_stats
                    .counter("actual_moov_box_size")
                    .add(writer.actual_moov_box_size.get());
                processor_stats
                    .counter("total_audio_chunk_count")
                    .add(writer.total_audio_chunk_count.get());
                processor_stats
                    .counter("total_video_chunk_count")
                    .add(writer.total_video_chunk_count.get());
                processor_stats
                    .counter("total_audio_sample_count")
                    .add(writer.total_audio_sample_count.get());
                processor_stats
                    .counter("total_video_sample_count")
                    .add(writer.total_video_sample_count.get());
                processor_stats
                    .gauge_f64("total_audio_track_seconds")
                    .set(writer.total_audio_track_duration.get().as_secs_f64());
                processor_stats
                    .gauge_f64("total_video_track_seconds")
                    .set(writer.total_video_track_duration.get().as_secs_f64());
            }
        }
    }
    stats
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::legacy_processor_stats as legacy;

    fn counter(value: u64) -> legacy::SharedAtomicCounter {
        let counter = legacy::SharedAtomicCounter::default();
        counter.set(value);
        counter
    }

    fn duration_secs(value: u64) -> legacy::SharedAtomicDuration {
        legacy::SharedAtomicDuration::new(Duration::from_secs(value))
    }

    #[test]
    fn convert_legacy_stats_to_stats_preserves_compose_summary_metrics() {
        let writer = legacy::Mp4WriterStats {
            audio_codec: legacy::SharedOption::new(Some(crate::types::CodecName::Opus)),
            video_codec: legacy::SharedOption::new(Some(crate::types::CodecName::Vp9)),
            total_audio_sample_data_byte_size: {
                let v = legacy::SharedAtomicCounter::default();
                v.set(1000);
                v
            },
            total_video_sample_data_byte_size: {
                let v = legacy::SharedAtomicCounter::default();
                v.set(2000);
                v
            },
            total_audio_track_duration: legacy::SharedAtomicDuration::new(Duration::from_secs(2)),
            total_video_track_duration: legacy::SharedAtomicDuration::new(Duration::from_secs(4)),
            ..Default::default()
        };

        let video_mixer = legacy::VideoMixerStats {
            output_video_resolution: legacy::VideoResolution {
                width: 320,
                height: 240,
            },
            ..Default::default()
        };

        let legacy_processors = vec![
            ProcessorStats::AudioEncoder(legacy::AudioEncoderStats::new(
                crate::types::EngineName::Opus,
                crate::types::CodecName::Opus,
            )),
            ProcessorStats::VideoEncoder(legacy::VideoEncoderStats {
                engine: legacy::SharedOption::new(Some(crate::types::EngineName::Libvpx)),
                codec: legacy::SharedOption::new(Some(crate::types::CodecName::Vp9)),
                ..Default::default()
            }),
            ProcessorStats::VideoMixer(video_mixer),
            ProcessorStats::Mp4Writer(writer),
        ];

        let stats = convert_legacy_stats_to_stats(&legacy_processors);
        let entries = stats.entries().expect("entries must succeed");

        assert!(entries.iter().any(|entry| {
            entry.metric_name == "audio_codec"
                && entry.labels.get("processor_type") == Some(&"mp4_writer".to_owned())
                && entry.value.as_string() == Some("OPUS".to_owned())
        }));
        assert!(entries.iter().any(|entry| {
            entry.metric_name == "video_codec"
                && entry.labels.get("processor_type") == Some(&"mp4_writer".to_owned())
                && entry.value.as_string() == Some("VP9".to_owned())
        }));
        assert!(entries.iter().any(|entry| {
            entry.metric_name == "engine"
                && entry.labels.get("processor_type") == Some(&"audio_encoder".to_owned())
                && entry.value.as_string() == Some("opus".to_owned())
        }));
        assert!(entries.iter().any(|entry| {
            entry.metric_name == "engine"
                && entry.labels.get("processor_type") == Some(&"video_encoder".to_owned())
                && entry.value.as_string() == Some("libvpx".to_owned())
        }));
        assert!(entries.iter().any(|entry| {
            entry.metric_name == "output_video_width"
                && entry.labels.get("processor_type") == Some(&"video_mixer".to_owned())
                && entry.value.as_gauge() == Some(320)
        }));
        assert!(entries.iter().any(|entry| {
            entry.metric_name == "output_video_height"
                && entry.labels.get("processor_type") == Some(&"video_mixer".to_owned())
                && entry.value.as_gauge() == Some(240)
        }));
    }

    #[test]
    fn convert_legacy_stats_to_stats_restores_simple_legacy_fields() {
        let legacy_processors = vec![
            ProcessorStats::Mp4AudioReader(legacy::Mp4AudioReaderStats {
                current_input_file: legacy::SharedOption::new(Some(PathBuf::from("/tmp/a.mp4"))),
                codec: Some(crate::types::CodecName::Opus),
                total_sample_count: counter(11),
                total_track_duration: duration_secs(4),
                track_duration_offset: duration_secs(1),
                start_time: Duration::from_secs(2),
                ..Default::default()
            }),
            ProcessorStats::Mp4VideoReader(legacy::Mp4VideoReaderStats {
                current_input_file: legacy::SharedOption::new(Some(PathBuf::from("/tmp/v.mp4"))),
                codec: legacy::SharedOption::new(Some(crate::types::CodecName::Vp9)),
                total_sample_count: counter(22),
                total_track_duration: duration_secs(5),
                track_duration_offset: duration_secs(2),
                start_time: Duration::from_secs(3),
                ..Default::default()
            }),
            ProcessorStats::WebmAudioReader(legacy::WebmAudioReaderStats {
                current_input_file: legacy::SharedOption::new(Some(PathBuf::from("/tmp/a.webm"))),
                codec: Some(crate::types::CodecName::Opus),
                total_cluster_count: counter(3),
                total_simple_block_count: counter(4),
                total_track_duration: duration_secs(6),
                track_duration_offset: duration_secs(1),
                start_time: Duration::from_secs(4),
                ..Default::default()
            }),
            ProcessorStats::WebmVideoReader(legacy::WebmVideoReaderStats {
                current_input_file: legacy::SharedOption::new(Some(PathBuf::from("/tmp/v.webm"))),
                codec: legacy::SharedOption::new(Some(crate::types::CodecName::Vp9)),
                total_cluster_count: counter(5),
                total_simple_block_count: counter(6),
                total_track_duration: duration_secs(7),
                track_duration_offset: duration_secs(2),
                start_time: Duration::from_secs(5),
                ..Default::default()
            }),
            ProcessorStats::AudioDecoder(legacy::AudioDecoderStats {
                engine: Some(crate::types::EngineName::Opus),
                codec: Some(crate::types::CodecName::Opus),
                total_audio_data_count: counter(8),
                ..Default::default()
            }),
            ProcessorStats::VideoDecoder(legacy::VideoDecoderStats {
                engine: legacy::SharedOption::new(Some(crate::types::EngineName::Libvpx)),
                codec: legacy::SharedOption::new(Some(crate::types::CodecName::Vp9)),
                total_input_video_frame_count: counter(9),
                total_output_video_frame_count: counter(10),
                ..Default::default()
            }),
            ProcessorStats::AudioMixer(legacy::AudioMixerStats {
                total_input_audio_data_count: counter(12),
                total_output_audio_data_count: counter(13),
                total_output_audio_data_duration: duration_secs(14),
                total_output_sample_count: counter(15),
                total_output_filled_sample_count: counter(16),
                total_trimmed_sample_count: counter(17),
                ..Default::default()
            }),
            ProcessorStats::VideoMixer(legacy::VideoMixerStats {
                output_video_resolution: legacy::VideoResolution {
                    width: 640,
                    height: 360,
                },
                total_input_video_frame_count: counter(18),
                total_output_video_frame_count: counter(19),
                total_output_video_frame_duration: duration_secs(20),
                total_trimmed_video_frame_count: counter(21),
                total_extended_video_frame_count: counter(22),
                ..Default::default()
            }),
            ProcessorStats::AudioEncoder(legacy::AudioEncoderStats {
                engine: crate::types::EngineName::Opus,
                codec: crate::types::CodecName::Opus,
                total_audio_data_count: counter(23),
                ..legacy::AudioEncoderStats::new(
                    crate::types::EngineName::Opus,
                    crate::types::CodecName::Opus,
                )
            }),
            ProcessorStats::VideoEncoder(legacy::VideoEncoderStats {
                engine: legacy::SharedOption::new(Some(crate::types::EngineName::Libvpx)),
                codec: legacy::SharedOption::new(Some(crate::types::CodecName::Vp9)),
                total_input_video_frame_count: counter(24),
                total_output_video_frame_count: counter(25),
                ..Default::default()
            }),
            ProcessorStats::Mp4Writer(legacy::Mp4WriterStats {
                audio_codec: legacy::SharedOption::new(Some(crate::types::CodecName::Opus)),
                video_codec: legacy::SharedOption::new(Some(crate::types::CodecName::Vp9)),
                reserved_moov_box_size: counter(26),
                actual_moov_box_size: counter(27),
                total_audio_chunk_count: counter(28),
                total_video_chunk_count: counter(29),
                total_audio_sample_count: counter(30),
                total_video_sample_count: counter(31),
                total_audio_sample_data_byte_size: counter(32),
                total_video_sample_data_byte_size: counter(33),
                total_audio_track_duration: duration_secs(34),
                total_video_track_duration: duration_secs(35),
                ..Default::default()
            }),
        ];

        let stats = convert_legacy_stats_to_stats(&legacy_processors);
        let entries = stats.entries().expect("entries must succeed");
        let has_counter = |processor_type: &str, metric_name: &str, value: u64| {
            entries.iter().any(|entry| {
                entry.labels.get("processor_type") == Some(&processor_type.to_owned())
                    && entry.metric_name == metric_name
                    && entry.value.as_counter() == Some(value)
            })
        };
        let has_gauge_f64 = |processor_type: &str, metric_name: &str, value: f64| {
            entries.iter().any(|entry| {
                entry.labels.get("processor_type") == Some(&processor_type.to_owned())
                    && entry.metric_name == metric_name
                    && entry.value.as_gauge_f64() == Some(value)
            })
        };

        assert!(has_counter("mp4_audio_reader", "total_sample_count", 11,));
        assert!(has_counter("webm_audio_reader", "total_cluster_count", 3,));
        assert!(has_counter("audio_decoder", "total_audio_data_count", 8,));
        assert!(has_counter(
            "video_decoder",
            "total_output_video_frame_count",
            10,
        ));
        assert!(has_gauge_f64(
            "audio_mixer",
            "total_output_audio_data_seconds",
            14.0,
        ));
        assert!(has_gauge_f64(
            "video_mixer",
            "total_output_video_frame_seconds",
            20.0,
        ));
        assert!(has_counter("audio_encoder", "total_audio_data_count", 23,));
        assert!(has_counter(
            "video_encoder",
            "total_input_video_frame_count",
            24,
        ));
        assert!(has_counter("mp4_writer", "reserved_moov_box_size", 26,));
    }

    #[test]
    fn convert_legacy_stats_to_stats_omits_total_processing_seconds() {
        let legacy_processors = vec![ProcessorStats::AudioDecoder(
            legacy::AudioDecoderStats::default(),
        )];

        let stats = convert_legacy_stats_to_stats(&legacy_processors);
        let entries = stats.entries().expect("entries must succeed");
        assert!(
            !entries
                .iter()
                .any(|entry| entry.metric_name == "total_processing_seconds")
        );
    }
}
