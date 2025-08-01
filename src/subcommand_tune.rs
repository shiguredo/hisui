use std::{
    num::NonZeroUsize,
    path::PathBuf,
    process::{Command, Stdio},
};

use orfail::OrFail;

use crate::{
    json::{JsonObject, JsonValue},
    optuna::{OptunaStudy, SearchSpace, TrialValues},
    subcommand_vmaf,
};

const DEFAULT_LAYOUT_JSON: &str = include_str!("../layout-examples/tune-libvpx-vp8.json");
const DEFAULT_SEARCH_SPACE_JSON: &str = include_str!("../search-space-examples/full.json");

#[derive(Debug)]
struct Args {
    layout_file_path: Option<PathBuf>,
    search_space_file_path: Option<PathBuf>,
    tune_working_dir: PathBuf,
    study_name: String,
    trial_count: usize,
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
                .doc(concat!(
                    "パラメータ調整に使用するレイアウトファイルを指定します\n",
                    "\n",
                    "省略された場合には ",
                    "hisui/layout-examples/tune-libvpx-vp8.json の内容が使用されます",
                ))
                .take(raw_args)
                .present_and_then(|a| a.value().parse())?,
            search_space_file_path: noargs::opt("search-space-file")
                .short('s')
                .ty("PATH")
                .doc(concat!(
                    "探索空間定義ファイル（JSON）のパスを指定します\n",
                    "\n",
                    "省略された場合には hisui/search-space-examples/full.json の内容が使用されます",
                ))
                .take(raw_args)
                .present_and_then(|a| a.value().parse())?,
            tune_working_dir: noargs::opt("tune-working-dir")
                .ty("PATH")
                .default("hisui-tune/")
                .doc(concat!(
                    "チューニング用に使われる作業ディレクトリを指定します\n",
                    "\n",
                    "相対パスの場合は ROOT_DIR が起点となります"
                ))
                .take(raw_args)
                .then(|a| a.value().parse())?,
            study_name: noargs::opt("study-name")
                .ty("NAME")
                .default("hisui-tune")
                .doc("Optuna の study 名を指定します")
                .take(raw_args)
                .then(|a| a.value().parse())?,
            trial_count: noargs::opt("trial-count")
                .short('n')
                .ty("INTEGER")
                .default("100")
                .doc("実行する試行回数を指定します")
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
                    "調整処理を行うプロセスが使用するコア数の上限を指定します\n",
                    "（未指定時には上限なし）\n",
                    "\n",
                    "NOTE: macOS ではこの引数は無視されます",
                ))
                .take(raw_args)
                .present_and_then(|a| a.value().parse())?,
            frame_count: noargs::opt("frame-count")
                .short('f')
                .ty("FRAMES")
                // 全体の実行時間に大きく影響するので vmaf コマンドに比べてデフォルト値が小さめにしておく
                .default("300")
                .doc("調整用にエンコードする映像フレームの数を指定します")
                .take(raw_args)
                .then(|a| a.value().parse())?,
            root_dir: noargs::arg("ROOT_DIR")
                .example("/path/to/archive/RECORDING_ID/")
                .doc(concat!(
                    "調整処理を行う際のルートディレクトリを指定します\n",
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
    // コマンドライン引数処理
    let mut args = Args::parse(&mut raw_args)?;
    if let Some(help) = raw_args.finish()? {
        print!("{help}");
        return Ok(());
    }

    // 最初に optuna と vmaf コマンドが利用可能かどうかをチェックする
    OptunaStudy::check_optuna_availability().or_fail()?;
    subcommand_vmaf::check_vmaf_availability().or_fail()?;

    // 必要なら tune_working_dir を作る
    args.tune_working_dir = args.root_dir.join(args.tune_working_dir);
    if !args.tune_working_dir.exists() {
        std::fs::create_dir_all(&args.tune_working_dir).or_fail_with(|e| {
            format!(
                "failed to create working directory {}: {e}",
                args.tune_working_dir.display()
            )
        })?;
    }

    // レイアウトファイル（テンプレート）を読み込む
    let layout_template: JsonValue = if let Some(path) = &args.layout_file_path {
        crate::json::parse_file(path).or_fail()?
    } else {
        crate::json::parse_str(DEFAULT_LAYOUT_JSON).or_fail()?
    };
    log::debug!("layout template: {layout_template:?}");

    // 探索空間ファイルを読み込む
    let mut search_space: SearchSpace = if let Some(path) = &args.search_space_file_path {
        crate::json::parse_file(path).or_fail()?
    } else {
        crate::json::parse_str(DEFAULT_SEARCH_SPACE_JSON).or_fail()?
    };

    // 探索空間から不要なエントリを除外する（Optuna の探索を効率化するため）
    search_space
        .params
        .retain(|path, _| matches!(path.get(&layout_template), Some(JsonValue::Null)));
    log::debug!("search space: {search_space:?}");

    // 探索を始める前にいろいろと情報を表示する
    let storage_url = format!(
        "sqlite:///{}",
        args.tune_working_dir.join("optuna.db").display()
    );
    eprintln!("====== INFO ======");
    eprintln!(
        "layout file to tune:\t {}",
        args.layout_file_path
            .as_ref()
            .map_or("DEFAULT".to_owned(), |p| p.display().to_string())
    );
    eprintln!(
        "search space file:\t {}",
        args.search_space_file_path
            .as_ref()
            .map_or("DEFAULT".to_owned(), |p| p.display().to_string())
    );
    eprintln!("tune working dir:\t {}", args.tune_working_dir.display());
    eprintln!("optuna storage:\t {storage_url}");
    eprintln!("optuna study name:\t {}", args.study_name);
    eprintln!("optuna trial count:\t {}", args.trial_count);
    eprintln!("tuning metrics:\t [Execution Time (minimize), VMAF Score Mean (maximize)]");
    eprintln!("tuning parameters ({}):", search_space.params.len());
    for (key, value) in &search_space.params {
        eprintln!("  {key}:\t {}", nojson::Json(value));
    }
    eprintln!();

    // optuna の study を作る
    eprintln!("====== CREATE OPTUNA STUDY ======");
    let mut optuna = OptunaStudy::new(args.study_name.clone(), storage_url);
    optuna.create_study().or_fail()?;
    eprintln!();

    let mut displayed_best_trials = false;
    for i in 0..args.trial_count {
        eprintln!(
            "====== OPTUNA TRIAL ({}/{}) ======",
            i + 1,
            args.trial_count
        );
        eprintln!("=== SAMPLE PARAMETERS ===");
        let ask_output = optuna.ask(&search_space).or_fail()?;

        let mut layout = layout_template.clone();
        ask_output.apply_params_to_layout(&mut layout).or_fail()?;
        log::debug!("actual layout: {layout:?}");

        match run_trial_evaluation(&args, ask_output.number, &layout).or_fail() {
            Ok(metrics) => {
                optuna.tell(ask_output.number, &metrics).or_fail()?;
            }
            Err(e) => {
                eprintln!("failed to VMAF evaluation: {e}",);
                optuna.tell_fail(ask_output.number).or_fail()?;
            }
        }
        eprintln!();

        displayed_best_trials =
            display_best_trials_if_updated(&args, &mut optuna, false).or_fail()?;
    }

    if !displayed_best_trials {
        // 直前で表示していないなら、最後に結果を表示する
        display_best_trials_if_updated(&args, &mut optuna, true).or_fail()?;
    }

    Ok(())
}

fn trial_dir(args: &Args, trial_number: usize) -> PathBuf {
    args.tune_working_dir
        .join(&args.study_name)
        .join(format!("trial-{}", trial_number))
}

fn run_trial_evaluation(
    args: &Args,
    trial_number: usize,
    layout: &JsonValue,
) -> orfail::Result<TrialValues> {
    // トライアルの作業用ディレクトリを作成
    let trial_dir = trial_dir(args, trial_number);
    std::fs::create_dir_all(&trial_dir).or_fail_with(|e| {
        format!(
            "failed to create trial directory {}: {e}",
            trial_dir.display()
        )
    })?;
    let trial_dir = trial_dir.canonicalize().or_fail()?;

    // レイアウトファイルを作成
    let layout_file_path = trial_dir.join("layout.json");
    let layout_json = crate::json::to_pretty_string(layout);
    std::fs::write(&layout_file_path, layout_json).or_fail_with(|e| {
        format!(
            "failed to write layout file {}: {e}",
            layout_file_path.display(),
        )
    })?;

    // hisui vmaf コマンドを実行
    let mut cmd = Command::new("hisui");
    cmd.arg("vmaf")
        .arg("--layout-file")
        .arg(&layout_file_path)
        .arg("--frame-count")
        .arg(args.frame_count.to_string())
        .arg("--reference-yuv-file")
        .arg(trial_dir.join("reference.yuv"))
        .arg("--distorted-yuv-file")
        .arg(trial_dir.join("distorted.yuv"))
        .arg("--vmaf-output-file")
        .arg(trial_dir.join("vmaf-output.json"))
        .arg(&args.root_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if let Some(openh264_path) = &args.openh264 {
        cmd.arg("--openh264").arg(openh264_path);
    }

    if let Some(cores) = &args.max_cpu_cores {
        cmd.arg("--max-cpu-cores").arg(cores.to_string());
    }
    eprintln!();
    eprintln!("=== EVALUATE PARAMETERS ===");
    eprintln!("$ {cmd:?}");
    eprintln!();

    let output = cmd
        .output()
        .or_fail_with(|e| format!("failed to execute `$ hisui vmaf` command: {e}"))?;
    output
        .status
        .success()
        .or_fail_with(|()| "`$ hisui vmaf` command failed".to_owned())?;

    // YUV ファイルはサイズが大きいので不要になったら削除する
    for name in ["reference.yuv", "distorted.yuv"] {
        let path = trial_dir.join(name);
        if let Err(e) = std::fs::remove_file(&path) {
            eprintln!("[WARN] failed to remove file {}: {e}", path.display());
        }
    }

    // 出力結果をパース
    let stdout = String::from_utf8(output.stdout).or_fail()?;
    let result = nojson::RawJson::parse(&stdout).or_fail()?;
    let object = JsonObject::new(result.value()).or_fail()?;

    // メトリクスを抽出
    let vmaf_mean: f64 = object.get_required("vmaf_mean").or_fail()?;
    let elapsed_seconds: f64 = object.get_required("elapsed_seconds").or_fail()?;

    // TODO(sile): hisui compose コマンドを実行して所要時間を計測することを検討する
    //
    // 今は `hisui vmaf` コマンドの所要時間を使って最適化を行っているが、
    // これは以下の点で、実際の合成の処理とは異なっている:
    // - YUV データの書き出しがある
    // - 合成後の画像のエンコード後に、追加のデコード処理が走る (YUV 取得のため）
    //   - デコードコストはコーデックやデコーダーによって変わるので、コーデックが変わった場合に `elapsed_seconds` の単純な比較が難しくなる
    //
    // そのため `hisui compose` を使って所要時間を計測した方が、実際の値に近くなる。
    // ただし、その場合、（余計な合成処理が増えるので）最適化にかかる時間が長くなる、というデメリットがある。
    // また、`hisui vmaf` での所要時間計測方法が多少不正確だとしても、最適化の用途では通常は問題ない
    // とも考えられるので、この TODO は実際に必要になったタイミングで改めて対応を検討することにする。

    // 後から参照できるように保存しておく
    std::fs::write(trial_dir.join("metrics.json"), &stdout).or_fail()?;

    Ok(TrialValues {
        elapsed_seconds,
        vmaf_mean,
    })
}

fn display_best_trials_if_updated(
    args: &Args,
    optuna: &mut OptunaStudy,
    force: bool,
) -> orfail::Result<bool> {
    let (updated, mut best_trials) = optuna.get_best_trials().or_fail()?;
    if !updated && !force {
        // 更新なし
        return Ok(false);
    };

    // 所要時間が短い順にソートする
    best_trials.sort_by(|a, b| {
        a.values
            .elapsed_seconds
            .total_cmp(&b.values.elapsed_seconds)
    });

    eprintln!("====== BEST TRIALS (sorted by execution time) ======");
    for trial in best_trials {
        eprintln!("Trial #{}", trial.number);
        eprintln!("  Execution Time:\t {:.4}s", trial.values.elapsed_seconds);
        eprintln!("  VMAF Score Mean:\t {:.4}", trial.values.vmaf_mean);
        eprintln!("  Parameters:");
        for (key, value) in &trial.params {
            eprintln!("    {}:\t {}", key, nojson::Json(value));
        }

        let layout_file_path = trial_dir(args, trial.number).join("layout.json");

        eprintln!("  Compose Command:");
        eprintln!(
            "    $ hisui compose -l {} {}",
            layout_file_path.display(),
            args.root_dir.display()
        );
        eprintln!();
    }

    Ok(true)
}
