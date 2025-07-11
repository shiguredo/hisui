use std::{
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use orfail::OrFail;

use crate::{
    json::{JsonObject, JsonValue},
    optuna::{Optuna, SearchSpace, TrialMetrics},
    subcommand_vmaf,
};

const DEFAULT_LAYOUT_JSON: &str = include_str!("../layout-examples/tune-vp8.json");
const DEFAULT_SEARCH_SPACE_JSON: &str = include_str!("../search-space-examples/full.json");

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let layout_file_path: Option<PathBuf> = noargs::opt("layout-file")
        .short('l')
        .ty("PATH")
        .doc(concat!(
            "パラメータ調整に使用するレイアウトファイルを指定します\n",
            "\n",
            "省略された場合には hisui/layout-examples/tune-vp8.json が使用されます",
        ))
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let search_space_file_path: Option<PathBuf> = noargs::opt("search-space-file")
        .short('s')
        .ty("PATH")
        .doc(concat!(
            "探索空間定義ファイル（JSON）のパスを指定します\n",
            "\n",
            "省略された場合には hisui/search-space-examples/full.json が使用されます",
        ))
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let tune_working_dir: PathBuf = noargs::opt("tune-working-dir")
        .ty("PATH")
        .default("hisui-tune/")
        .doc(concat!(
            "チューニング用に使われる作業ディレクトリを指定します\n",
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
    let trial_count: usize = noargs::opt("trial-count")
        .short('n')
        .ty("INTEGER")
        .default("100")
        .doc("実行する試行回数を指定します")
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let best_trials_interval: usize = noargs::opt("best-trials-interval")
        .short('i')
        .ty("INTEGER")
        .default("10")
        .doc("最適化結果の最良試行を表示する間隔を指定します（トライアル数）")
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
        // 全体の実行時間に大きく影響するので vmaf コマンドに比べてデフォルト値が小さめにしておく
        .default("300")
        .doc("調整用にエンコードする映像フレームの数を指定します")
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

    // 最初に optuna と vmaf コマンドが利用可能かどうかをチェックする
    Optuna::check_optuna_availability().or_fail()?;
    subcommand_vmaf::check_vmaf_availability().or_fail()?;

    // 必要なら tune_working_dir を作る
    let tune_working_dir = root_dir.join(tune_working_dir);
    if !tune_working_dir.exists() {
        std::fs::create_dir_all(&tune_working_dir).or_fail_with(|e| {
            format!(
                "failed to create tune working directory {}: {e}",
                tune_working_dir.display()
            )
        })?;
    }

    // レイアウトファイル（テンプレート）を読み込む
    let layout_template: JsonValue = if let Some(path) = &layout_file_path {
        std::fs::read_to_string(path)
            .or_fail()?
            .parse()
            .map(|nojson::Json(v)| v)
            .or_fail()?
    } else {
        DEFAULT_LAYOUT_JSON
            .parse()
            .map(|nojson::Json(v)| v)
            .or_fail()?
    };
    log::debug!("template: {layout_template:?}");

    // 探索空間ファイルを読み込む
    let search_space_json_string = if let Some(path) = &search_space_file_path {
        std::fs::read_to_string(path).or_fail()?
    } else {
        DEFAULT_SEARCH_SPACE_JSON.to_owned()
    };
    let search_space_raw_json = nojson::RawJson::parse(&search_space_json_string).or_fail()?;
    let mut search_space = SearchSpace::new(search_space_raw_json.value()).or_fail()?;

    // レイアウトテンプレートの処理に不要なエントリは捨てる
    search_space
        .items
        .retain(|path, _| matches!(path.get(&layout_template), Some(JsonValue::Null)));
    log::debug!("search space: {search_space:?}");

    // 最初にいろいろと情報を表示する
    let storage_url = format!("sqlite:///{}", tune_working_dir.join("optuna.db").display());
    eprintln!("====== INFO ======");
    eprintln!(
        "- layout file to tune:\t {}",
        layout_file_path
            .as_ref()
            .map_or("DEFAULT".to_owned(), |p| p.display().to_string())
    );
    eprintln!(
        "- search space file:\t {}",
        search_space_file_path
            .as_ref()
            .map_or("DEFAULT".to_owned(), |p| p.display().to_string())
    );
    eprintln!("- tune working dir:\t {}", tune_working_dir.display());
    eprintln!("- optuna storage:\t {storage_url}");
    eprintln!("- optuna study name:\t {study_name}");
    eprintln!("- optuna trial count:\t {trial_count}");
    eprintln!("- tuning metrics:\t [Encoding Speed Ratio (maximize), VMAF Score Mean (maximize)]");
    eprintln!("- tuning parameters ({}):", search_space.items.len());
    for (key, value) in &search_space.items {
        eprintln!("    - {key}:\t {}", nojson::Json(value));
    }
    eprintln!();

    // optuna の study を作る
    let optuna = Optuna::new(study_name.clone(), storage_url);
    optuna.create_study().or_fail()?;

    for i in 0..trial_count {
        eprintln!("====== TUNE TRIAL ({}/{trial_count}) ======", i + 1);
        // TODO: layout ファイルのパスを表示
        let ask_output = optuna.ask(&search_space).or_fail()?;

        let mut layout = layout_template.clone();
        ask_output.update_layout(&mut layout).or_fail()?;
        log::debug!(
            "[trial:{}] actual layout: {layout:?}",
            ask_output.trial_number
        );

        match run_trial_evaluation(
            &tune_working_dir,
            &study_name,
            ask_output.trial_number,
            &root_dir,
            &layout,
            frame_count,
            openh264.as_ref(),
            max_cpu_cores,
            no_progress_bar,
        )
        .or_fail()
        {
            Ok(metrics) => {
                optuna.tell(ask_output.trial_number, &metrics).or_fail()?;
            }
            Err(e) => {
                log::warn!(
                    "[trial:{}] failed to VMAF evaluation: {e}",
                    ask_output.trial_number
                );
                optuna.tell_fail(ask_output.trial_number).or_fail()?;
            }
        }

        if (i + 1) % best_trials_interval == 0 {
            display_best_trials(&optuna, &root_dir).or_fail()?;
        }
    }

    Ok(())
}

fn run_trial_evaluation(
    tune_working_dir: &Path,
    study_name: &str,
    trial_number: usize,
    root_dir: &Path,
    layout: &JsonValue,
    frame_count: usize,
    openh264: Option<&PathBuf>,
    max_cpu_cores: Option<NonZeroUsize>,
    no_progress_bar: bool,
) -> orfail::Result<TrialMetrics> {
    // トライアルの作業用ディレクトリを作成
    let trial_dir = tune_working_dir
        .join(study_name)
        .join(format!("trial-{}", trial_number));
    std::fs::create_dir_all(&trial_dir).or_fail_with(|e| {
        format!(
            "failed to create trial directory {}: {e}",
            trial_dir.display()
        )
    })?;
    let trial_dir = trial_dir.canonicalize().or_fail()?;

    // レイアウトファイルを作成
    let layout_file_path = trial_dir.join("layout.json");
    let layout_json = nojson::json(|f| {
        f.set_indent_size(2);
        f.set_spacing(true);
        f.value(layout)
    })
    .to_string();
    std::fs::write(&layout_file_path, layout_json).or_fail_with(|e| {
        format!(
            "failed to write layout file {}: {e}",
            layout_file_path.display(),
        )
    })?;

    // hisui vmaf コマンドを実行
    let mut cmd = Command::new("hisui");
    cmd.arg("vmaf");
    if no_progress_bar {
        cmd.arg("--no-progress-bar");
    }
    cmd.arg("--layout-file")
        .arg(&layout_file_path)
        .arg("--frame-count")
        .arg(frame_count.to_string())
        .arg("--reference-yuv-file")
        .arg(trial_dir.join("reference.yuv"))
        .arg("--distorted-yuv-file")
        .arg(trial_dir.join("distorted.yuv"))
        .arg("--vmaf-output-file")
        .arg(trial_dir.join("vmaf-output.json"))
        .arg(root_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    if let Some(openh264_path) = openh264 {
        cmd.arg("--openh264").arg(openh264_path);
    }

    if let Some(cores) = max_cpu_cores {
        cmd.arg("--max-cpu-cores").arg(cores.to_string());
    }

    let output = cmd
        .output()
        .or_fail_with(|e| format!("failed to execute hisui vmaf command: {e}"))?;
    output
        .status
        .success()
        .or_fail_with(|()| "hisui vmaf command failed".to_owned())?;

    // 出力結果をパース
    let stdout = String::from_utf8(output.stdout).or_fail()?;
    let result = nojson::RawJson::parse(&stdout).or_fail()?;
    let result_obj = JsonObject::new(result.value()).or_fail()?;

    // メトリクスを抽出
    let vmaf_mean: f64 = result_obj.get_required("vmaf_mean").or_fail()?;
    let encoding_speed_ratio: f64 = result_obj.get_required("encoding_speed_ratio").or_fail()?;

    // 後から参照できるように保存しておく
    std::fs::write(trial_dir.join("metrics.json"), &stdout).or_fail()?;

    Ok(TrialMetrics {
        encoding_speed_ratio,
        vmaf_mean,
    })
}

fn display_best_trials(optuna: &Optuna, root_dir: &Path) -> orfail::Result<()> {
    let mut best_trials = optuna.get_best_trials().or_fail()?;
    if best_trials.is_empty() {
        return Ok(());
    }
    best_trials.sort_by(|a, b| a.values[0].total_cmp(&b.values[0]).reverse());

    eprintln!("====== BEST TRIALS ======");
    eprintln!(
        "Top {} trials (sorted by encoding speed ratio):",
        best_trials.len()
    );
    eprintln!();

    for trial in best_trials {
        eprintln!("Trial #{}", trial.number);
        eprintln!("  Encoding Speed Ratio: {:.4}", trial.values[0]);
        eprintln!("  VMAF Score Mean: {:.4}", trial.values[1]);
        eprintln!("  Parameters:");
        for (key, value) in &trial.params {
            eprintln!("    {}: {}", key, format_param_value(value));
        }
        let layout_file_path = format!(
            "hisui-tune/{}/trial-{}/layout.json",
            optuna.study_name, trial.number
        );

        eprintln!("  Compose command:");
        eprintln!(
            "    hisui compose -l {} {}",
            layout_file_path,
            root_dir.display()
        );

        // TODO: layout ファイルのパスを表示
        // TODO: このレイアウトファイルを使った hisui compose コマンドを表示（手軽に試せるように）
        eprintln!();
    }

    Ok(())
}

fn format_param_value(value: &JsonValue) -> String {
    match value {
        JsonValue::String(s) => s.clone(),
        JsonValue::Integer(n) => n.to_string(),
        JsonValue::Float(n) => n.to_string(),
        JsonValue::Boolean(b) => b.to_string(),
        _ => format!("{:?}", value),
    }
}
