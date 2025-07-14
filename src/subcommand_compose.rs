use std::{
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{composer::Composer, layout::Layout};

const DEFAULT_LAYOUT_JSON: &str = include_str!("../layout-examples/compose-default.json");

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let layout_file_path: Option<PathBuf> = noargs::opt("layout-file")
        .short('l')
        .ty("PATH")
        .env("HISUI_LAYOUT_FILE_PATH")
        .doc(concat!(
            "合成に使用するレイアウトファイルを指定します\n",
            "\n",
            "省略された場合には hisui/layout-examples/compose-default.json の内容が使用されます",
        ))
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let output_file_path: PathBuf = noargs::opt("output-file")
        .short('o')
        .ty("PATH")
        .default("output.mp4")
        .doc(concat!(
            "合成結果を保存するファイルを指定します\n",
            "\n",
            "この引数が未指定の場合には ROOT_DIR 引数で\n",
            "指定したディレクトリに `output.mp4` という名前で保存されます\n",
            "\n",
            "相対パスの場合は ROOT_DIR が起点となります"
        ))
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let stats_file_path: Option<PathBuf> = noargs::opt("stats-file")
        .short('s')
        .ty("PATH")
        .doc(concat!(
            "合成中に収集した統計情報 (JSON) を保存するファイルを指定します\n",
            "\n",
            "相対パスの場合は ROOT_DIR が起点となります"
        ))
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
    let out_file_path = root_dir.join(output_file_path);

    // Composer を作成して設定
    let mut composer = Composer::new(layout);
    composer.openh264_lib = openh264_lib;
    composer.show_progress_bar = !no_progress_bar;
    composer.max_cpu_cores = max_cpu_cores.map(|n| n.get());
    composer.stats_file_path = stats_file_path.map(|path| root_dir.join(path));

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
        Layout::from_layout_json_file(root_dir.clone(), layout_file_path).or_fail()
    } else {
        // デフォルトレイアウトを作成
        Layout::from_layout_json_str(root_dir.clone(), DEFAULT_LAYOUT_JSON).or_fail()
    }
}
