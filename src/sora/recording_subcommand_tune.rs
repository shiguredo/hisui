use std::{
    num::NonZeroUsize,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

use crate::tune::{SearchSpace, TrialValues, Tuner, json_value::JsonValue};

const DEFAULT_LAYOUT_JSON: &str = include_str!("../../layout-examples/tune-libvpx-vp9.jsonc");
const DEFAULT_SEARCH_SPACE_JSON: &str = include_str!("../../search-space-examples/full.jsonc");

#[derive(Debug)]
struct Args {
    layout_file_path: Option<PathBuf>,
    search_space_file_path: Option<PathBuf>,
    tune_working_dir: Option<PathBuf>,
    name: String,
    trial_count: usize,
    trial_timeout: Option<Duration>,
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
                .default("HISUI_REPO/layout-examples/tune-libvpx-vp9.jsonc")
                .doc("パラメータ調整に使用するレイアウトファイルを指定します")
                .take(raw_args)
                .then(crate::arg_utils::parse_non_default_opt)?,
            search_space_file_path: noargs::opt("search-space-file")
                .short('s')
                .ty("PATH")
                .default("HISUI_REPO/search-space-examples/full.jsonc")
                .doc("探索空間定義ファイル（JSON）のパスを指定します")
                .take(raw_args)
                .then(crate::arg_utils::parse_non_default_opt)?,
            tune_working_dir: noargs::opt("tune-working-dir")
                .ty("PATH")
                .default("ROOT_DIR/hisui-tune/")
                .doc("チューニング用に使われる作業ディレクトリを指定します")
                .take(raw_args)
                .then(crate::arg_utils::parse_non_default_opt)?,
            name: noargs::opt("name")
                .ty("NAME")
                .default("hisui-tune")
                .doc("探索履歴の保存に使う名前を指定します（名前ごとに履歴が分かれます）")
                .take(raw_args)
                .then(|a| a.value().parse())?,
            trial_count: noargs::opt("trial-count")
                .short('n')
                .ty("INTEGER")
                .default("100")
                .doc(concat!(
                    "目標とする合計試行回数を指定します\n",
                    "（既存の履歴を含む合計がこの値に達するまで試行します。\n",
                    "既存の履歴がこの値以上の場合は新規試行は行いません）"
                ))
                .take(raw_args)
                .then(|a| a.value().parse())?,
            trial_timeout: noargs::opt("trial-timeout")
                .short('t')
                .ty("SECONDS")
                .doc(concat!(
                    "各試行トライアルのタイムアウト時間（秒）を指定します",
                    "（超過した場合は失敗扱い）"
                ))
                .take(raw_args)
                .present_and_then(|a| a.value().parse::<f32>().map(Duration::from_secs_f32))?,
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

    fn tune_working_dir(&self) -> PathBuf {
        // メソッド呼び出しの度にメモリアロケーションが発生するが、
        // そのコストは無視できる程度のものなので、コードの簡潔さの方を優先している
        self.tune_working_dir
            .clone()
            .unwrap_or_else(|| self.root_dir.join("hisui-tune/"))
    }
}

pub fn try_run(args: &mut noargs::RawArgs) -> noargs::Result<bool> {
    if !noargs::cmd("tune")
        .doc("映像エンコードパラメーターの調整を行います")
        .take(args)
        .is_present()
    {
        return Ok(false);
    }
    run(args)?;
    Ok(true)
}

fn run(raw_args: &mut noargs::RawArgs) -> noargs::Result<()> {
    // コマンドライン引数処理
    let args = Args::parse(raw_args)?;
    if raw_args.metadata().help_mode {
        return Ok(());
    }

    run_internal(args).map_err(noargs::Error::from)
}

fn run_internal(args: Args) -> crate::Result<()> {
    // 必要なら tune_working_dir を作る
    if !args.tune_working_dir().exists() {
        std::fs::create_dir_all(args.tune_working_dir()).map_err(|e| {
            crate::Error::new(format!(
                "failed to create working directory {}: {e}",
                args.tune_working_dir().display()
            ))
        })?;
    }

    // レイアウトファイル（テンプレート）を読み込む
    let layout_template: JsonValue = if let Some(path) = &args.layout_file_path {
        crate::json::parse_file(path)?
    } else {
        crate::json::parse_str(DEFAULT_LAYOUT_JSON)?
    };
    tracing::debug!("layout template: {layout_template:?}");

    // 探索空間ファイルを読み込む
    let mut search_space: SearchSpace = if let Some(path) = &args.search_space_file_path {
        crate::json::parse_file(path)?
    } else {
        crate::json::parse_str(DEFAULT_SEARCH_SPACE_JSON)?
    };

    // 探索空間から不要なエントリを除外する（探索を効率化するため）
    search_space
        .params
        .retain(|path, _| matches!(path.get(&layout_template), Some(JsonValue::Null)));
    tracing::debug!("search space: {search_space:?}");

    if search_space.params.is_empty() {
        return Err(crate::Error::new(
            concat!(
                "No tunable parameters found in the search space. ",
                "This could happen if the layout file doesn't contain any null values ",
                "that correspond to the parameters defined in the search space file."
            )
            .to_owned(),
        ));
    }

    // 探索を始める前にいろいろと情報を表示する
    let jsonl_path = args.tune_working_dir().join(format!("{}.jsonl", args.name));
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
    eprintln!("tune working dir:\t {}", args.tune_working_dir().display());
    eprintln!("trials file:\t {}", jsonl_path.display());
    eprintln!("name:\t {}", args.name);
    eprintln!("target total trials:\t {}", args.trial_count);
    eprintln!("tuning metrics:\t [Execution Time (minimize), VMAF Score Mean (maximize)]");
    eprintln!("tuning parameters ({}):", search_space.params.len());
    for (key, value) in &search_space.params {
        eprintln!("  {key}:\t {}", nojson::Json(value));
    }
    eprintln!();

    // チューナーを開く（既存の履歴があれば続きから最適化する）
    eprintln!("====== OPEN HISTORY ======");
    let mut tuner = Tuner::new(args.name.clone(), args.tune_working_dir())?;

    // --trial-count は「合計到達ベース」。既存件数を差し引いた残り回数だけ新たに試行する。
    let existing = tuner.trial_count();
    let remaining = args.trial_count.saturating_sub(existing);
    eprintln!("existing trials:\t {existing}");
    eprintln!("remaining trials:\t {remaining}");
    eprintln!();

    let mut displayed_best_trials = false;
    for i in 0..remaining {
        eprintln!(
            "====== TRIAL ({}/{}) (total target {}) ======",
            i + 1,
            remaining,
            args.trial_count
        );
        eprintln!("=== SAMPLE PARAMETERS ===");
        let ask_output = tuner.ask(&search_space)?;

        let mut layout = layout_template.clone();
        ask_output.apply_params_to_layout(&mut layout)?;
        tracing::debug!("actual layout: {layout:?}");

        match run_trial_evaluation(&args, ask_output.number, &layout) {
            Ok(metrics) => {
                tuner.tell(ask_output.number, &metrics)?;
            }
            Err(e) => {
                eprintln!("failed to VMAF evaluation: {e:?}",);
                tuner.tell_fail(ask_output.number)?;
            }
        }
        eprintln!();

        displayed_best_trials = display_best_trials_if_updated(&args, &mut tuner, false)?;
    }

    if !displayed_best_trials {
        // 直前で表示していないなら、最後に結果を表示する
        display_best_trials_if_updated(&args, &mut tuner, true)?;
    }

    Ok(())
}

fn trial_dir(args: &Args, trial_number: usize) -> PathBuf {
    args.tune_working_dir()
        .join(&args.name)
        .join(format!("trial-{}", trial_number))
}

fn run_trial_evaluation(
    args: &Args,
    trial_number: usize,
    layout: &JsonValue,
) -> crate::Result<TrialValues> {
    // トライアルの作業用ディレクトリを作成
    let trial_dir = trial_dir(args, trial_number);
    std::fs::create_dir_all(&trial_dir).map_err(|e| {
        crate::Error::new(format!(
            "failed to create trial directory {}: {e}",
            trial_dir.display()
        ))
    })?;
    let trial_dir = trial_dir.canonicalize()?;

    // レイアウトファイルを作成
    let layout_file_path = trial_dir.join("layout.jsonc");
    let layout_json = crate::json::to_pretty_string(layout);
    std::fs::write(&layout_file_path, layout_json).map_err(|e| {
        crate::Error::new(format!(
            "failed to write layout file {}: {e}",
            layout_file_path.display(),
        ))
    })?;

    // hisui vmaf コマンドを実行する。
    // 自分自身の vmaf サブコマンドを呼ぶので、PATH 上の別の hisui を誤って拾わないよう
    // current_exe() で実行中のバイナリを直接指定する。
    let hisui_exe = std::env::current_exe()
        .map_err(|e| crate::Error::new(format!("failed to resolve current executable: {e}")))?;
    let mut cmd = Command::new(&hisui_exe);
    // 共通フラグ --emit-exit-metrics を子プロセスへ env 経由で継承させない。
    // 子の hisui vmaf は結果 JSON のみを stdout に出すことを親が前提とする
    // (tune 親はその stdout を nojson でパースする) ため、env 継承による
    // 終了時メトリクス行の混入を防ぐ。
    // NOTE: この env_remove を削除すると、親プロセスで HISUI_EMIT_EXIT_METRICS=1
    // が設定された状態で tune を実行した際に子 vmaf の stdout 末尾に metrics 行が
    // 混入し、tune 親の nojson パースがサイレントに壊滅する。削除厳禁。
    cmd.env_remove("HISUI_EMIT_EXIT_METRICS");
    cmd.arg("vmaf")
        .arg("--layout-file")
        .arg(&layout_file_path)
        .arg("--frame-count")
        .arg(args.frame_count.to_string())
        .arg("--reference-yuv-file")
        .arg(trial_dir.join("reference.yuv"))
        .arg("--distorted-yuv-file")
        .arg(trial_dir.join("distorted.yuv"))
        .arg(&args.root_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if let Some(openh264_path) = &args.openh264 {
        cmd.arg("--openh264").arg(openh264_path);
    }
    if let Some(timeout) = &args.trial_timeout {
        cmd.arg(format!("--timeout={}", timeout.as_secs_f32()));
    }

    if let Some(cores) = &args.max_cpu_cores {
        cmd.arg("--max-cpu-cores").arg(cores.to_string());
    }
    eprintln!();
    eprintln!("=== EVALUATE PARAMETERS ===");
    eprintln!("$ {cmd:?}");
    eprintln!();

    let result = cmd
        .output()
        .map_err(|e| crate::Error::new(format!("failed to execute `$ hisui vmaf` command: {e}")))
        .and_then(|output| {
            output
                .status
                .success()
                .then_some(())
                .ok_or_else(|| crate::Error::new("`$ hisui vmaf` command failed"))?;
            Ok(output)
        });

    // YUV ファイルはサイズが大きいので不要になったら削除する
    for name in ["reference.yuv", "distorted.yuv"] {
        let path = trial_dir.join(name);
        if path.exists()
            && let Err(e) = std::fs::remove_file(&path)
        {
            eprintln!("[WARN] failed to remove file {}: {e}", path.display());
        }
    }
    let output = result?;

    // 出力結果をパース
    let stdout = String::from_utf8(output.stdout)?;
    let result = nojson::RawJson::parse(&stdout)?;
    let object = result.value();

    // メトリクスを抽出
    let vmaf_mean: f64 = object.to_member("vmaf_mean")?.required()?.try_into()?;
    let elapsed_seconds: f64 = object
        .to_member("elapsed_seconds")?
        .required()?
        .try_into()?;

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
    std::fs::write(trial_dir.join("metrics.json"), &stdout)?;

    Ok(TrialValues {
        elapsed_seconds,
        vmaf_mean,
    })
}

fn display_best_trials_if_updated(
    args: &Args,
    tuner: &mut Tuner,
    force: bool,
) -> crate::Result<bool> {
    let (updated, mut best_trials) = tuner.get_best_trials()?;
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

        let layout_file_path = trial_dir(args, trial.number).join("layout.jsonc");

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
