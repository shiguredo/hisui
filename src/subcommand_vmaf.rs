use std::{
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{
    channel::ErrorFlag,
    composer,
    decoder::{VideoDecoder, VideoDecoderOptions},
    encoder::{VideoEncoder, VideoEncoderThread},
    json::JsonObject,
    layout::Layout,
    media::MediaStreamId,
    mixer_video::VideoMixerThread,
    stats::{Seconds, SharedStats},
    types::EngineName,
    video::FrameRate,
    writer_yuv::YuvWriter,
};

const DEFAULT_LAYOUT_JSON: &str = include_str!("../layout-examples/vmaf-default.json");

#[derive(Debug)]
struct Args {
    layout_file_path: Option<PathBuf>,
    reference_yuv_file_path: PathBuf,
    distorted_yuv_file_path: PathBuf,
    vmaf_output_file_path: PathBuf,
    openh264: Option<PathBuf>,
    max_cpu_cores: Option<NonZeroUsize>,
    frame_count: usize,
    root_dir: PathBuf,
}

impl Args {
    fn parse(raw_args: &mut noargs::RawArgs) -> noargs::Result<Self> {
        Ok(Self {
            layout_file_path: noargs::opt("layout-file")
                .short('l')
                .ty("PATH")
                .env("HISUI_LAYOUT_FILE_PATH")
                .doc(concat!(
                    "合成に使用するレイアウトファイルを指定します\n",
                    "\n",
                    "省略された場合には ",
                    "hisui/layout-examples/vmaf-default.json の内容が使用されます",
                ))
                .take(raw_args)
                .present_and_then(|a| a.value().parse())?,
            reference_yuv_file_path: noargs::opt("reference-yuv-file")
                .ty("PATH")
                .default("reference.yuv")
                .doc(concat!(
                    "参照映像（合成前）のYUVファイルの出力先を指定します\n",
                    "\n",
                    "相対パスの場合は ROOT_DIR が起点となります"
                ))
                .take(raw_args)
                .then(|a| a.value().parse())?,
            distorted_yuv_file_path: noargs::opt("distorted-yuv-file")
                .ty("PATH")
                .default("distorted.yuv")
                .doc(concat!(
                    "歪み映像（合成後）のYUVファイルの出力先を指定します\n",
                    "\n",
                    "相対パスの場合は ROOT_DIR が起点となります"
                ))
                .take(raw_args)
                .then(|a| a.value().parse())?,
            vmaf_output_file_path: noargs::opt("vmaf-output-file")
                .ty("PATH")
                .default("vmaf-output.json")
                .doc(concat!(
                    "vmaf コマンドの実行結果ファイルの出力先を指定します\n",
                    "\n",
                    "相対パスの場合は ROOT_DIR が起点となります"
                ))
                .take(raw_args)
                .then(|a| a.value().parse())?,
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

    // CPU コア数制限を適用
    crate::arg_utils::maybe_limit_cpu_cores(args.max_cpu_cores).or_fail()?;

    // 統計情報の準備（実際にファイル出力するかどうかに関わらず、集計自体は常に行う）
    let stats = SharedStats::new();
    let start_time = Instant::now();

    // 映像ソースの準備（音声の方は使わないので単に無視する）
    let error_flag = ErrorFlag::new();
    let (_audio_source_rxs, video_source_rxs) = composer::create_audio_and_video_sources(
        &layout,
        error_flag.clone(),
        stats.clone(),
        openh264_lib.clone(),
    )
    .or_fail()?;

    // プログレスバーを準備
    let progress_bar = crate::arg_utils::create_frame_progress_bar(true, args.frame_count as u64);

    // ミキサースレッドを起動
    let mut mixed_video_rx = VideoMixerThread::start(
        error_flag.clone(),
        layout.clone(),
        video_source_rxs,
        stats.clone(),
    );

    // エンコード前の画像の書き込みスレッドを起動
    let reference_yuv_file_path = args.root_dir.join(&args.reference_yuv_file_path);
    let mut reference_yuv_writer = YuvWriter::new(&reference_yuv_file_path).or_fail()?;
    let (mixed_video_temp_tx, mixed_video_temp_rx) = crate::channel::sync_channel();
    std::thread::spawn(move || {
        let mut count = 0;
        while let Some(frame) = mixed_video_rx.recv() {
            if count < args.frame_count
                && let Err(e) = reference_yuv_writer.append(&frame).or_fail()
            {
                log::error!("failed to write reference YUV frame: {e}");
                break;
            }
            if !mixed_video_temp_tx.send(frame) {
                break;
            }
            count += 1;
        }
    });

    // 映像エンコードスレッドを起動
    let encoder = VideoEncoder::new(&layout, openh264_lib.clone()).or_fail()?;
    let encoder_name = encoder.name();
    let mut encoded_video_rx = VideoEncoderThread::start(
        error_flag.clone(),
        mixed_video_temp_rx,
        encoder,
        stats.clone(),
    );

    // 最終的な映像のデコード＆ YUV 書き出しの準備
    let options = VideoDecoderOptions {
        openh264_lib: openh264_lib.clone(),
    };
    let dummy_stream_id = MediaStreamId::new(0);
    let mut decoder = VideoDecoder::new(dummy_stream_id, dummy_stream_id, options);
    let distorted_yuv_file_path = args.root_dir.join(&args.distorted_yuv_file_path);
    let mut distorted_yuv_writer = YuvWriter::new(&distorted_yuv_file_path).or_fail()?;

    // 必要なフレームの処理が終わるまでループを回す
    eprintln!("# Compose for VMAF");
    let mut encoded_byte_size = 0;
    let mut encoded_duration = Duration::ZERO;
    let mut encoded_frame_count = 0;
    while encoded_frame_count < args.frame_count {
        let Some(encoded_frame) = encoded_video_rx.recv() else {
            // 合成フレームの総数が frame_count よりも少なかった場合にここに来る
            decoder.finish().or_fail()?;
            while let Some(decoded_frame) = decoder.next_decoded_frame() {
                distorted_yuv_writer.append(&decoded_frame).or_fail()?;
                progress_bar.inc(1);
                encoded_frame_count += 1;
                if encoded_frame_count >= args.frame_count {
                    break;
                }
            }
            break;
        };
        encoded_byte_size += encoded_frame.data.len() as u64;
        encoded_duration += encoded_frame.duration;
        decoder.decode(encoded_frame).or_fail()?;
        while let Some(decoded_frame) = decoder.next_decoded_frame() {
            distorted_yuv_writer.append(&decoded_frame).or_fail()?;
            progress_bar.inc(1);
            encoded_frame_count += 1;
            if encoded_frame_count >= args.frame_count {
                break;
            }
        }

        if error_flag.get() {
            // ファイル読み込み、デコード、合成、エンコード、のいずれかで失敗したものがあるとここに来る
            log::error!("The composition process was aborted");
            break;
        }
    }
    (encoded_frame_count > 0).or_fail_with(|()| "failed to encode frames".to_owned())?;

    // VMAF の下準備としての処理は全て完了した
    progress_bar.finish();
    eprintln!("=> done\n");

    // vmaf コマンドを実行
    eprintln!("# Run vmaf command");
    let vmaf_output_file_path = args.root_dir.join(args.vmaf_output_file_path);
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
        encoded_frame_count,
        encoded_byte_size,
        encoded_duration_seconds: Seconds::new(encoded_duration),
        elapsed_seconds: Seconds::new(start_time.elapsed()),
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
    encoded_byte_size: u64,
    encoded_duration_seconds: Seconds,
    elapsed_seconds: Seconds,
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
            f.member("encoded_byte_size", self.encoded_byte_size)?;
            f.member("encoded_duration_seconds", self.encoded_duration_seconds)?;
            f.member("elapsed_seconds", self.elapsed_seconds)?;
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
