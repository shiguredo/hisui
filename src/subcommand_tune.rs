use std::{
    num::NonZeroUsize,
    path::{Path, PathBuf},
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
    let output_file_path: PathBuf = noargs::opt("output-file")
        .short('o')
        .ty("PATH")
        .default("tuned-output.mp4")
        .doc(concat!(
            "調整結果を保存するファイルを指定します\n",
            "\n",
            "この引数が未指定の場合には ROOT_DIR 引数で\n",
            "指定したディレクトリに `tuned-output.mp4` という名前で保存されます\n",
            "\n",
            "相対パスの場合は ROOT_DIR が起点となります"
        ))
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let study_name: String = noargs::opt("study-name")
        .ty("NAME")
        .default("hisui-tune")
        .doc("Optuna の study 名を指定します")
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let n_trials: usize = noargs::opt("n-trials")
        .short('n')
        .ty("INTEGER")
        .default("100")
        .doc("実行する試行回数を指定します")
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let timeout: Option<u64> = noargs::opt("timeout")
        .short('t')
        .ty("SECONDS")
        .doc("調整処理のタイムアウト時間（秒）を指定します")
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
        .doc("指定された場合は、調整の進捗を非表示にします")
        .take(&mut args)
        .is_present();
    let max_cpu_cores: Option<NonZeroUsize> = noargs::opt("max-cpu-cores")
        .short('c')
        .ty("INTEGER")
        .env("HISUI_MAX_CPU_CORES")
        .doc(concat!(
            "調整処理を行うプロセスが使用するコア数の上限を指定します\n",
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
        .doc("調整に使用するフレーム数を指定します")
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let target_metric: String = noargs::opt("target-metric")
        .ty("METRIC")
        .default("vmaf")
        .doc("最適化対象の指標を指定します (vmaf, psnr, ssim)")
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let root_dir: PathBuf = noargs::arg("ROOT_DIR")
        .example("/path/to/archive/RECORDING_ID/")
        .doc(concat!(
            "調整処理を行う際のルートディレクトリを指定します\n",
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

    // TODO: Optuna の availability チェック
    check_optuna_availability().or_fail()?;

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
        crate::composer::limit_cpu_cores(cores.get()).or_fail()?;
    }

    // 出力ファイルパスを決定
    let out_file_path = root_dir.join(output_file_path);

    // 調整設定を作成
    let tune_config = TuneConfig {
        layout,
        openh264_lib,
        study_name,
        n_trials,
        timeout,
        show_progress_bar: !no_progress_bar,
        frame_count,
        target_metric,
        root_dir,
        out_file_path,
    };

    // 調整を実行
    eprintln!("# Starting parameter tuning with Optuna");
    let result = run_parameter_tuning(tune_config).or_fail()?;

    // 結果を出力
    println!(
        "{}",
        nojson::json(|f| {
            f.set_indent_size(2);
            f.set_spacing(true);
            f.value(&result)
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

fn check_optuna_availability() -> orfail::Result<()> {
    // TODO: Optuna の availability チェック実装
    // Python の optuna パッケージが利用可能かチェック
    eprintln!("# Checking Optuna availability");
    Ok(())
}

fn run_parameter_tuning(config: TuneConfig) -> orfail::Result<TuneResult> {
    // TODO: 実際の parameter tuning 実装
    eprintln!("# Running parameter tuning");
    eprintln!("  Study name: {}", config.study_name);
    eprintln!("  Trials: {}", config.n_trials);
    eprintln!("  Frame count: {}", config.frame_count);
    eprintln!("  Target metric: {}", config.target_metric);

    // 仮の結果を返す
    Ok(TuneResult {
        study_name: config.study_name,
        n_trials: config.n_trials,
        best_score: 85.0, // 仮の値
        // best_params: std::collections::HashMap::new(),
        output_file_path: config.out_file_path,
    })
}

#[derive(Debug)]
struct TuneConfig {
    layout: Layout,
    openh264_lib: Option<Openh264Library>,
    study_name: String,
    n_trials: usize,
    timeout: Option<u64>,
    show_progress_bar: bool,
    frame_count: usize,
    target_metric: String,
    root_dir: PathBuf,
    out_file_path: PathBuf,
}

#[derive(Debug)]
struct TuneResult {
    study_name: String,
    n_trials: usize,
    best_score: f64,
    //best_params: todo!(),
    output_file_path: PathBuf,
}

impl nojson::DisplayJson for TuneResult {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("study_name", &self.study_name)?;
            f.member("n_trials", self.n_trials)?;
            f.member("best_score", self.best_score)?;
            //f.member("best_params", &self.best_params)?;
            f.member("output_file_path", &self.output_file_path)?;
            Ok(())
        })
    }
}
