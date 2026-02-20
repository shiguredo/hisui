use std::{
    collections::BTreeSet,
    future::Future,
    num::NonZeroUsize,
    path::PathBuf,
    time::{Duration, Instant},
};

use shiguredo_openh264::Openh264Library;

use crate::{
    Error, MediaPipeline, Message, ProcessorHandle, ProcessorId, ProcessorMetadata, Result,
    TrackId,
    decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions},
    encoder::{AudioEncoder, VideoEncoder, VideoEncoderOptions},
    layout::{DEFAULT_LAYOUT_JSON, Layout},
    mixer_audio::AudioMixer,
    mixer_video::{VideoMixer, VideoMixerSpec},
    reader::{AudioReader, VideoReader},
    stats::{StatsEntry, StatsValue},
    writer_mp4::{Mp4Writer, Mp4WriterOptions},
};

#[derive(Debug)]
struct Args {
    layout_file_path: Option<PathBuf>,
    output_file_path: Option<PathBuf>,
    stats_file_path: Option<PathBuf>,
    openh264: Option<PathBuf>,
    no_progress_bar: bool,
    worker_threads: NonZeroUsize,
    root_dir: PathBuf,
}

impl Args {
    fn parse(raw_args: &mut noargs::RawArgs) -> noargs::Result<Self> {
        Ok(Self {
            layout_file_path: noargs::opt("layout-file")
                .short('l')
                .ty("PATH")
                .env("HISUI_LAYOUT_FILE_PATH")
                .default("HISUI_REPO/layout-examples/compose-default.jsonc")
                .doc("合成に使用するレイアウトファイルを指定します")
                .take(raw_args)
                .then(crate::arg_utils::parse_non_default_opt)?,
            output_file_path: noargs::opt("output-file")
                .short('o')
                .ty("PATH")
                .default("ROOT_DIR/output.mp4")
                .doc("合成結果を保存するファイルを指定します")
                .take(raw_args)
                .then(crate::arg_utils::parse_non_default_opt)?,
            stats_file_path: noargs::opt("stats-file")
                .short('s')
                .ty("PATH")
                .doc("合成中に収集した統計情報 (JSON) を保存するファイルを指定します")
                .take(raw_args)
                .present_and_then(|a| a.value().parse())?,
            openh264: noargs::opt("openh264")
                .ty("PATH")
                .env("HISUI_OPENH264_PATH")
                .doc("OpenH264 の共有ライブラリのパスを指定します")
                .take(raw_args)
                .present_and_then(|a| a.value().parse())?,
            no_progress_bar: noargs::flag("no-progress-bar")
                .short('P')
                .doc("指定された場合は、合成の進捗を非表示にします")
                .take(raw_args)
                .is_present(),
            worker_threads: noargs::opt("thread-count")
                .short('T')
                .ty("INTEGER")
                .default("1")
                .env("HISUI_THREAD_COUNT")
                .doc(concat!(
                    "合成処理に使用するワーカースレッド数を指定します\n",
                    "\n",
                    "なおこれはあくまでも Hisui 自体が起動するスレッドの数であり、\n",
                    "各エンコーダーやデコーダーが内部で起動するスレッドには関与しません",
                ))
                .take(raw_args)
                .then(|a| a.value().parse())?,
            root_dir: noargs::arg("ROOT_DIR")
                .example("/path/to/archive/RECORDING_ID/")
                .doc(concat!(
                    "合成処理を行う際のルートディレクトリを指定します\n",
                    "\n",
                    "レイアウトファイル内に記載された相対パスの基点は、",
                    "このディレクトリとなります。\n",
                    "また、レイアウト内で、",
                    "このディレクトリの外のファイルが参照された場合にはエラーとなります。"
                ))
                .take(raw_args)
                .then(crate::arg_utils::validate_existing_directory_path)?,
        })
    }
}

pub fn run(mut raw_args: noargs::RawArgs) -> noargs::Result<()> {
    let args = Args::parse(&mut raw_args)?;
    if let Some(help) = raw_args.finish()? {
        print!("{help}");
        return Ok(());
    }

    run_internal(args).map_err(noargs::Error::from)
}

fn run_internal(args: Args) -> crate::Result<()> {
    // レイアウトを準備
    let layout = Layout::from_layout_json_file_or_default(
        args.root_dir.clone(),
        args.layout_file_path.as_deref(),
        DEFAULT_LAYOUT_JSON,
    )?;
    tracing::debug!("layout: {layout:?}");

    // 必要に応じて openh264 の共有ライブラリを読み込む
    let openh264_lib = if let Some(path) = args.openh264.as_ref().filter(|_| layout.has_video()) {
        Some(Openh264Library::load(path)?)
    } else {
        None
    };

    // 出力ファイルパスを決定
    let output_file_path = args
        .output_file_path
        .unwrap_or_else(|| args.root_dir.join("output.mp4"));
    let result = run_compose(
        layout,
        openh264_lib,
        !args.no_progress_bar,
        args.worker_threads,
        output_file_path.clone(),
        args.stats_file_path.as_ref(),
    )?;
    let entries = result
        .stats
        .entries()
        .map_err(|e: crate::Error| e.with_context("failed to load compose stats entries"))?;

    if !result.success {
        // エラー発生時は終了コードを変える
        std::process::exit(1);
    }

    crate::json::pretty_print(nojson::json(|f| {
        f.object(|f| {
            if let Some(path) = &args.layout_file_path {
                f.member("layout_file_path", path)?;
            }
            if let Some(path) = &args.stats_file_path {
                f.member("stats_file_path", path)?;
            }
            f.member("input_root_dir", &args.root_dir)?;
            print_input_stats_summary(f, &entries)?;
            f.member("output_file_path", &output_file_path)?;
            print_output_stats_summary(f, &entries)?;
            print_time_stats_summary(f, result.elapsed_duration.as_secs_f64())?;

            Ok(())
        })
    }))?;

    Ok(())
}

#[derive(Debug)]
struct ComposeResult {
    stats: crate::stats::Stats,
    elapsed_duration: Duration,
    success: bool,
}

#[derive(Debug)]
struct ComposePipelineSetup {
    processor_tasks: tokio::task::JoinSet<(ProcessorId, Result<()>)>,
}

fn run_compose(
    layout: Layout,
    openh264_lib: Option<Openh264Library>,
    show_progress_bar: bool,
    worker_threads: NonZeroUsize,
    out_file_path: PathBuf,
    stats_file_path: Option<&PathBuf>,
) -> Result<ComposeResult> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads.get())
        .enable_all()
        .build()
        .map_err(|e| Error::from(e).with_context("failed to build compose runtime"))?;

    let result = runtime.block_on(run_compose_pipeline(
        layout,
        openh264_lib,
        show_progress_bar,
        out_file_path,
    ))?;

    if let Some(path) = stats_file_path {
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
            if let Err(shutdown_error) = shutdown_pipeline(pipeline_handle, pipeline_task).await {
                tracing::warn!(
                    "failed to shutdown compose pipeline after setup failure: {}",
                    shutdown_error.display()
                );
            }
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
    let mut next_processor_index = 0usize;
    let mut next_processor = |processor_type: &'static str| {
        let processor_id = ProcessorId::new(format!("{processor_type}:{next_processor_index}"));
        next_processor_index += 1;
        (processor_id, ProcessorMetadata::new(processor_type))
    };

    let mut processor_tasks = tokio::task::JoinSet::new();

    // リーダーとデコーダーを登録する。
    let mut audio_mixer_input_track_ids = Vec::new();
    for source_id in layout.audio_source_ids() {
        let source_info = layout.sources.get(source_id).ok_or_else(|| {
            Error::new(format!(
                "missing source info for source id: {}",
                source_id.get()
            ))
        })?;

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
                let reader = AudioReader::from_source_info(&source_info, handle.stats())?;
                reader.run(handle).await
            },
            &mut processor_tasks,
        )
        .await?;

        let (decoder_processor_id, decoder_metadata) = next_processor("audio_decoder");
        let decoder_output_track_id = TrackId::new(decoder_processor_id.get());
        let reader_output_track_id_for_decoder = reader_output_track_id.clone();
        let decoder_output_track_id_for_decoder = decoder_output_track_id.clone();
        spawn_processor_task(
            pipeline_handle,
            decoder_processor_id,
            decoder_metadata,
            move |handle| async move {
                let decoder = AudioDecoder::new(handle.stats())?;
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
        audio_mixer_input_track_ids.push(decoder_output_track_id);
    }

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
                let reader = VideoReader::from_source_info(&source_info, handle.stats())?;
                reader.run(handle).await
            },
            &mut processor_tasks,
        )
        .await?;

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
                let decoder = VideoDecoder::new(decoder_options_for_decoder, handle.stats());
                decoder.run(
                    handle,
                    reader_output_track_id_for_decoder.clone(),
                    decoder_output_track_id_for_decoder.clone(),
                )
            },
            &mut processor_tasks,
        )
        .await?;
        video_mixer_input_track_ids.push(decoder_output_track_id);
    }

    // ミキサーを登録する。
    let (audio_mixer_processor_id, audio_mixer_metadata) = next_processor("audio_mixer");
    let audio_mixer_output_track_id = TrackId::new(audio_mixer_processor_id.get());
    let trim_spans_for_audio_mixer = layout.trim_spans.clone();
    let audio_mixer_output_track_id_for_mixer = audio_mixer_output_track_id.clone();
    let audio_mixer_input_track_ids_for_new = audio_mixer_input_track_ids.clone();
    let audio_mixer_input_track_ids_for_run = audio_mixer_input_track_ids;
    spawn_processor_task(
        pipeline_handle,
        audio_mixer_processor_id,
        audio_mixer_metadata,
        move |handle| {
            let mixer = AudioMixer::new(
                trim_spans_for_audio_mixer,
                audio_mixer_input_track_ids_for_new,
                audio_mixer_output_track_id_for_mixer.clone(),
                handle.stats(),
            );
            mixer.run(
                handle,
                audio_mixer_input_track_ids_for_run,
                audio_mixer_output_track_id_for_mixer.clone(),
            )
        },
        &mut processor_tasks,
    )
    .await?;

    let (video_mixer_processor_id, video_mixer_metadata) = next_processor("video_mixer");
    let video_mixer_output_track_id = TrackId::new(video_mixer_processor_id.get());
    let video_mixer_spec = VideoMixerSpec::from_layout(layout);
    let video_mixer_output_track_id_for_mixer = video_mixer_output_track_id.clone();
    let video_mixer_input_track_ids_for_new = video_mixer_input_track_ids.clone();
    let video_mixer_input_track_ids_for_run = video_mixer_input_track_ids;
    spawn_processor_task(
        pipeline_handle,
        video_mixer_processor_id,
        video_mixer_metadata,
        move |handle| {
            let mixer = VideoMixer::new(
                video_mixer_spec,
                video_mixer_input_track_ids_for_new,
                video_mixer_output_track_id_for_mixer.clone(),
                handle.stats(),
            );
            mixer.run(
                handle,
                video_mixer_input_track_ids_for_run,
                video_mixer_output_track_id_for_mixer.clone(),
            )
        },
        &mut processor_tasks,
    )
    .await?;

    // エンコーダーを登録する。
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
            let encoder = AudioEncoder::new(audio_codec, audio_bitrate, handle.stats())?;
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
                writer_input_audio_track_id.clone(),
                writer_input_video_track_id.clone(),
                handle.stats(),
            )?;
            writer
                .run(
                    handle,
                    writer_input_audio_track_id.clone(),
                    writer_input_video_track_id.clone(),
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
            move |handle| {
                run_progress_bar(
                    handle,
                    progress_audio_track_id,
                    progress_video_track_id,
                    output_duration,
                )
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
    processor_tasks: &mut tokio::task::JoinSet<(ProcessorId, Result<()>)>,
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
    let task_processor_id = processor_id.clone();
    processor_tasks.spawn(async move {
        let result = f(processor_handle).await;
        if let Err(e) = &result {
            processor_stats.flag("error").set(true);
            tracing::debug!(
                "processor {} marked as error in stats: {}",
                task_processor_id,
                e.display()
            );
        }
        (task_processor_id, result)
    });

    Ok(())
}

async fn wait_processor_tasks(
    processor_tasks: &mut tokio::task::JoinSet<(ProcessorId, Result<()>)>,
) -> bool {
    let mut success = true;
    while let Some(join_result) = processor_tasks.join_next().await {
        match join_result {
            Ok((_processor_id, Ok(()))) => {}
            Ok((processor_id, Err(e))) => {
                success = false;
                tracing::error!("processor {} failed: {}", processor_id, e.display());
            }
            Err(e) => {
                success = false;
                tracing::error!("processor task join failed: {e}");
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
    const SHUTDOWN_TIMEOUT_SECONDS: u64 = 5;
    drop(pipeline_handle);
    match tokio::time::timeout(
        Duration::from_secs(SHUTDOWN_TIMEOUT_SECONDS),
        &mut pipeline_task,
    )
    .await
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(Error::new(format!("media pipeline task failed: {e}"))),
        Err(_) => {
            tracing::warn!(
                "compose pipeline shutdown timed out after {} seconds; aborting pipeline task",
                SHUTDOWN_TIMEOUT_SECONDS
            );
            pipeline_task.abort();
            if let Err(e) = pipeline_task.await
                && !e.is_cancelled()
            {
                tracing::warn!("pipeline task join after abort failed: {e}");
            }
            Ok(())
        }
    }
}

fn print_input_stats_summary(
    f: &mut nojson::JsonObjectFormatter<'_, '_, '_>,
    entries: &[StatsEntry],
) -> std::fmt::Result {
    // NOTE: 個別の reader / decoder の情報を出すと JSON の要素数が可変かつ挙動になる可能性があるので省く
    //（その情報が必要なら stats ファイルを出力して、そっちを参照するのがいい）
    let count = count_processors_by_types(entries, &["mp4_audio_reader", "webm_audio_reader"]);
    if count > 0 {
        f.member("input_audio_source_count", count)?;
    }

    let count = count_processors_by_types(entries, &["mp4_video_reader", "webm_video_reader"]);
    if count > 0 {
        f.member("input_video_source_count", count)?;
    }

    Ok(())
}

fn print_output_stats_summary(
    f: &mut nojson::JsonObjectFormatter<'_, '_, '_>,
    entries: &[StatsEntry],
) -> std::fmt::Result {
    let Some(writer_id) = find_first_processor_id_by_type(entries, "mp4_writer") else {
        return Ok(());
    };

    if let Some(codec) = find_string_metric_by_processor(entries, &writer_id, "audio_codec") {
        f.member("output_audio_codec", codec)?;
        if let Some(engine) = find_first_string_metric_by_type(entries, "audio_encoder", "engine") {
            f.member("output_audio_encode_engine", engine)?;
        }
        if let Some(duration_seconds) =
            find_numeric_metric_by_processor(entries, &writer_id, "total_audio_track_seconds")
        {
            f.member("output_audio_duration_seconds", duration_seconds)?;
            if duration_seconds > 0.0
                && let Some(byte_size) = find_numeric_metric_by_processor(
                    entries,
                    &writer_id,
                    "total_audio_sample_data_byte_size",
                )
            {
                let bitrate = (byte_size * 8.0) / duration_seconds;
                f.member("output_audio_bitrate", bitrate as u64)?;
            }
        }
    }
    if let Some(codec) = find_string_metric_by_processor(entries, &writer_id, "video_codec") {
        f.member("output_video_codec", codec)?;
        if let Some(engine) = find_first_string_metric_by_type(entries, "video_encoder", "engine") {
            f.member("output_video_encode_engine", engine)?;
        }
        if let Some(duration_seconds) =
            find_numeric_metric_by_processor(entries, &writer_id, "total_video_track_seconds")
        {
            f.member("output_video_duration_seconds", duration_seconds)?;
            if duration_seconds > 0.0
                && let Some(byte_size) = find_numeric_metric_by_processor(
                    entries,
                    &writer_id,
                    "total_video_sample_data_byte_size",
                )
            {
                let bitrate = (byte_size * 8.0) / duration_seconds;
                f.member("output_video_bitrate", bitrate as u64)?;
            }
        }
    }

    if let Some(width) =
        find_first_numeric_metric_by_type(entries, "video_mixer", "output_video_width")
    {
        f.member("output_video_width", width as usize)?;
    }
    if let Some(height) =
        find_first_numeric_metric_by_type(entries, "video_mixer", "output_video_height")
    {
        f.member("output_video_height", height as usize)?;
    }

    Ok(())
}

fn print_time_stats_summary(
    f: &mut nojson::JsonObjectFormatter<'_, '_, '_>,
    elapsed_seconds: f64,
) -> std::fmt::Result {
    f.member("elapsed_seconds", elapsed_seconds)?;

    Ok(())
}

fn count_processors_by_types(entries: &[StatsEntry], processor_types: &[&str]) -> usize {
    let mut processor_ids = BTreeSet::new();
    for entry in entries {
        // 1 つの processor は複数の metric を出すため、
        // processor_id / processor_type label が付いた entry から
        // processor_id を一意化して processor 数を求める。
        let Some(processor_id) = label_value_non_empty(entry, "processor_id") else {
            continue;
        };
        let Some(processor_type) = label_value_non_empty(entry, "processor_type") else {
            continue;
        };
        if !processor_types.iter().any(|t| t == &processor_type) {
            continue;
        }
        processor_ids.insert(processor_id.to_owned());
    }
    processor_ids.len()
}

fn label_value<'a>(entry: &'a StatsEntry, name: &str) -> Option<&'a str> {
    entry.labels.get(name).map(String::as_str)
}

// 空文字を欠損扱いにしたいラベル用
fn label_value_non_empty<'a>(entry: &'a StatsEntry, name: &str) -> Option<&'a str> {
    label_value(entry, name).filter(|value| !value.is_empty())
}

fn find_first_processor_id_by_type(entries: &[StatsEntry], processor_type: &str) -> Option<String> {
    entries.iter().find_map(|entry| {
        if label_value_non_empty(entry, "processor_type") != Some(processor_type) {
            return None;
        }
        label_value_non_empty(entry, "processor_id").map(ToOwned::to_owned)
    })
}

fn find_string_metric_by_processor(
    entries: &[StatsEntry],
    processor_id: &str,
    metric_name: &str,
) -> Option<String> {
    find_metric(
        entries,
        "processor_id",
        processor_id,
        metric_name,
        |value| value.as_string().filter(|s| !s.is_empty()),
    )
}

fn find_numeric_metric_by_processor(
    entries: &[StatsEntry],
    processor_id: &str,
    metric_name: &str,
) -> Option<f64> {
    find_metric(
        entries,
        "processor_id",
        processor_id,
        metric_name,
        |value| value.as_numeric_f64(),
    )
}

fn find_first_string_metric_by_type(
    entries: &[StatsEntry],
    processor_type: &str,
    metric_name: &str,
) -> Option<String> {
    find_metric(
        entries,
        "processor_type",
        processor_type,
        metric_name,
        |value| value.as_string().filter(|s| !s.is_empty()),
    )
}

fn find_first_numeric_metric_by_type(
    entries: &[StatsEntry],
    processor_type: &str,
    metric_name: &str,
) -> Option<f64> {
    find_metric(
        entries,
        "processor_type",
        processor_type,
        metric_name,
        |value| value.as_numeric_f64(),
    )
}

fn find_metric<T>(
    entries: &[StatsEntry],
    label_name: &str,
    label_value_to_match: &str,
    metric_name: &str,
    extract: impl Fn(&StatsValue) -> Option<T>,
) -> Option<T> {
    entries.iter().find_map(|entry| {
        if entry.metric_name != metric_name {
            return None;
        }
        if label_value_non_empty(entry, label_name) != Some(label_value_to_match) {
            return None;
        }
        extract(&entry.value)
    })
}
