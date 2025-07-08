use std::{
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::Command,
    sync::LazyLock,
};

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::layout::Layout;

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
    let reference_yuv: PathBuf = noargs::opt("reference-yuv")
        .ty("PATH")
        .default("reference.yuv")
        .doc(concat!(
            "参照映像（合成前）のYUVファイルの出力先を指定します\n",
            "\n",
            "相対パスの場合は ROOT_DIR が起点となります"
        ))
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let distorted_yuv: PathBuf = noargs::opt("distorted-yuv")
        .ty("PATH")
        .default("distorted.yuv")
        .doc(concat!(
            "歪み映像（合成後）のYUVファイルの出力先を指定します\n",
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
    let frame_count: u32 = noargs::opt("frame-count")
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
    // TODO: layout.audio_sources.clear(); // 音声処理を無効化

    // 時間範囲の設定
    // if let Some(start) = start_time {
    //     layout.start_time = Some(Duration::from_secs_f64(start));
    // }
    // if let Some(dur) = duration {
    //     layout.duration = Some(Duration::from_secs_f64(dur));
    // }

    log::debug!("layout: {layout:?}");

    // 必要に応じて openh264 の共有ライブラリを読み込む
    let openh264_lib = if let Some(path) = openh264.as_ref().filter(|_| layout.has_video()) {
        Some(Openh264Library::load(path).or_fail()?)
    } else {
        None
    };

    // VMAFコンポーザーを作成して設定
    let mut composer = VmafComposer::new(layout.clone(), reference_yuv, distorted_yuv);
    composer.openh264_lib = openh264_lib;
    composer.show_progress_bar = !no_progress_bar;
    composer.max_cpu_cores = max_cpu_cores.map(|n| n.get());

    // 合成を実行してYUVファイルを出力
    let result = composer.compose().or_fail()?;

    if !result.success {
        // エラー発生時は終了コードを変える
        std::process::exit(1);
    }

    // VMAF評価を実行
    run_vmaf_evaluation(&composer.reference_yuv, &composer.distorted_yuv, &layout).or_fail()?;

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

/// VMAF用のコンポーザー
pub struct VmafComposer {
    pub layout: Layout,
    pub openh264_lib: Option<Openh264Library>,
    pub show_progress_bar: bool,
    pub max_cpu_cores: Option<usize>,
    pub reference_yuv: PathBuf,
    pub distorted_yuv: PathBuf,
}

impl VmafComposer {
    pub fn new(layout: Layout, reference_yuv: PathBuf, distorted_yuv: PathBuf) -> Self {
        Self {
            layout,
            openh264_lib: None,
            show_progress_bar: false,
            max_cpu_cores: None,
            reference_yuv,
            distorted_yuv,
        }
    }

    pub fn compose(&self) -> orfail::Result<VmafComposeResult> {
        // // 通常のComposerを使用して合成処理を実行
        // let mut composer = Composer::new(self.layout.clone());
        // composer.openh264_lib = self.openh264_lib.clone();
        // composer.show_progress_bar = self.show_progress_bar;
        // composer.max_cpu_cores = self.max_cpu_cores;

        // // 一時的なMP4ファイルを作成
        // let temp_file = tempfile::NamedTempFile::new().or_fail()?;
        // let result = composer.compose(temp_file.path()).or_fail()?;

        // if result.success {
        //     // MP4からYUVを抽出
        //     self.extract_yuv_from_mp4(temp_file.path()).or_fail()?;
        // }

        // Ok(VmafComposeResult {
        //     success: result.success,
        // })
        todo!()
    }

    fn extract_yuv_from_mp4(&self, mp4_path: &Path) -> orfail::Result<()> {
        // 参照映像（合成前）のYUVを抽出
        // 実際の実装では、最初のソースから直接YUVを抽出する必要がある
        // ここでは簡略化のため、合成後のファイルから抽出
        let output = Command::new("ffmpeg")
            .args([
                "-i",
                mp4_path.to_str().unwrap(),
                "-pix_fmt",
                "yuv420p",
                "-f",
                "rawvideo",
                "-y",
                self.reference_yuv.to_str().unwrap(),
            ])
            .output()
            .or_fail()?;

        if !output.status.success() {
            // return Err(
            //     format!("ffmpeg failed: {}", String::from_utf8_lossy(&output.stderr)).into(),
            // );
            todo!()
        }

        // 歪み映像（合成後）のYUVを抽出
        let output = Command::new("ffmpeg")
            .args([
                "-i",
                mp4_path.to_str().unwrap(),
                "-pix_fmt",
                "yuv420p",
                "-f",
                "rawvideo",
                "-y",
                self.distorted_yuv.to_str().unwrap(),
            ])
            .output()
            .or_fail()?;

        if !output.status.success() {
            // return Err(
            //     format!("ffmpeg failed: {}", String::from_utf8_lossy(&output.stderr)).into(),
            // );
            todo!()
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct VmafComposeResult {
    pub success: bool,
}

fn run_vmaf_evaluation(
    reference_yuv: &Path,
    distorted_yuv: &Path,
    layout: &Layout,
) -> orfail::Result<()> {
    todo!()
    // let width = layout.resolution.width().to_string();
    // let height = layout.resolution.height().to_string();

    // let output = Command::new("vmaf")
    //     .args([
    //         "--reference",
    //         reference_yuv.to_str().unwrap(),
    //         "--distorted",
    //         distorted_yuv.to_str().unwrap(),
    //         "--width",
    //         &width,
    //         "--height",
    //         &height,
    //         "--pixel_format",
    //         "yuv420p",
    //         "--bitdepth",
    //         "8",
    //         "--output",
    //         vmaf_output.to_str().unwrap(),
    //         "--json",
    //     ])
    //     .output()
    //     .or_fail()?;

    // if !output.status.success() {
    //     return Err(format!("vmaf failed: {}", String::from_utf8_lossy(&output.stderr)).into());
    // }

    // VMAF結果を読み込んで表示
    // let vmaf_result = std::fs::read_to_string(vmaf_output).or_fail()?;
    // let json: Value = serde_json::from_str(&vmaf_result).or_fail()?;

    // if let Some(pooled_metrics) = json.get("pooled_metrics") {
    //     if let Some(vmaf) = pooled_metrics.get("vmaf") {
    //         if let Some(mean) = vmaf.get("mean") {
    //             println!("VMAF Score: {}", mean);
    //         }
    //     }
    // }

    // Ok(())
}
