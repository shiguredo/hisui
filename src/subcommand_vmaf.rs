use std::{
    collections::HashSet,
    future::Future,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{
    Error, MediaPipeline, Message, ProcessorHandle, ProcessorId, Result, TrackId,
    decoder::{VideoDecoder, VideoDecoderOptions},
    encoder::{VideoEncoder, VideoEncoderOptions},
    json::JsonObject,
    layout::Layout,
    media::{MediaSample, MediaStreamId},
    mixer_video::{VideoMixer, VideoMixerSpec},
    reader::VideoReader,
    types::EngineName,
    video::FrameRate,
    writer_yuv::YuvWriter,
};

const DEFAULT_LAYOUT_JSON: &str = include_str!("../layout-examples/vmaf-default.jsonc");

#[derive(Debug)]
struct Args {
    layout_file_path: Option<PathBuf>,
    reference_yuv_file_path: Option<PathBuf>,
    distorted_yuv_file_path: Option<PathBuf>,
    vmaf_output_file_path: Option<PathBuf>,
    openh264: Option<PathBuf>,
    #[expect(dead_code)]
    max_cpu_cores: Option<NonZeroUsize>,
    frame_count: usize,
    timeout: Option<Duration>,
    root_dir: PathBuf,
}

impl Args {
    fn parse(raw_args: &mut noargs::RawArgs) -> noargs::Result<Self> {
        Ok(Self {
            layout_file_path: noargs::opt("layout-file")
                .short('l')
                .ty("PATH")
                .env("HISUI_LAYOUT_FILE_PATH")
                .default("HISUI_REPO/layout-examples/vmaf-default.jsonc")
                .doc("合成に使用するレイアウトファイルを指定します")
                .take(raw_args)
                .then(crate::arg_utils::parse_non_default_opt)?,
            reference_yuv_file_path: noargs::opt("reference-yuv-file")
                .ty("PATH")
                .default("ROOT_DIR/reference.yuv")
                .doc("参照映像のYUVファイルの出力先を指定します")
                .take(raw_args)
                .then(crate::arg_utils::parse_non_default_opt)?,
            distorted_yuv_file_path: noargs::opt("distorted-yuv-file")
                .ty("PATH")
                .default("ROOT_DIR/distorted.yuv")
                .doc("歪み映像のYUVファイルの出力先を指定します")
                .take(raw_args)
                .then(crate::arg_utils::parse_non_default_opt)?,
            vmaf_output_file_path: noargs::opt("vmaf-output-file")
                .ty("PATH")
                .default("ROOT_DIR/vmaf-output.json")
                .doc("vmaf コマンドの実行結果ファイルの出力先を指定します")
                .take(raw_args)
                .then(crate::arg_utils::parse_non_default_opt)?,
            openh264: noargs::opt("openh264")
                .ty("PATH")
                .env("HISUI_OPENH264_PATH")
                .doc("OpenH264 の共有ライブラリのパスを指定します")
                .take(raw_args)
                .present_and_then(|a| a.value().parse())?,
            max_cpu_cores: noargs::opt("max-cpu-cores")
                .short('c')
                .ty("INTEGER")
                .env("HISUI_MAX_CPU_CORES")
                .doc(concat!(
                    "合成処理を行うプロセスが使用するコア数の上限を指定します\n",
                    "（未指定時には上限なし）\n",
                    "\n",
                    "NOTE: macOS ではこの引数は無視されます",
                ))
                .take(raw_args)
                .present_and_then(|a| a.value().parse())?,
            frame_count: noargs::opt("frame-count")
                .short('f')
                .ty("FRAMES")
                .default("1000")
                .doc("変換するフレーム数を指定します")
                .take(raw_args)
                .then(|a| a.value().parse())?,
            timeout: noargs::opt("timeout")
                .ty("SECONDS")
                .doc("処理のタイムアウト時間（秒）を指定します（超過した場合は失敗扱い）")
                .take(raw_args)
                .present_and_then(|a| a.value().parse::<f32>().map(Duration::from_secs_f32))?,
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

    // 最初に vmaf コマンドが利用可能かどうかをチェックする
    check_vmaf_availability().or_fail()?;

    // レイアウトを準備（音声処理は無効化）
    let mut layout = Layout::from_layout_json_file_or_default(
        args.root_dir.clone(),
        args.layout_file_path.as_deref(),
        DEFAULT_LAYOUT_JSON,
    )
    .or_fail()?;
    layout.audio_source_ids.clear();
    tracing::debug!("layout: {layout:?}");
    layout
        .has_video()
        .or_fail_with(|()| "no video sources".to_owned())?;

    // 必要に応じて openh264 の共有ライブラリを読み込む
    let openh264_lib = if let Some(path) = args.openh264.as_ref().filter(|_| layout.has_video()) {
        Some(Openh264Library::load(path).or_fail()?)
    } else {
        None
    };

    let distorted_yuv_file_path = args
        .distorted_yuv_file_path
        .clone()
        .unwrap_or_else(|| args.root_dir.join("distorted.yuv"));
    let reference_yuv_file_path = args
        .reference_yuv_file_path
        .clone()
        .unwrap_or_else(|| args.root_dir.join("reference.yuv"));

    // 合成処理を実行
    eprintln!("# Compose for VMAF");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .or_fail()?;
    let compose_result = runtime
        .block_on(compose_for_vmaf(
            layout.clone(),
            openh264_lib.clone(),
            args.frame_count,
            args.timeout,
            distorted_yuv_file_path.clone(),
            reference_yuv_file_path.clone(),
        ))
        .map_err(|e| orfail::Failure::new(e.to_string()))?;
    if !compose_result.success {
        return Err(orfail::Failure::new(format!(
            "video composition process failed{}",
            if compose_result.timeout_expired {
                " (timeout)"
            } else {
                ""
            }
        ))
        .into());
    }

    // VMAF の下準備としての処理は全て完了した
    eprintln!("=> done\n");

    // vmaf コマンドを実行
    eprintln!("# Run vmaf command");
    let vmaf_output_file_path = args
        .vmaf_output_file_path
        .unwrap_or_else(|| args.root_dir.join("vmaf-output.json"));
    run_vmaf_evaluation(
        &reference_yuv_file_path,
        &distorted_yuv_file_path,
        &vmaf_output_file_path,
        &layout,
    )
    .or_fail()?;
    eprintln!("=> done\n");

    // VMAF 結果を読み込んで解析
    let vmaf = parse_vmaf_output(&vmaf_output_file_path).or_fail()?;

    // 実行結果の要約を標準出力に出力する
    let output = Output {
        layout_file_path: args.layout_file_path,
        reference_yuv_file_path,
        distorted_yuv_file_path,
        vmaf_output_file_path,
        encode_engine: compose_result.encoder_stats.engine.get().or_fail()?,
        width: layout.resolution.width().get(),
        height: layout.resolution.height().get(),
        frame_rate: layout.frame_rate,
        encoded_frame_count: compose_result
            .encoder_stats
            .total_output_video_frame_count
            .get() as usize,
        elapsed_duration: compose_result.elapsed_duration,
        vmaf,
    };
    println!(
        "{}",
        nojson::json(|f| {
            f.set_indent_size(2);
            f.set_spacing(true);
            f.value(&output)
        })
    );

    Ok(())
}

#[derive(Debug)]
struct ComposeForVmafResult {
    success: bool,
    timeout_expired: bool,
    elapsed_duration: Duration,
    encoder_stats: crate::stats::VideoEncoderStats,
}

#[derive(Debug)]
struct VmafPipelineSetup {
    processor_tasks: Vec<SpawnedProcessorTask>,
    encoder_stats: crate::stats::VideoEncoderStats,
}

#[derive(Debug)]
struct SpawnedProcessorTask {
    processor_id: ProcessorId,
    task: tokio::task::JoinHandle<Result<()>>,
}

async fn compose_for_vmaf(
    layout: Layout,
    openh264_lib: Option<Openh264Library>,
    frame_count: usize,
    timeout: Option<Duration>,
    distorted_yuv_file_path: PathBuf,
    reference_yuv_file_path: PathBuf,
) -> Result<ComposeForVmafResult> {
    let pipeline = MediaPipeline::new()?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());

    let mut setup = match setup_vmaf_pipeline(
        &pipeline_handle,
        layout.clone(),
        openh264_lib,
        frame_count,
        distorted_yuv_file_path,
        reference_yuv_file_path,
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
    let (success, timeout_expired) =
        wait_processor_tasks(&mut setup.processor_tasks, timeout).await;
    let elapsed_duration = start.elapsed();

    shutdown_pipeline(pipeline_handle, pipeline_task).await?;

    Ok(ComposeForVmafResult {
        success,
        timeout_expired,
        elapsed_duration,
        encoder_stats: setup.encoder_stats,
    })
}

async fn setup_vmaf_pipeline(
    pipeline_handle: &crate::MediaPipelineHandle,
    layout: Layout,
    openh264_lib: Option<Openh264Library>,
    frame_count: usize,
    distorted_yuv_file_path: PathBuf,
    reference_yuv_file_path: PathBuf,
) -> Result<VmafPipelineSetup> {
    let mut next_stream_id = MediaStreamId::new(0);
    let mut next_processor_number = 0usize;
    let mut next_track_number = 0usize;
    let mut processor_tasks = Vec::new();

    let decoder_options = VideoDecoderOptions {
        openh264_lib: openh264_lib.clone(),
        decode_params: layout.decode_params.clone(),
        engines: None,
    };
    let video_source_ids = layout.video_source_ids().cloned().collect::<HashSet<_>>();

    let mut mixer_input_stream_ids = Vec::new();
    let mut mixer_input_track_ids = Vec::new();
    for source_info in layout
        .sources
        .iter()
        .filter_map(|(source_id, source_info)| {
            video_source_ids.contains(source_id).then_some(source_info)
        })
    {
        let reader_output_stream_id = next_stream_id.fetch_add(1);
        let reader_output_track_id = next_track_id(&mut next_track_number, "reader_output");
        let reader = VideoReader::from_source_info(reader_output_stream_id, source_info)
            .map_err(error_from)?;
        spawn_processor_task(
            pipeline_handle,
            next_processor_id(&mut next_processor_number, "video_reader"),
            move |handle| async move {
                reader
                    .run(handle)
                    .await
                    .map_err(|e| Error::new(e.to_string()))
            },
            &mut processor_tasks,
        )
        .await?;

        let decoder_output_stream_id = next_stream_id.fetch_add(1);
        let decoder_output_track_id = next_track_id(&mut next_track_number, "decoder_output");
        let decoder = VideoDecoder::new(
            reader_output_stream_id,
            decoder_output_stream_id,
            decoder_options.clone(),
        );
        let decoder_output_track_id_for_decoder = decoder_output_track_id.clone();
        spawn_processor_task(
            pipeline_handle,
            next_processor_id(&mut next_processor_number, "video_decoder"),
            move |handle| {
                decoder.run(
                    handle,
                    reader_output_track_id.clone(),
                    decoder_output_track_id_for_decoder.clone(),
                )
            },
            &mut processor_tasks,
        )
        .await?;

        mixer_input_stream_ids.push(decoder_output_stream_id);
        mixer_input_track_ids.push(decoder_output_track_id);
    }

    let mixer_output_stream_id = next_stream_id.fetch_add(1);
    let mixer_output_track_id = next_track_id(&mut next_track_number, "mixer_output");
    let mixer = VideoMixer::new(
        VideoMixerSpec::from_layout(&layout),
        mixer_input_stream_ids,
        mixer_output_stream_id,
    );
    let mixer_output_track_id_for_mixer = mixer_output_track_id.clone();
    spawn_processor_task(
        pipeline_handle,
        next_processor_id(&mut next_processor_number, "video_mixer"),
        move |handle| {
            mixer.run(
                handle,
                mixer_input_track_ids,
                mixer_output_track_id_for_mixer,
            )
        },
        &mut processor_tasks,
    )
    .await?;

    let limiter_output_track_id = next_track_id(&mut next_track_number, "limiter_output");
    let limiter = FrameCountLimiter::new(frame_count);
    let mixer_output_track_id_for_limiter = mixer_output_track_id.clone();
    let limiter_output_track_id_for_limiter = limiter_output_track_id.clone();
    spawn_processor_task(
        pipeline_handle,
        next_processor_id(&mut next_processor_number, "frame_count_limiter"),
        move |handle| {
            limiter.run(
                handle,
                mixer_output_track_id_for_limiter.clone(),
                limiter_output_track_id_for_limiter.clone(),
            )
        },
        &mut processor_tasks,
    )
    .await?;

    let distorted_writer = YuvWriter::new(&distorted_yuv_file_path)?;
    let limiter_output_track_id_for_distorted_writer = limiter_output_track_id.clone();
    spawn_processor_task(
        pipeline_handle,
        next_processor_id(&mut next_processor_number, "distorted_yuv_writer"),
        move |handle| {
            distorted_writer.run(handle, limiter_output_track_id_for_distorted_writer.clone())
        },
        &mut processor_tasks,
    )
    .await?;

    let limiter_output_stream_id = next_stream_id.fetch_add(1);
    let encoder_output_stream_id = next_stream_id.fetch_add(1);
    let encoder_output_track_id = next_track_id(&mut next_track_number, "encoder_output");
    let encoder = VideoEncoder::new(
        &VideoEncoderOptions::from_layout(&layout),
        limiter_output_stream_id,
        encoder_output_stream_id,
        openh264_lib,
    )
    .map_err(error_from)?;
    let encoder_stats = encoder.encoder_stats().clone();
    let limiter_output_track_id_for_encoder = limiter_output_track_id.clone();
    let encoder_output_track_id_for_encoder = encoder_output_track_id.clone();
    spawn_processor_task(
        pipeline_handle,
        next_processor_id(&mut next_processor_number, "video_encoder"),
        move |handle| {
            encoder.run(
                handle,
                limiter_output_track_id_for_encoder.clone(),
                encoder_output_track_id_for_encoder.clone(),
            )
        },
        &mut processor_tasks,
    )
    .await?;

    let decoder_output_stream_id = next_stream_id.fetch_add(1);
    let decoder_output_track_id = next_track_id(&mut next_track_number, "decoded_output");
    let decoder = VideoDecoder::new(
        encoder_output_stream_id,
        decoder_output_stream_id,
        decoder_options,
    );
    let encoder_output_track_id_for_decoder = encoder_output_track_id.clone();
    let decoder_output_track_id_for_decoder = decoder_output_track_id.clone();
    spawn_processor_task(
        pipeline_handle,
        next_processor_id(&mut next_processor_number, "decoded_video_decoder"),
        move |handle| {
            decoder.run(
                handle,
                encoder_output_track_id_for_decoder.clone(),
                decoder_output_track_id_for_decoder.clone(),
            )
        },
        &mut processor_tasks,
    )
    .await?;

    let reference_writer = YuvWriter::new(&reference_yuv_file_path)?;
    let decoder_output_track_id_for_reference_writer = decoder_output_track_id.clone();
    spawn_processor_task(
        pipeline_handle,
        next_processor_id(&mut next_processor_number, "reference_yuv_writer"),
        move |handle| {
            reference_writer.run(handle, decoder_output_track_id_for_reference_writer.clone())
        },
        &mut processor_tasks,
    )
    .await?;

    let progress = ProgressBar::new(frame_count as u64);
    let decoder_output_track_id_for_progress = decoder_output_track_id.clone();
    spawn_processor_task(
        pipeline_handle,
        next_processor_id(&mut next_processor_number, "progress_bar"),
        move |handle| progress.run(handle, decoder_output_track_id_for_progress.clone()),
        &mut processor_tasks,
    )
    .await?;

    Ok(VmafPipelineSetup {
        processor_tasks,
        encoder_stats,
    })
}

async fn spawn_processor_task<F, T>(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_id: ProcessorId,
    f: F,
    processor_tasks: &mut Vec<SpawnedProcessorTask>,
) -> Result<()>
where
    F: FnOnce(ProcessorHandle) -> T + Send + 'static,
    T: Future<Output = Result<()>> + Send + 'static,
{
    let processor_handle = pipeline_handle
        .register_processor(processor_id.clone())
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
    let task = tokio::spawn(async move { f(processor_handle).await });
    processor_tasks.push(SpawnedProcessorTask { processor_id, task });
    Ok(())
}

async fn wait_processor_tasks(
    processor_tasks: &mut Vec<SpawnedProcessorTask>,
    timeout: Option<Duration>,
) -> (bool, bool) {
    let mut success = true;
    let mut timeout_expired = false;
    let deadline = timeout.map(|timeout| tokio::time::Instant::now() + timeout);

    while let Some(mut processor_task) = processor_tasks.pop() {
        let join_result = if let Some(deadline) = deadline {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                timeout_expired = true;
                processor_task.task.abort();
                for task in processor_tasks.drain(..) {
                    task.task.abort();
                }
                break;
            }

            let timeout = deadline.saturating_duration_since(now);
            match tokio::time::timeout(timeout, &mut processor_task.task).await {
                Ok(result) => result,
                Err(_) => {
                    timeout_expired = true;
                    processor_task.task.abort();
                    for task in processor_tasks.drain(..) {
                        task.task.abort();
                    }
                    break;
                }
            }
        } else {
            processor_task.task.await
        };

        match join_result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                success = false;
                tracing::error!("processor {} failed: {e}", processor_task.processor_id);
            }
            Err(e) => {
                success = false;
                tracing::error!("processor task {} failed: {e}", processor_task.processor_id);
            }
        }
    }

    (success && !timeout_expired, timeout_expired)
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

fn next_processor_id(next_number: &mut usize, prefix: &str) -> ProcessorId {
    let number = *next_number;
    *next_number += 1;
    ProcessorId::new(format!("vmaf_{prefix}_{number}"))
}

fn next_track_id(next_number: &mut usize, prefix: &str) -> TrackId {
    let number = *next_number;
    *next_number += 1;
    TrackId::new(format!("vmaf_{prefix}_{number}"))
}

fn error_from<E: std::fmt::Display>(error: E) -> Error {
    Error::new(error.to_string())
}

pub fn check_vmaf_availability() -> orfail::Result<()> {
    let output = Command::new("vmaf")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output();

    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(_) => Err(orfail::Failure::new(
            "vmaf command failed to execute properly",
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(orfail::Failure::new(
            "vmaf command not found. Please install vmaf and ensure it's in your PATH",
        )),
        Err(e) => Err(orfail::Failure::new(format!(
            "failed to check vmaf availability: {e}"
        ))),
    }
}

fn run_vmaf_evaluation(
    reference_yuv_file_path: &Path,
    distorted_yuv_file_path: &Path,
    vmaf_output_file_path: &Path,
    layout: &Layout,
) -> orfail::Result<()> {
    let output = Command::new("vmaf")
        .args([
            "--reference",
            reference_yuv_file_path.to_str().or_fail()?,
            "--distorted",
            distorted_yuv_file_path.to_str().or_fail()?,
            "--width",
            &layout.resolution.width().get().to_string(),
            "--height",
            &layout.resolution.height().get().to_string(),
            "--output",
            vmaf_output_file_path.to_str().or_fail()?,
            "--json",
            // 以降のパラメータは hisui では固定
            "--pixel_format",
            "420",
            "--bitdepth",
            "8",
        ])
        .stderr(Stdio::inherit())
        .output()
        .or_fail()?;
    output
        .status
        .success()
        .or_fail_with(|()| format!("vmaf failed: {}", String::from_utf8_lossy(&output.stderr)))?;
    Ok(())
}

fn parse_vmaf_output(vmaf_output_file_path: &Path) -> orfail::Result<VmafScoreStats> {
    let vmaf_content = std::fs::read_to_string(vmaf_output_file_path)
        .or_fail_with(|e| format!("failed to read VMAF output file: {e}"))?;
    let json = nojson::RawJson::parse(&vmaf_content).or_fail()?;
    let vmaf_data = JsonObject::new(json.value()).or_fail()?;
    let pooled_metrics = vmaf_data
        .get_required_with("pooled_metrics", JsonObject::new)
        .or_fail()?;
    let vmaf_metrics = pooled_metrics
        .get_required_with("vmaf", JsonObject::new)
        .or_fail()?;
    Ok(VmafScoreStats {
        min: vmaf_metrics.get_required("min").or_fail()?,
        max: vmaf_metrics.get_required("max").or_fail()?,
        mean: vmaf_metrics.get_required("mean").or_fail()?,
        harmonic_mean: vmaf_metrics.get_required("harmonic_mean").or_fail()?,
    })
}

#[derive(Debug)]
struct Output {
    layout_file_path: Option<PathBuf>,
    reference_yuv_file_path: PathBuf,
    distorted_yuv_file_path: PathBuf,
    vmaf_output_file_path: PathBuf,
    encode_engine: EngineName,
    width: usize,
    height: usize,
    frame_rate: FrameRate,
    encoded_frame_count: usize,
    elapsed_duration: Duration,
    vmaf: VmafScoreStats,
}

impl nojson::DisplayJson for Output {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            if let Some(path) = &self.layout_file_path {
                f.member("layout_file_path", path)?;
            }
            f.member("reference_yuv_file_path", &self.reference_yuv_file_path)?;
            f.member("distorted_yuv_file_path", &self.distorted_yuv_file_path)?;
            f.member("vmaf_output_file_path", &self.vmaf_output_file_path)?;
            f.member("encode_engine", self.encode_engine)?;
            f.member("width", self.width)?;
            f.member("height", self.height)?;
            f.member("frame_rate", self.frame_rate)?;
            f.member("encoded_frame_count", self.encoded_frame_count)?;
            f.member("elapsed_seconds", self.elapsed_duration.as_secs_f32())?;
            f.member("vmaf_min", self.vmaf.min)?;
            f.member("vmaf_max", self.vmaf.max)?;
            f.member("vmaf_mean", self.vmaf.mean)?;
            f.member("vmaf_harmonic_mean", self.vmaf.harmonic_mean)?;

            Ok(())
        })
    }
}

#[derive(Debug)]
struct VmafScoreStats {
    min: f64,
    max: f64,
    mean: f64,
    harmonic_mean: f64,
}

// 処理対象のフレーム数を制限するためのプロセッサ
#[derive(Debug)]
struct FrameCountLimiter {
    remaining_frame_count: usize,
}

impl FrameCountLimiter {
    fn new(total_frame_count: usize) -> Self {
        Self {
            remaining_frame_count: total_frame_count,
        }
    }

    async fn run(
        mut self,
        handle: ProcessorHandle,
        input_track_id: TrackId,
        output_track_id: TrackId,
    ) -> Result<()> {
        let mut input_rx = handle.subscribe_track(input_track_id.clone());
        let mut output_tx = handle.publish_track(output_track_id).await?;
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        loop {
            if self.remaining_frame_count == 0 {
                output_tx.send_eos();
                break;
            }

            match input_rx.recv().await {
                Message::Media(MediaSample::Video(frame)) => {
                    self.remaining_frame_count -= 1;
                    if !output_tx.send_media(MediaSample::Video(frame)) {
                        break;
                    }
                }
                Message::Media(MediaSample::Audio(_)) => {
                    return Err(Error::new(format!(
                        "expected a video sample on track {}, but got an audio sample",
                        input_track_id.get()
                    )));
                }
                Message::Eos => {
                    output_tx.send_eos();
                    break;
                }
                Message::Syn(_) => {}
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
struct ProgressBar {
    bar: crate::progress::ProgressBar,
}

impl ProgressBar {
    fn new(total_frame_count: u64) -> Self {
        Self {
            bar: crate::progress::ProgressBar::new(
                total_frame_count,
                crate::progress::ProgressKind::Frame,
            ),
        }
    }

    async fn run(mut self, handle: ProcessorHandle, input_track_id: TrackId) -> Result<()> {
        let mut input_rx = handle.subscribe_track(input_track_id.clone());
        handle.notify_ready();

        loop {
            match input_rx.recv().await {
                Message::Media(MediaSample::Video(_)) => {
                    self.bar.inc(1);
                }
                Message::Media(MediaSample::Audio(_)) => {
                    return Err(Error::new(format!(
                        "expected a video sample on track {}, but got an audio sample",
                        input_track_id.get()
                    )));
                }
                Message::Eos => {
                    self.bar.finish();
                    break;
                }
                Message::Syn(_) => {}
            }
        }

        Ok(())
    }
}
