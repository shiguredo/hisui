use std::{
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{composer::Composer, layout::Layout, types::CodecName, video::FrameRate};

// TODO: resolution は必須ではなくして、省略時には動的に求められるようにする
const DEFAULT_LAYOUT_JSON: &str = r#"{
  "resolution": "1280x720",
  "audio_sources": [ "archive*.json" ],
  "video_layout": {"main": {
    "max_columns": 3,
    "video_sources": [ "archive*.json" ]
  }}
}"#;

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let layout_file_path: Option<PathBuf> = noargs::opt("layout-file")
        .short('l')
        .ty("PATH")
        .env("HISUI_LAYOUT_FILE_PATH")
        .doc(concat!(
            "合成に使用するレイアウトファイルを指定します\n",
            "\n",
            "省略された場合には、以下の内容のレイアウトで合成が行われます:\n",
            // TODO: DEFAULT_LAYOUT_JSON を参照するようにしたい
            r#"{"#,
            r#"  "audio_sources": [ "archive*.json" ],"#,
            r#"  "video_layout": {"main": {"#,
            r#"    "max_columns": 3,"#,
            r#"    "video_sources": [ "archive*.json" ]"#,
            r#"  }}"#,
            r#"}"#
        ))
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let output_file_path: Option<PathBuf> = noargs::opt("output-file")
        .short('o')
        .ty("PATH")
        .doc(concat!(
            "合成結果を保存するファイルを指定します\n",
            "\n",
            "この引数が未指定の場合には `--root-dir` 引数で\n",
            "指定したディレクトリに `output.mp4` という名前で保存されます"
        ))
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let stats_file_path: Option<PathBuf> = noargs::opt("stats-file")
        .short('s')
        .ty("PATH")
        .doc("合成中に収集した統計情報 (JSON) を保存するファイルを指定します")
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
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

    // レイアウトを準備
    let layout = create_layout(&root_dir, layout_file_path.as_deref()).or_fail()?;
    log::debug!("layout: {layout:?}");

    // 必要に応じて openh264 の共有ライブラリを読み込む
    let openh264_lib = if let Some(path) = openh264.as_ref().filter(|_| layout.has_video()) {
        Some(Openh264Library::load(path).or_fail()?)
    } else {
        None
    };

    // 出力ファイルパスを決定
    let out_file_path = output_file_path.unwrap_or_else(|| root_dir.join("output.mp4"));

    // Composer を作成して設定
    let mut composer = Composer::new(layout);
    composer.out_video_codec = CodecName::Vp8; // デフォルト値
    composer.out_audio_codec = CodecName::Opus; // デフォルト値
    composer.openh264_lib = openh264_lib;
    composer.show_progress_bar = !no_progress_bar;
    composer.max_cpu_cores = max_cpu_cores.map(|n| n.get());
    composer.out_stats_file = stats_file_path;

    // 合成を実行
    let result = composer.compose(&out_file_path).or_fail()?;

    if !result.success {
        // エラー発生時は終了コードを変える
        std::process::exit(1);
    }

    Ok(())
}

fn create_layout(root_dir: &PathBuf, layout_file_path: Option<&Path>) -> orfail::Result<Layout> {
    if let Some(layout_file_path) = layout_file_path {
        // レイアウトファイルが指定された場合
        let layout_json = std::fs::read_to_string(layout_file_path)
            .or_fail_with(|e| format!("failed to read {}: {e}", layout_file_path.display()))?;
        Layout::from_layout_json(layout_file_path, &layout_json, FrameRate::FPS_25).or_fail()
    } else {
        // デフォルトレイアウトを作成
        Layout::from_layout_json(
            &root_dir.join("default-layout.json"),
            DEFAULT_LAYOUT_JSON,
            FrameRate::FPS_25,
        )
        .or_fail()
    }
}
