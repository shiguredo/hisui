use std::{
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::LazyLock,
    time::Instant,
};

use indicatif::{ProgressBar, ProgressStyle};
use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{
    channel::ErrorFlag,
    composer,
    decoder::{VideoDecoder, VideoDecoderOptions},
    encoder::{VideoEncoder, VideoEncoderThread},
    layout::Layout,
    mixer_video::VideoMixerThread,
    stats::{SharedStats, VideoDecoderStats},
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
    let stats_file_path: PathBuf = noargs::opt("stats-file")
        .ty("PATH")
        .default("stats.json")
        .doc(concat!(
            "合成中に収集した統計情報 (JSON) を保存するファイルを指定します\n",
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

    let stats_file_path = root_dir.join(stats_file_path);
    stats.finish(start_time, &stats_file_path);

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

    // 実行結果の要約を標準出力に出力する
    let output = Output {
        width: layout.resolution.width().get(),
        height: layout.resolution.height().get(),
        frame_rate: layout.frame_rate,
        reference_yuv_file_path,
        distorted_yuv_file_path,
        vmaf_output_file_path,
        hisui_stats_file_path: stats_file_path,
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

#[derive(Debug)]
struct Output {
    width: usize,
    height: usize,
    frame_rate: FrameRate,
    reference_yuv_file_path: PathBuf,
    distorted_yuv_file_path: PathBuf,
    vmaf_output_file_path: PathBuf,
    hisui_stats_file_path: PathBuf,
}

impl nojson::DisplayJson for Output {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("width", self.width)?;
            f.member("height", self.height)?;
            f.member("frame_rate", self.frame_rate.to_string())?;
            f.member("reference_yuv_file_path", &self.reference_yuv_file_path)?;
            f.member("distorted_yuv_file_path", &self.distorted_yuv_file_path)?;
            f.member("vmaf_output_file_path", &self.vmaf_output_file_path)?;
            f.member("hisui_stats_file_path", &self.hisui_stats_file_path)?;
            Ok(())
        })
    }
}
