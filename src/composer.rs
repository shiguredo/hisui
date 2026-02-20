use std::{
    future::Future,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use shiguredo_openh264::Openh264Library;

use crate::{
    Error, MediaPipeline, Message, ProcessorHandle, ProcessorId, ProcessorMetadata, Result,
    TrackId,
    decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions},
    encoder::{AudioEncoder, VideoEncoder, VideoEncoderOptions},
    layout::Layout,
    media::MediaStreamId,
    mixer_audio::AudioMixer,
    mixer_video::{VideoMixer, VideoMixerSpec},
    reader::{AudioReader, VideoReader},
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

#[derive(Debug)]
struct ComposePipelineSetup {
    processor_tasks: Vec<SpawnedProcessorTask>,
}

#[derive(Debug)]
struct SpawnedProcessorTask {
    processor_id: ProcessorId,
    task: tokio::task::JoinHandle<Result<()>>,
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

    pub fn compose(&self, out_file_path: &Path) -> Result<ComposeResult> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(self.worker_threads.get())
            .enable_all()
            .build()
            .map_err(|e| Error::from(e).with_context("failed to build compose runtime"))?;

        let result = runtime.block_on(run_compose_pipeline(
            self.layout.clone(),
            self.openh264_lib.clone(),
            self.show_progress_bar,
            out_file_path.to_path_buf(),
        ))?;

        if let Some(path) = &self.stats_file_path {
            // TODO: compose 実行基盤を tokio ランタイムへ移行した後に、
            // `tokio::runtime::Handle::current().metrics()` を収集して
            // stats JSON の `tokio_metrics` へ反映する。
            match crate::stats_legacy_json::to_legacy_stats_json(
                &result.stats,
                result.elapsed_duration.as_secs_f64(),
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
                        "failed to build stats JSON: path={}, reason={}",
                        path.display(),
                        e.display()
                    );
                }
            }
        }

        Ok(result)
    }
}

async fn run_compose_pipeline(
    layout: Layout,
    openh264_lib: Option<Openh264Library>,
    show_progress_bar: bool,
    out_file_path: PathBuf,
) -> Result<ComposeResult> {
    let pipeline = MediaPipeline::new()?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());

    let mut setup = match setup_pipeline(
        &pipeline_handle,
        &layout,
        openh264_lib,
        show_progress_bar,
        out_file_path,
    )
    .await
    {
        Ok(setup) => setup,
        Err(e) => {
            let _ = shutdown_pipeline(pipeline_handle, pipeline_task).await;
            return Err(e);
        }
    };

    pipeline_handle.complete_initial_processor_registration();

    let start = Instant::now();
    let task_success = wait_processor_tasks(&mut setup.processor_tasks).await;
    let elapsed_duration = start.elapsed();

    let stats = pipeline_handle.stats();
    let metric_success = !has_processor_error_metric(&stats)?;

    shutdown_pipeline(pipeline_handle, pipeline_task).await?;

    Ok(ComposeResult {
        stats,
        elapsed_duration,
        success: task_success && metric_success,
    })
}

async fn setup_pipeline(
    pipeline_handle: &crate::MediaPipelineHandle,
    layout: &Layout,
    openh264_lib: Option<Openh264Library>,
    show_progress_bar: bool,
    out_file_path: PathBuf,
) -> Result<ComposePipelineSetup> {
    let mut next_stream_id = MediaStreamId::new(0);
    let mut next_processor_index = 0usize;
    let mut next_processor = |processor_type: &'static str| {
        let processor_id = ProcessorId::new(format!("{processor_type}:{next_processor_index}"));
        next_processor_index += 1;
        (processor_id, ProcessorMetadata::new(processor_type))
    };

    let mut processor_tasks = Vec::new();

    // リーダーとデコーダーを登録する。
    let mut audio_mixer_input_stream_ids = Vec::new();
    let mut audio_mixer_input_track_ids = Vec::new();
    for source_id in layout.audio_source_ids() {
        let source_info = layout.sources.get(source_id).ok_or_else(|| {
            Error::new(format!(
                "missing source info for source id: {}",
                source_id.get()
            ))
        })?;

        let reader_output_stream_id = next_stream_id.fetch_add(1);
        let source_info = source_info.clone();
        let reader_processor_type = match source_info.format {
            crate::metadata::ContainerFormat::Mp4 => "mp4_audio_reader",
            crate::metadata::ContainerFormat::Webm => "webm_audio_reader",
        };
        let (reader_processor_id, reader_metadata) = next_processor(reader_processor_type);
        let reader_output_track_id = TrackId::new(reader_processor_id.get());
        spawn_processor_task(
            pipeline_handle,
            reader_processor_id,
            reader_metadata,
            move |handle| async move {
                let reader = AudioReader::from_source_info(
                    reader_output_stream_id,
                    &source_info,
                    handle.stats(),
                )?;
                reader.run(handle).await
            },
            &mut processor_tasks,
        )
        .await?;

        let decoder_output_stream_id = next_stream_id.fetch_add(1);
        let (decoder_processor_id, decoder_metadata) = next_processor("audio_decoder");
        let decoder_output_track_id = TrackId::new(decoder_processor_id.get());
        let reader_output_track_id_for_decoder = reader_output_track_id.clone();
        let decoder_output_track_id_for_decoder = decoder_output_track_id.clone();
        spawn_processor_task(
            pipeline_handle,
            decoder_processor_id,
            decoder_metadata,
            move |handle| async move {
                let decoder = AudioDecoder::new(
                    reader_output_stream_id,
                    decoder_output_stream_id,
                    handle.stats(),
                )?;
                decoder
                    .run(
                        handle,
                        reader_output_track_id_for_decoder.clone(),
                        decoder_output_track_id_for_decoder.clone(),
                    )
                    .await
            },
            &mut processor_tasks,
        )
        .await?;
        audio_mixer_input_stream_ids.push(decoder_output_stream_id);
        audio_mixer_input_track_ids.push(decoder_output_track_id);
    }

    let mut video_mixer_input_stream_ids = Vec::new();
    let mut video_mixer_input_track_ids = Vec::new();
    let decoder_options = VideoDecoderOptions {
        openh264_lib: openh264_lib.clone(),
        decode_params: layout.decode_params.clone(),
        engines: layout.video_decode_engines.clone(),
    };
    for source_id in layout.video_source_ids() {
        let source_info = layout.sources.get(source_id).ok_or_else(|| {
            Error::new(format!(
                "missing source info for source id: {}",
                source_id.get()
            ))
        })?;

        let reader_output_stream_id = next_stream_id.fetch_add(1);
        let source_info = source_info.clone();
        let reader_processor_type = match source_info.format {
            crate::metadata::ContainerFormat::Mp4 => "mp4_video_reader",
            crate::metadata::ContainerFormat::Webm => "webm_video_reader",
        };
        let (reader_processor_id, reader_metadata) = next_processor(reader_processor_type);
        let reader_output_track_id = TrackId::new(reader_processor_id.get());
        spawn_processor_task(
            pipeline_handle,
            reader_processor_id,
            reader_metadata,
            move |handle| async move {
                let reader = VideoReader::from_source_info(
                    reader_output_stream_id,
                    &source_info,
                    handle.stats(),
                )?;
                reader.run(handle).await
            },
            &mut processor_tasks,
        )
        .await?;

        let decoder_output_stream_id = next_stream_id.fetch_add(1);
        let (decoder_processor_id, decoder_metadata) = next_processor("video_decoder");
        let decoder_output_track_id = TrackId::new(decoder_processor_id.get());
        let reader_output_track_id_for_decoder = reader_output_track_id.clone();
        let decoder_output_track_id_for_decoder = decoder_output_track_id.clone();
        let decoder_options_for_decoder = decoder_options.clone();
        spawn_processor_task(
            pipeline_handle,
            decoder_processor_id,
            decoder_metadata,
            move |handle| {
                let decoder = VideoDecoder::new(
                    reader_output_stream_id,
                    decoder_output_stream_id,
                    decoder_options_for_decoder,
                    handle.stats(),
                );
                decoder.run(
                    handle,
                    reader_output_track_id_for_decoder.clone(),
                    decoder_output_track_id_for_decoder.clone(),
                )
            },
            &mut processor_tasks,
        )
        .await?;
        video_mixer_input_stream_ids.push(decoder_output_stream_id);
        video_mixer_input_track_ids.push(decoder_output_track_id);
    }

    // ミキサーを登録する。
    let audio_mixer_output_stream_id = next_stream_id.fetch_add(1);
    let (audio_mixer_processor_id, audio_mixer_metadata) = next_processor("audio_mixer");
    let audio_mixer_output_track_id = TrackId::new(audio_mixer_processor_id.get());
    let trim_spans_for_audio_mixer = layout.trim_spans.clone();
    let audio_mixer_output_track_id_for_mixer = audio_mixer_output_track_id.clone();
    spawn_processor_task(
        pipeline_handle,
        audio_mixer_processor_id,
        audio_mixer_metadata,
        move |handle| {
            let mixer = AudioMixer::new(
                trim_spans_for_audio_mixer,
                audio_mixer_input_stream_ids,
                audio_mixer_output_stream_id,
                handle.stats(),
            );
            mixer.run(
                handle,
                audio_mixer_input_track_ids,
                audio_mixer_output_track_id_for_mixer.clone(),
            )
        },
        &mut processor_tasks,
    )
    .await?;

    let video_mixer_output_stream_id = next_stream_id.fetch_add(1);
    let (video_mixer_processor_id, video_mixer_metadata) = next_processor("video_mixer");
    let video_mixer_output_track_id = TrackId::new(video_mixer_processor_id.get());
    let video_mixer_spec = VideoMixerSpec::from_layout(layout);
    let video_mixer_output_track_id_for_mixer = video_mixer_output_track_id.clone();
    spawn_processor_task(
        pipeline_handle,
        video_mixer_processor_id,
        video_mixer_metadata,
        move |handle| {
            let mixer = VideoMixer::new(
                video_mixer_spec,
                video_mixer_input_stream_ids,
                video_mixer_output_stream_id,
                handle.stats(),
            );
            mixer.run(
                handle,
                video_mixer_input_track_ids,
                video_mixer_output_track_id_for_mixer.clone(),
            )
        },
        &mut processor_tasks,
    )
    .await?;

    // エンコーダーを登録する。
    let audio_encoder_output_stream_id = next_stream_id.fetch_add(1);
    let (audio_encoder_processor_id, audio_encoder_metadata) = next_processor("audio_encoder");
    let audio_encoder_output_track_id = TrackId::new(audio_encoder_processor_id.get());
    let audio_codec = layout.audio_codec;
    let audio_bitrate = layout.audio_bitrate_bps();
    let audio_mixer_output_track_id_for_encoder = audio_mixer_output_track_id.clone();
    let audio_encoder_output_track_id_for_encoder = audio_encoder_output_track_id.clone();
    spawn_processor_task(
        pipeline_handle,
        audio_encoder_processor_id,
        audio_encoder_metadata,
        move |handle| async move {
            let encoder = AudioEncoder::new(
                audio_codec,
                audio_bitrate,
                audio_mixer_output_stream_id,
                audio_encoder_output_stream_id,
                handle.stats(),
            )?;
            encoder
                .run(
                    handle,
                    audio_mixer_output_track_id_for_encoder.clone(),
                    audio_encoder_output_track_id_for_encoder.clone(),
                )
                .await
        },
        &mut processor_tasks,
    )
    .await?;

    let video_encoder_output_stream_id = next_stream_id.fetch_add(1);
    let (video_encoder_processor_id, video_encoder_metadata) = next_processor("video_encoder");
    let video_encoder_output_track_id = TrackId::new(video_encoder_processor_id.get());
    let video_encoder_options = VideoEncoderOptions::from_layout(layout);
    let openh264_lib_for_encoder = openh264_lib;
    let video_mixer_output_track_id_for_encoder = video_mixer_output_track_id.clone();
    let video_encoder_output_track_id_for_encoder = video_encoder_output_track_id.clone();
    spawn_processor_task(
        pipeline_handle,
        video_encoder_processor_id,
        video_encoder_metadata,
        move |handle| async move {
            let encoder = VideoEncoder::new(
                &video_encoder_options,
                video_mixer_output_stream_id,
                video_encoder_output_stream_id,
                openh264_lib_for_encoder,
                handle.stats(),
            )?;
            encoder
                .run(
                    handle,
                    video_mixer_output_track_id_for_encoder.clone(),
                    video_encoder_output_track_id_for_encoder.clone(),
                )
                .await
        },
        &mut processor_tasks,
    )
    .await?;

    // ライターを登録する。
    let (writer_processor_id, writer_metadata) = next_processor("mp4_writer");
    let writer_options = Mp4WriterOptions::from_layout(layout);
    let writer_input_audio_stream_id = layout.has_audio().then_some(audio_encoder_output_stream_id);
    let writer_input_video_stream_id = layout.has_video().then_some(video_encoder_output_stream_id);
    let writer_input_audio_track_id = layout
        .has_audio()
        .then_some(audio_encoder_output_track_id.clone());
    let writer_input_video_track_id = layout
        .has_video()
        .then_some(video_encoder_output_track_id.clone());
    spawn_processor_task(
        pipeline_handle,
        writer_processor_id,
        writer_metadata,
        move |handle| async move {
            let writer = Mp4Writer::new(
                &out_file_path,
                Some(writer_options),
                writer_input_audio_stream_id,
                writer_input_video_stream_id,
                handle.stats(),
            )?;
            writer
                .run(
                    handle,
                    writer_input_audio_track_id,
                    writer_input_video_track_id,
                )
                .await
        },
        &mut processor_tasks,
    )
    .await?;

    // プログレスバーを登録する。
    if show_progress_bar {
        let (progress_processor_id, progress_metadata) = next_processor("progress_bar");
        let progress_audio_track_id = audio_encoder_output_track_id;
        let progress_video_track_id = video_encoder_output_track_id;
        let output_duration = layout.output_duration();
        spawn_processor_task(
            pipeline_handle,
            progress_processor_id,
            progress_metadata,
            move |handle| async move {
                run_progress_bar(
                    handle,
                    progress_audio_track_id,
                    progress_video_track_id,
                    output_duration,
                )
                .await
            },
            &mut processor_tasks,
        )
        .await?;
    }

    Ok(ComposePipelineSetup { processor_tasks })
}

async fn run_progress_bar(
    handle: ProcessorHandle,
    audio_track_id: TrackId,
    video_track_id: TrackId,
    output_duration: Duration,
) -> Result<()> {
    let mut audio_rx = Some(handle.subscribe_track(audio_track_id));
    let mut video_rx = Some(handle.subscribe_track(video_track_id));
    let mut bar = crate::progress::ProgressBar::new(
        output_duration.as_secs(),
        crate::progress::ProgressKind::Time,
    );
    let mut max_timestamp = Duration::ZERO;

    handle.notify_ready();
    handle.wait_subscribers_ready().await?;

    while audio_rx.is_some() || video_rx.is_some() {
        tokio::select! {
            message = crate::future::recv_or_pending(&mut audio_rx) => {
                handle_progress_message(message, &mut audio_rx, &mut bar, &mut max_timestamp);
            }
            message = crate::future::recv_or_pending(&mut video_rx) => {
                handle_progress_message(message, &mut video_rx, &mut bar, &mut max_timestamp);
            }
        }
    }

    bar.finish();

    Ok(())
}

fn handle_progress_message(
    message: Message,
    rx: &mut Option<crate::MessageReceiver>,
    bar: &mut crate::progress::ProgressBar,
    max_timestamp: &mut Duration,
) {
    match message {
        Message::Media(sample) => {
            *max_timestamp = (*max_timestamp).max(sample.timestamp());
            bar.set_position(max_timestamp.as_secs());
        }
        Message::Eos => {
            *rx = None;
        }
        Message::Syn(_) => {}
    }
}

async fn spawn_processor_task<F, T>(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_id: ProcessorId,
    processor_metadata: ProcessorMetadata,
    f: F,
    processor_tasks: &mut Vec<SpawnedProcessorTask>,
) -> Result<()>
where
    F: FnOnce(ProcessorHandle) -> T + Send + 'static,
    T: Future<Output = Result<()>> + Send + 'static,
{
    let processor_handle = pipeline_handle
        .register_processor(processor_id.clone(), processor_metadata)
        .await
        .map_err(|e| match e {
            crate::RegisterProcessorError::PipelineTerminated => {
                Error::new("failed to register processor: pipeline has terminated")
            }
            crate::RegisterProcessorError::DuplicateProcessorId => Error::new(format!(
                "processor ID already exists: {}",
                processor_id.get()
            )),
        })?;

    let mut processor_stats = processor_handle.stats();
    let task = tokio::spawn(async move {
        if let Err(e) = f(processor_handle).await {
            processor_stats.flag("error").set(true);
            return Err(e);
        }
        Ok(())
    });
    processor_tasks.push(SpawnedProcessorTask { processor_id, task });
    Ok(())
}

async fn wait_processor_tasks(processor_tasks: &mut Vec<SpawnedProcessorTask>) -> bool {
    let mut success = true;
    while let Some(processor_task) = processor_tasks.pop() {
        match processor_task.task.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                success = false;
                tracing::error!(
                    "processor {} failed: {}",
                    processor_task.processor_id,
                    e.display()
                );
            }
            Err(e) => {
                success = false;
                tracing::error!("processor task {} failed: {e}", processor_task.processor_id);
            }
        }
    }
    success
}

fn has_processor_error_metric(stats: &crate::stats::Stats) -> Result<bool> {
    Ok(stats
        .entries()?
        .into_iter()
        .any(|entry| entry.metric_name == "error" && entry.value.as_flag() == Some(true)))
}

async fn shutdown_pipeline(
    pipeline_handle: crate::MediaPipelineHandle,
    mut pipeline_task: tokio::task::JoinHandle<()>,
) -> Result<()> {
    drop(pipeline_handle);
    match tokio::time::timeout(Duration::from_secs(5), &mut pipeline_task).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(Error::new(format!("media pipeline task failed: {e}"))),
        Err(_) => {
            pipeline_task.abort();
            let _ = pipeline_task.await;
            Ok(())
        }
    }
}
