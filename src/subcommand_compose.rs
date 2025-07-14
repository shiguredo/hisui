use std::{
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{composer::Composer, layout::Layout};

const DEFAULT_LAYOUT_JSON: &str = include_str!("../layout-examples/compose-default.json");

#[derive(Debug)]
struct Args {
    layout_file_path: Option<PathBuf>,
    output_file_path: PathBuf,
    stats_file_path: Option<PathBuf>,
    openh264: Option<PathBuf>,
    no_progress_bar: bool,
    max_cpu_cores: Option<NonZeroUsize>,
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
                    "hisui/layout-examples/compose-default.json の内容が使用されます",
                ))
                .take(raw_args)
                .present_and_then(|a| a.value().parse())?,
            output_file_path: noargs::opt("output-file")
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
                .take(raw_args)
                .then(|a| a.value().parse())?,
            stats_file_path: noargs::opt("stats-file")
                .short('s')
                .ty("PATH")
                .doc(concat!(
                    "合成中に収集した統計情報 (JSON) を保存するファイルを指定します\n",
                    "\n",
                    "相対パスの場合は ROOT_DIR が起点となります"
                ))
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

    // レイアウトを準備
    let layout = create_layout(&args.root_dir, args.layout_file_path.as_deref()).or_fail()?;
    log::debug!("layout: {layout:?}");

    // 必要に応じて openh264 の共有ライブラリを読み込む
    let openh264_lib = if let Some(path) = args.openh264.as_ref().filter(|_| layout.has_video()) {
        Some(Openh264Library::load(path).or_fail()?)
    } else {
        None
    };

    // 出力ファイルパスを決定
    let out_file_path = args.root_dir.join(args.output_file_path);

    // Composer を作成して設定
    let mut composer = Composer::new(layout);
    composer.openh264_lib = openh264_lib;
    composer.show_progress_bar = !args.no_progress_bar;
    composer.max_cpu_cores = args.max_cpu_cores.map(|n| n.get());
    composer.stats_file_path = args.stats_file_path.map(|path| args.root_dir.join(path));

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
