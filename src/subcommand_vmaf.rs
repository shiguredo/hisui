use std::{
    collections::VecDeque,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{
    decoder::{VideoDecoder, VideoDecoderOptions},
    encoder::{VideoEncoder, VideoEncoderOptions},
    json::JsonObject,
    layout::Layout,
    media::{MediaSample, MediaStreamId},
    mixer_video::{VideoMixer, VideoMixerSpec},
    processor::{
        MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
        MediaProcessorWorkloadHint,
    },
    reader::VideoReader,
    scheduler::Scheduler,
    stats::ProcessorStats,
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
    log::debug!("layout: {layout:?}");
    layout
        .has_video()
        .or_fail_with(|()| "no video sources".to_owned())?;

    // 必要に応じて openh264 の共有ライブラリを読み込む
    let openh264_lib = if let Some(path) = args.openh264.as_ref().filter(|_| layout.has_video()) {
        Some(Openh264Library::load(path).or_fail()?)
    } else {
        None
    };

    // プロセッサを準備
    let mut scheduler = Scheduler::new();
    let mut next_stream_id = MediaStreamId::new(0);

    // リーダーとデコーダーを登録
    let mut mixer_input_stream_ids = Vec::new();
    let decoder_options = VideoDecoderOptions {
        openh264_lib: openh264_lib.clone(),
        decode_params: layout.decode_params.clone(),
    };
    for (source_id, source_info) in &layout.sources {
        if layout.video_source_ids().all(|id| id != source_id) {
            continue;
        }

        let reader_output_stream_id = next_stream_id.fetch_add(1);
        let reader =
            VideoReader::from_source_info(reader_output_stream_id, source_info).or_fail()?;
        scheduler.register(reader).or_fail()?;

        let decoder_output_stream_id = next_stream_id.fetch_add(1);
        let decoder = VideoDecoder::new(
            reader_output_stream_id,
            decoder_output_stream_id,
            decoder_options.clone(),
        );
        scheduler.register(decoder).or_fail()?;

        mixer_input_stream_ids.push(decoder_output_stream_id);
    }

    // ミキサーを登録
    let mixer_output_stream_id = next_stream_id.fetch_add(1);
    let mixer = VideoMixer::new(
        VideoMixerSpec::from_layout(&layout),
        mixer_input_stream_ids,
        mixer_output_stream_id,
    );
    scheduler.register(mixer).or_fail()?;

    // フレーム数を制限する
    let limiter_output_stream_id = next_stream_id.fetch_add(1);
    let limiter = FrameCountLimiter::new(
        mixer_output_stream_id,
        limiter_output_stream_id,
        args.frame_count,
    );
    scheduler.register(limiter).or_fail()?;

    // エンコード前の画像の YUV 書き込みを登録
    let distorted_yuv_file_path = args
        .distorted_yuv_file_path
        .unwrap_or_else(|| args.root_dir.join("distorted.yuv"));
    let writer = YuvWriter::new(limiter_output_stream_id, &distorted_yuv_file_path).or_fail()?;
    scheduler.register(writer).or_fail()?;

    // エンコーダーを登録
    let encoder_output_stream_id = next_stream_id.fetch_add(1);
    let encoder = VideoEncoder::new(
        &VideoEncoderOptions::from_layout(&layout),
        limiter_output_stream_id,
        encoder_output_stream_id,
        openh264_lib.clone(),
    )
    .or_fail()?;
    let encoder_name = encoder.name();
    let encoder_stats = encoder.encoder_stats().clone();
    scheduler.register(encoder).or_fail()?;

    // エンコード後の画像（のデコード結果）の YUV 書き込みを登録
    let decoder_output_stream_id = next_stream_id.fetch_add(1);
    let decoder = VideoDecoder::new(
        encoder_output_stream_id,
        decoder_output_stream_id,
        decoder_options.clone(),
    );
    scheduler.register(decoder).or_fail()?;

    let reference_yuv_file_path = args
        .reference_yuv_file_path
        .unwrap_or_else(|| args.root_dir.join("reference.yuv"));
    let writer = YuvWriter::new(decoder_output_stream_id, &reference_yuv_file_path).or_fail()?;
    scheduler.register(writer).or_fail()?;

    // プログレスバーを準備
    let progress = ProgressBar::new(decoder_output_stream_id, args.frame_count as u64);
    scheduler.register(progress).or_fail()?;

    // 合成処理を実行
    eprintln!("# Compose for VMAF");
    let (timeout_expired, stats) = if let Some(timeout) = args.timeout {
        scheduler.run_timeout(timeout).or_fail()?
    } else {
        (false, scheduler.run().or_fail()?)
    };
    if stats.error.get() {
        return Err(orfail::Failure::new(format!(
            "video composition process failed{}",
            if timeout_expired { " (timeout)" } else { "" }
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
        encoder_name,
        width: layout.resolution.width().get(),
        height: layout.resolution.height().get(),
        frame_rate: layout.frame_rate,
        encoded_frame_count: encoder_stats.total_output_video_frame_count.get() as usize,
        elapsed_duration: stats.elapsed_duration,
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
    encoder_name: EngineName,
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
            f.member("encoder_name", self.encoder_name)?;
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
    input_stream_id: MediaStreamId,
    output_stream_id: MediaStreamId,
    remaining_frame_count: usize,
    queue: VecDeque<MediaSample>,
}

impl FrameCountLimiter {
    fn new(
        input_stream_id: MediaStreamId,
        output_stream_id: MediaStreamId,
        total_frame_count: usize,
    ) -> Self {
        Self {
            input_stream_id,
            output_stream_id,
            remaining_frame_count: total_frame_count,
            queue: VecDeque::new(),
        }
    }
}

impl MediaProcessor for FrameCountLimiter {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: vec![self.input_stream_id],
            output_stream_ids: vec![self.output_stream_id],
            stats: ProcessorStats::other("frame-count-limiter"),
            workload_hint: MediaProcessorWorkloadHint::CPU_MISC,
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        if let Some(sample) = input.sample
            && let Some(n) = self.remaining_frame_count.checked_sub(1)
        {
            self.queue.push_back(sample);
            self.remaining_frame_count = n;
        } else {
            // 指定数フレームを処理した or 入力がEOSに達した
            self.remaining_frame_count = 0;
        };
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        if let Some(sample) = self.queue.pop_front() {
            Ok(MediaProcessorOutput::Processed {
                stream_id: self.output_stream_id,
                sample,
            })
        } else if self.remaining_frame_count == 0 {
            Ok(MediaProcessorOutput::Finished)
        } else {
            Ok(MediaProcessorOutput::pending(self.input_stream_id))
        }
    }
}

#[derive(Debug)]
struct ProgressBar {
    input_stream_id: MediaStreamId,
    eos: bool,
    bar: indicatif::ProgressBar,
}

impl ProgressBar {
    fn new(input_stream_id: MediaStreamId, total_frame_count: u64) -> Self {
        Self {
            input_stream_id,
            eos: false,
            bar: crate::arg_utils::create_frame_progress_bar(total_frame_count),
        }
    }
}

impl MediaProcessor for ProgressBar {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: vec![self.input_stream_id],
            output_stream_ids: Vec::new(),
            stats: ProcessorStats::other("progress_bar"),
            workload_hint: MediaProcessorWorkloadHint::WRITER,
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        if input.sample.is_some() {
            self.bar.inc(1);
        } else {
            self.eos = true;
            self.bar.finish();
        };
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        if self.eos {
            Ok(MediaProcessorOutput::Finished)
        } else {
            Ok(MediaProcessorOutput::pending(self.input_stream_id))
        }
    }
}
