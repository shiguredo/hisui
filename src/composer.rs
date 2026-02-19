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
    stats_legacy::{ProcessorStats, Stats as LegacyStats, WorkerThreadStats},
    stats_legacy_json::LegacyWorkerThreadStats,
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
    pub worker_threads: Vec<LegacyWorkerThreadStats>,
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
        let legacy_stats = scheduler.run().or_fail()?;
        let stats = convert_legacy_stats_to_stats(&legacy_stats);
        let worker_threads = convert_worker_thread_stats(&legacy_stats.worker_threads);

        if let Some(path) = &self.stats_file_path {
            match crate::stats_legacy_json::to_legacy_stats_json(
                &stats,
                legacy_stats.elapsed_duration.as_secs_f64(),
                worker_threads.clone(),
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
            elapsed_duration: legacy_stats.elapsed_duration,
            worker_threads,
            success: !legacy_stats.error.get(),
        })
    }
}

fn convert_worker_thread_stats(
    worker_threads: &[WorkerThreadStats],
) -> Vec<LegacyWorkerThreadStats> {
    worker_threads
        .iter()
        .map(|worker| LegacyWorkerThreadStats {
            total_processing_seconds: worker.total_processing_duration.get().as_secs_f64(),
            total_waiting_seconds: worker.total_waiting_duration.get().as_secs_f64(),
        })
        .collect()
}

fn convert_legacy_stats_to_stats(legacy_stats: &LegacyStats) -> crate::stats::Stats {
    let stats = crate::stats::Stats::new();
    for (index, processor) in legacy_stats.processors.iter().enumerate() {
        let (processor_type, total_processing_seconds, error) = match processor {
            ProcessorStats::Mp4AudioReader(reader) => (
                "mp4_audio_reader",
                reader.total_processing_duration.get().as_secs_f64(),
                reader.error.get(),
            ),
            ProcessorStats::Mp4VideoReader(reader) => (
                "mp4_video_reader",
                reader.total_processing_duration.get().as_secs_f64(),
                reader.error.get(),
            ),
            ProcessorStats::WebmAudioReader(reader) => (
                "webm_audio_reader",
                reader.total_processing_duration.get().as_secs_f64(),
                reader.error.get(),
            ),
            ProcessorStats::WebmVideoReader(reader) => (
                "webm_video_reader",
                reader.total_processing_duration.get().as_secs_f64(),
                reader.error.get(),
            ),
            ProcessorStats::AudioDecoder(decoder) => (
                "audio_decoder",
                decoder.total_processing_duration.get().as_secs_f64(),
                decoder.error.get(),
            ),
            ProcessorStats::VideoDecoder(decoder) => (
                "video_decoder",
                decoder.total_processing_duration.get().as_secs_f64(),
                decoder.error.get(),
            ),
            ProcessorStats::AudioMixer(mixer) => (
                "audio_mixer",
                mixer.total_processing_duration.get().as_secs_f64(),
                mixer.error.get(),
            ),
            ProcessorStats::VideoMixer(mixer) => (
                "video_mixer",
                mixer.total_processing_duration.get().as_secs_f64(),
                mixer.error.get(),
            ),
            ProcessorStats::AudioEncoder(encoder) => (
                "audio_encoder",
                encoder.total_processing_duration.get().as_secs_f64(),
                encoder.error.get(),
            ),
            ProcessorStats::VideoEncoder(encoder) => (
                "video_encoder",
                encoder.total_processing_duration.get().as_secs_f64(),
                encoder.error.get(),
            ),
            ProcessorStats::Mp4Writer(writer) => (
                "mp4_writer",
                writer.total_processing_duration.get().as_secs_f64(),
                writer.error.get(),
            ),
            ProcessorStats::RtmpPublisher(publisher) => (
                "rtmp_publisher",
                publisher.total_processing_duration.get().as_secs_f64(),
                publisher.error.get(),
            ),
            ProcessorStats::RtmpOutboundEndpoint(endpoint) => (
                "rtmp_outbound_endpoint",
                endpoint.total_processing_duration.get().as_secs_f64(),
                endpoint.error.get(),
            ),
            ProcessorStats::RtmpInboundEndpoint(endpoint) => (
                "rtmp_inbound_endpoint",
                endpoint.total_processing_duration.get().as_secs_f64(),
                endpoint.error.get(),
            ),
            ProcessorStats::Other {
                processor_type,
                total_processing_duration,
                error,
            } => (
                processor_type.as_str(),
                total_processing_duration.get().as_secs_f64(),
                error.get(),
            ),
        };

        let mut processor_stats = stats.clone();
        processor_stats.set_default_label("processor_id", &format!("legacy_processor_{index}"));
        processor_stats.set_default_label("processor_type", processor_type);
        processor_stats.flag("error").set(error);
        processor_stats
            .gauge_f64("total_processing_seconds")
            .set(total_processing_seconds);

        match processor {
            ProcessorStats::AudioEncoder(encoder) => {
                processor_stats
                    .string("engine")
                    .set(encoder.engine.as_str());
                processor_stats.string("codec").set(encoder.codec.as_str());
            }
            ProcessorStats::VideoEncoder(encoder) => {
                if let Some(engine) = encoder.engine.get() {
                    processor_stats.string("engine").set(engine.as_str());
                }
                if let Some(codec) = encoder.codec.get() {
                    processor_stats.string("codec").set(codec.as_str());
                }
            }
            ProcessorStats::VideoMixer(mixer) => {
                processor_stats
                    .gauge("output_video_width")
                    .set(mixer.output_video_resolution.width as i64);
                processor_stats
                    .gauge("output_video_height")
                    .set(mixer.output_video_resolution.height as i64);
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
                    .gauge_f64("total_audio_track_seconds")
                    .set(writer.total_audio_track_duration.get().as_secs_f64());
                processor_stats
                    .gauge_f64("total_video_track_seconds")
                    .set(writer.total_video_track_duration.get().as_secs_f64());
            }
            _ => {}
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
            stats: ProcessorStats::other("progress_bar"),
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
    use super::*;

    #[test]
    fn convert_legacy_stats_to_stats_preserves_compose_summary_metrics() {
        let writer = crate::stats_legacy::Mp4WriterStats {
            audio_codec: crate::stats_legacy::SharedOption::new(Some(
                crate::types::CodecName::Opus,
            )),
            video_codec: crate::stats_legacy::SharedOption::new(Some(crate::types::CodecName::Vp9)),
            total_audio_sample_data_byte_size: {
                let v = crate::stats_legacy::SharedAtomicCounter::default();
                v.set(1000);
                v
            },
            total_video_sample_data_byte_size: {
                let v = crate::stats_legacy::SharedAtomicCounter::default();
                v.set(2000);
                v
            },
            total_audio_track_duration: crate::stats_legacy::SharedAtomicDuration::new(
                Duration::from_secs(2),
            ),
            total_video_track_duration: crate::stats_legacy::SharedAtomicDuration::new(
                Duration::from_secs(4),
            ),
            ..Default::default()
        };

        let video_mixer = crate::stats_legacy::VideoMixerStats {
            output_video_resolution: crate::stats_legacy::VideoResolution {
                width: 320,
                height: 240,
            },
            ..Default::default()
        };

        let legacy_stats = LegacyStats {
            elapsed_duration: Duration::from_secs(5),
            error: Default::default(),
            processors: vec![
                ProcessorStats::AudioEncoder(crate::stats_legacy::AudioEncoderStats::new(
                    crate::types::EngineName::Opus,
                    crate::types::CodecName::Opus,
                )),
                ProcessorStats::VideoEncoder(crate::stats_legacy::VideoEncoderStats {
                    engine: crate::stats_legacy::SharedOption::new(Some(
                        crate::types::EngineName::Libvpx,
                    )),
                    codec: crate::stats_legacy::SharedOption::new(Some(
                        crate::types::CodecName::Vp9,
                    )),
                    ..Default::default()
                }),
                ProcessorStats::VideoMixer(video_mixer),
                ProcessorStats::Mp4Writer(writer),
            ],
            worker_threads: Vec::new(),
        };

        let stats = convert_legacy_stats_to_stats(&legacy_stats);
        let entries = stats
            .snapshot_entries()
            .expect("snapshot_entries must succeed");

        assert!(entries.iter().any(|entry| {
            entry.metric_name == "audio_codec"
                && entry.labels.get("processor_type") == Some(&"mp4_writer".to_owned())
                && entry.value == crate::stats::StatsSnapshotValue::String("OPUS".to_owned())
        }));
        assert!(entries.iter().any(|entry| {
            entry.metric_name == "video_codec"
                && entry.labels.get("processor_type") == Some(&"mp4_writer".to_owned())
                && entry.value == crate::stats::StatsSnapshotValue::String("VP9".to_owned())
        }));
        assert!(entries.iter().any(|entry| {
            entry.metric_name == "engine"
                && entry.labels.get("processor_type") == Some(&"audio_encoder".to_owned())
                && entry.value == crate::stats::StatsSnapshotValue::String("opus".to_owned())
        }));
        assert!(entries.iter().any(|entry| {
            entry.metric_name == "engine"
                && entry.labels.get("processor_type") == Some(&"video_encoder".to_owned())
                && entry.value == crate::stats::StatsSnapshotValue::String("libvpx".to_owned())
        }));
        assert!(entries.iter().any(|entry| {
            entry.metric_name == "output_video_width"
                && entry.labels.get("processor_type") == Some(&"video_mixer".to_owned())
                && entry.value == crate::stats::StatsSnapshotValue::Gauge(320)
        }));
        assert!(entries.iter().any(|entry| {
            entry.metric_name == "output_video_height"
                && entry.labels.get("processor_type") == Some(&"video_mixer".to_owned())
                && entry.value == crate::stats::StatsSnapshotValue::Gauge(240)
        }));
    }

    #[test]
    fn convert_worker_thread_stats_preserves_seconds() {
        let worker = WorkerThreadStats {
            processors: Vec::new(),
            total_processing_duration: crate::stats_legacy::SharedAtomicDuration::new(
                Duration::from_millis(1500),
            ),
            total_waiting_duration: crate::stats_legacy::SharedAtomicDuration::new(
                Duration::from_millis(250),
            ),
        };
        let converted = convert_worker_thread_stats(&[worker]);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].total_processing_seconds, 1.5);
        assert_eq!(converted[0].total_waiting_seconds, 0.25);
    }
}
