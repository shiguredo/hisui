use std::{
    collections::BTreeMap,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::LazyLock,
    time::{Duration, Instant},
};

use indicatif::{ProgressBar, ProgressStyle};
use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{
    channel::ErrorFlag,
    composer,
    decoder::{VideoDecoder, VideoDecoderOptions},
    encoder::{VideoEncoder, VideoEncoderThread},
    json::JsonObject,
    layout::Layout,
    mixer_video::VideoMixerThread,
    stats::{Seconds, SharedStats, VideoDecoderStats},
    video::FrameRate,
    writer_yuv::YuvWriter,
};

const DEFAULT_LAYOUT_JSON: &str = r#"{
  "video_layout": {"main": {
    "cell_width": 320,
    "cell_height": 240,
    "max_columns": 4,
    "max_rows": 4,
    "video_sources": [ "archive*.json" ]
  }}
}"#;

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let layout_file_path: Option<PathBuf> = noargs::opt("layout-file")
        .short('l')
        .ty("PATH")
        .env("HISUI_LAYOUT_FILE_PATH")
        .doc({
            static DOC: LazyLock<String> = LazyLock::new(|| {
                format!(
                    concat!(
                        "合成に使用するレイアウトファイルを指定します\n",
                        "\n",
                        "省略された場合には、以下の内容のレイアウトで合成が行われます:\n",
                        "{}"
                    ),
                    DEFAULT_LAYOUT_JSON
                )
            });
            &*DOC
        })
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let reference_yuv_file_path: PathBuf = noargs::opt("reference-yuv-file")
        .ty("PATH")
        .default("reference.yuv")
        .doc(concat!(
            "参照映像（合成前）のYUVファイルの出力先を指定します\n",
            "\n",
            "相対パスの場合は ROOT_DIR が起点となります"
        ))
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let distorted_yuv_file_path: PathBuf = noargs::opt("distorted-yuv-file")
        .ty("PATH")
        .default("distorted.yuv")
        .doc(concat!(
            "歪み映像（合成後）のYUVファイルの出力先を指定します\n",
            "\n",
            "相対パスの場合は ROOT_DIR が起点となります"
        ))
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let vmaf_output_file_path: PathBuf = noargs::opt("vmaf-output-file")
        .ty("PATH")
        .default("vmaf-output.json")
        .doc(concat!(
            "vmaf コマンドの実行結果ファイルの出力先を指定します\n",
            "\n",
            "相対パスの場合は ROOT_DIR が起点となります"
        ))
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let openh264: Option<PathBuf> = noargs::opt("openh264")
        .ty("PATH")
        .env("HISUI_OPENH264_PATH")
        .doc("OpenH264 の共有ライブラリのパスを指定します")
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let no_progress_bar: bool = noargs::flag("no-progress-bar")
        .short('P')
        .doc("指定された場合は、合成の進捗を非表示にします")
        .take(&mut args)
        .is_present();
    let max_cpu_cores: Option<NonZeroUsize> = noargs::opt("max-cpu-cores")
        .short('c')
        .ty("INTEGER")
        .env("HISUI_MAX_CPU_CORES")
        .doc(concat!(
            "合成処理を行うプロセスが使用するコア数の上限を指定します\n",
            "（未指定時には上限なし）\n",
            "\n",
            "NOTE: macOS ではこの引数は無視されます",
        ))
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let frame_count: usize = noargs::opt("frame-count")
        .short('f')
        .ty("FRAMES")
        .default("1000")
        .doc("変換するフレーム数を指定します")
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let root_dir: PathBuf = noargs::arg("ROOT_DIR")
        .example("/path/to/archive/RECORDING_ID/")
        .doc(concat!(
            "合成処理を行う際のルートディレクトリを指定します\n",
            "\n",
            "レイアウトファイル内に記載された相対パスの基点は、このディレクトリとなります。\n",
            "また、レイアウト内で、",
            "このディレクトリの外のファイルが参照された場合にはエラーとなります。"
        ))
        .take(&mut args)
        .then(|a| -> Result<_, Box<dyn std::error::Error>> {
            let path: PathBuf = a.value().parse()?;

            if matches!(a, noargs::Arg::Example { .. }) {
            } else if !path.exists() {
                return Err("no such directory".into());
            } else if !path.is_dir() {
                return Err("not a directory".into());
            }

            Ok(path)
        })?;

    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    // レイアウトを準備（音声処理は無効化）
    let mut layout = create_layout(&root_dir, layout_file_path.as_deref()).or_fail()?;
    layout.audio_source_ids.clear();
    log::debug!("layout: {layout:?}");
    layout
        .has_video()
        .or_fail_with(|()| "no video sources".to_owned())?;

    // 必要に応じて openh264 の共有ライブラリを読み込む
    let openh264_lib = if let Some(path) = openh264.as_ref().filter(|_| layout.has_video()) {
        Some(Openh264Library::load(path).or_fail()?)
    } else {
        None
    };

    // CPU コア数制限を適用
    if let Some(cores) = max_cpu_cores {
        composer::limit_cpu_cores(cores.get()).or_fail()?;
    }

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
    let progress_bar = create_progress_bar(!no_progress_bar, frame_count);

    // ミキサースレッドを起動
    let mut mixed_video_rx = VideoMixerThread::start(
        error_flag.clone(),
        layout.clone(),
        video_source_rxs,
        stats.clone(),
    );

    // エンコード前の画像の書き込みスレッドを起動
    let reference_yuv_file_path = root_dir.join(&reference_yuv_file_path);
    let mut reference_yuv_writer = YuvWriter::new(&reference_yuv_file_path).or_fail()?;
    let (mixed_video_temp_tx, mixed_video_temp_rx) = crate::channel::sync_channel();
    std::thread::spawn(move || {
        let mut count = 0;
        while let Some(frame) = mixed_video_rx.recv() {
            if count < frame_count {
                if let Err(e) = reference_yuv_writer.append(&frame).or_fail() {
                    log::error!("failed to write reference YUV frame: {e}");
                    break;
                }
            }
            if !mixed_video_temp_tx.send(frame) {
                break;
            }
            count += 1;
        }
    });

    // 映像エンコードスレッドを起動
    let encoder = VideoEncoder::new(&layout, openh264_lib.clone()).or_fail()?;
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
    let mut decoder = VideoDecoder::new(options);
    let distorted_yuv_file_path = root_dir.join(&distorted_yuv_file_path);
    let mut distorted_yuv_writer = YuvWriter::new(&distorted_yuv_file_path).or_fail()?;

    // 必要なフレームの処理が終わるまでループを回す
    eprintln!("# Compose");
    let mut dummy_video_decoder_stats = VideoDecoderStats::default();
    let mut encoded_byte_size = 0;
    let mut encoded_duration = Duration::ZERO;
    for _ in 0..frame_count {
        let Some(encoded_frame) = encoded_video_rx.recv() else {
            // 合成フレームの総数が frame_count よりも少なかった場合にここに来る
            decoder.finish().or_fail()?;
            while let Some(decoded_frame) = decoder.next_decoded_frame() {
                distorted_yuv_writer.append(&decoded_frame).or_fail()?;
                progress_bar.inc(1);
            }
            break;
        };
        encoded_byte_size += encoded_frame.data.len() as u64;
        encoded_duration += encoded_frame.duration;
        decoder
            .decode(encoded_frame, &mut dummy_video_decoder_stats)
            .or_fail()?;
        while let Some(decoded_frame) = decoder.next_decoded_frame() {
            distorted_yuv_writer.append(&decoded_frame).or_fail()?;
            progress_bar.inc(1);
        }

        if error_flag.get() {
            // ファイル読み込み、デコード、合成、エンコード、のいずれかで失敗したものがあるとここに来る
            log::error!("The composition process was aborted");
            break;
        }
    }

    // VMAF の下準備としての処理は全て完了した
    progress_bar.finish();
    eprintln!("=> done\n");

    // vmaf コマンドを実行
    eprintln!("# Run vmaf command");
    let vmaf_output_file_path = root_dir.join(vmaf_output_file_path);
    run_vmaf_evaluation(
        &reference_yuv_file_path,
        &distorted_yuv_file_path,
        &vmaf_output_file_path,
        &layout,
    )
    .or_fail()?;
    eprintln!("=> done\n");

    // VMAF結果を読み込んで解析
    let vmaf_summary = parse_vmaf_output(&vmaf_output_file_path).or_fail()?;

    // 実行結果の要約を標準出力に出力する
    let output = Output {
        reference_yuv_file_path,
        distorted_yuv_file_path,
        vmaf_output_file_path,
        width: layout.resolution.width().get(),
        height: layout.resolution.height().get(),
        frame_rate: layout.frame_rate,
        encoded_frame_count: progress_bar.length().unwrap_or_default() as usize,
        encoded_byte_size,
        encoded_duration_seconds: Seconds::new(encoded_duration),
        elapsed_seconds: Seconds::new(start_time.elapsed()),
        vmaf_summary,
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

fn create_layout(root_dir: &PathBuf, layout_file_path: Option<&Path>) -> orfail::Result<Layout> {
    if let Some(layout_file_path) = layout_file_path {
        // レイアウトファイルが指定された場合
        let layout_json = std::fs::read_to_string(layout_file_path)
            .or_fail_with(|e| format!("failed to read {}: {e}", layout_file_path.display()))?;
        Layout::from_layout_json(root_dir.clone(), layout_file_path, &layout_json).or_fail()
    } else {
        // デフォルトレイアウトを作成
        Layout::from_layout_json(
            root_dir.clone(),
            &root_dir.join("default-layout.json"),
            DEFAULT_LAYOUT_JSON,
        )
        .or_fail()
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
            "--pixel_format",
            "420",
            "--bitdepth",
            "8",
            "--output",
            vmaf_output_file_path.to_str().or_fail()?,
            "--json",
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

fn create_progress_bar(show_progress_bar: bool, frame_count: usize) -> ProgressBar {
    let progress_bar = if show_progress_bar {
        ProgressBar::new(frame_count as u64)
    } else {
        ProgressBar::hidden()
    };
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
            )
            .unwrap()
            .progress_chars("#>-"),
    );
    progress_bar
}

fn parse_vmaf_output(vmaf_output_file_path: &Path) -> orfail::Result<VmafSummary> {
    let vmaf_content = std::fs::read_to_string(vmaf_output_file_path)
        .or_fail_with(|e| format!("failed to read VMAF output file: {e}"))?;

    let json = nojson::RawJson::parse(&vmaf_content).or_fail()?;
    let vmaf_data = JsonObject::new(json.value()).or_fail()?;

    // フレームごとの VMAF スコアを収集
    let frames = vmaf_data
        .get_required_with("frames", |v| Ok(v.to_array()?.collect::<Vec<_>>()))
        .or_fail()?;

    let mut vmaf_scores = Vec::new();
    for frame in frames {
        let frame = JsonObject::new(frame).or_fail()?;
        let metrics = frame
            .get_required_with("metrics", JsonObject::new)
            .or_fail()?;
        if let Some(vmaf_score) = metrics.get_required("vmaf").or_fail()? {
            vmaf_scores.push(vmaf_score);
        }
    }

    // pooled_metrics から統計情報を抽出
    let pooled_metrics = vmaf_data
        .get_required_with("pooled_metrics", Ok)
        .or_fail()?;

    let mut metrics_summary = BTreeMap::new();

    for (metric_name, metric_data) in pooled_metrics.to_object().or_fail()? {
        let metric_name = metric_name.to_unquoted_string_str().or_fail()?.into_owned();
        if let Ok(metric_obj) = JsonObject::new(metric_data) {
            let summary = MetricSummary {
                min: metric_obj.get("min").or_fail()?.unwrap_or(0.0),
                max: metric_obj.get("max").or_fail()?.unwrap_or(0.0),
                mean: metric_obj.get("mean").or_fail()?.unwrap_or(0.0),
                harmonic_mean: metric_obj.get("harmonic_mean").or_fail()?,
            };
            metrics_summary.insert(metric_name, summary);
        }
    }

    // VMAFスコアの統計を計算
    let vmaf_stats = calculate_vmaf_statistics(&vmaf_scores);

    Ok(VmafSummary {
        vmaf_statistics: vmaf_stats,
        metrics_summary,
    })
}

fn calculate_vmaf_statistics(scores: &[f64]) -> VmafStatistics {
    if scores.is_empty() {
        return VmafStatistics {
            min: 0.0,
            max: 0.0,
            mean: 0.0,
            percentile_25: 0.0,
            median: 0.0,
            percentile_75: 0.0,
        };
    }

    let mut sorted_scores = scores.to_vec();
    sorted_scores.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let min = sorted_scores[0];
    let max = sorted_scores[sorted_scores.len() - 1];
    let mean = sorted_scores.iter().sum::<f64>() / sorted_scores.len() as f64;

    let percentile_25 = percentile(&sorted_scores, 25.0);
    let median = percentile(&sorted_scores, 50.0);
    let percentile_75 = percentile(&sorted_scores, 75.0);

    VmafStatistics {
        min,
        max,
        mean,
        percentile_25,
        median,
        percentile_75,
    }
}

fn percentile(sorted_values: &[f64], p: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }

    let index = (p / 100.0) * (sorted_values.len() - 1) as f64;
    let lower_index = index.floor() as usize;
    let upper_index = index.ceil() as usize;

    if lower_index == upper_index {
        sorted_values[lower_index]
    } else {
        let weight = index - lower_index as f64;
        sorted_values[lower_index] * (1.0 - weight) + sorted_values[upper_index] * weight
    }
}

#[derive(Debug)]
struct Output {
    reference_yuv_file_path: PathBuf,
    distorted_yuv_file_path: PathBuf,
    vmaf_output_file_path: PathBuf,
    width: usize,
    height: usize,
    frame_rate: FrameRate,
    encoded_frame_count: usize,
    encoded_byte_size: u64,
    encoded_duration_seconds: Seconds,
    elapsed_seconds: Seconds,
    vmaf_summary: VmafSummary,
}

impl nojson::DisplayJson for Output {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("reference_yuv_file_path", &self.reference_yuv_file_path)?;
            f.member("distorted_yuv_file_path", &self.distorted_yuv_file_path)?;
            f.member("vmaf_output_file_path", &self.vmaf_output_file_path)?;
            f.member("width", self.width)?;
            f.member("height", self.height)?;
            f.member("frame_rate", self.frame_rate)?;
            f.member("encoded_frame_count", self.encoded_frame_count)?;
            f.member("encoded_byte_size", self.encoded_byte_size)?;
            f.member("encoded_duration_seconds", self.encoded_duration_seconds)?;
            f.member("elapsed_seconds", self.elapsed_seconds)?;

            // 何倍速で変換が行えたか
            //（elapsed_seconds にはデコードや合成の時間も含まれているのであくまでも概算値）
            f.member(
                "encoding_speed_ratio",
                self.encoded_duration_seconds.get().as_secs_f64()
                    / self.elapsed_seconds.get().as_secs_f64(),
            )?;

            f.member("vmaf_summary", &self.vmaf_summary)?;

            Ok(())
        })
    }
}

#[derive(Debug)]
struct VmafSummary {
    vmaf_statistics: VmafStatistics,
    metrics_summary: BTreeMap<String, MetricSummary>,
}

#[derive(Debug)]
struct VmafStatistics {
    min: f64,
    max: f64,
    mean: f64,
    percentile_25: f64,
    median: f64,
    percentile_75: f64,
}

#[derive(Debug)]
struct MetricSummary {
    min: f64,
    max: f64,
    mean: f64,
    harmonic_mean: Option<f64>,
}

impl nojson::DisplayJson for VmafSummary {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("vmaf_statistics", &self.vmaf_statistics)?;
            f.member("metrics_summary", &self.metrics_summary)?;
            Ok(())
        })
    }
}

impl nojson::DisplayJson for VmafStatistics {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("min", self.min)?;
            f.member("max", self.max)?;
            f.member("mean", self.mean)?;
            f.member("percentile_25", self.percentile_25)?;
            f.member("median", self.median)?;
            f.member("percentile_75", self.percentile_75)?;
            Ok(())
        })
    }
}

impl nojson::DisplayJson for MetricSummary {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("min", self.min)?;
            f.member("max", self.max)?;
            f.member("mean", self.mean)?;
            if let Some(harmonic_mean) = self.harmonic_mean {
                f.member("harmonic_mean", harmonic_mean)?;
            }
            Ok(())
        })
    }
}
