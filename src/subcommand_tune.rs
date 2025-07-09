use std::{
    num::NonZeroUsize,
    path::PathBuf,
    process::{Command, Stdio},
};

use orfail::OrFail;

use crate::subcommand_vmaf;

const DEFAULT_LAYOUT_JSON: &str = include_str!("../layout-examples/tune-vp8.json");

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
    let n_trials: usize = noargs::opt("n-trials")
        .short('n')
        .ty("INTEGER")
        .default("100")
        .doc("実行する試行回数を指定します")
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
    check_optuna_availability().or_fail()?;
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

    // optuna の study を作る
    let storage_url = format!("sqlite:///{}", tune_working_dir.join("optuna.db").display());
    create_optuna_study(&study_name, &storage_url).or_fail()?;

    Ok(())
}

fn check_optuna_availability() -> orfail::Result<()> {
    let output = Command::new("optuna")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output();

    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(_) => Err(orfail::Failure::new(
            "optuna command failed to execute properly",
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(orfail::Failure::new(
            "optuna command not found. Please install optuna and ensure it's in your PATH",
        )),
        Err(e) => Err(orfail::Failure::new(format!(
            "failed to check optuna availability: {e}"
        ))),
    }
}

fn create_optuna_study(study_name: &str, storage_url: &str) -> orfail::Result<()> {
    let output = Command::new("optuna")
        .arg("create-study")
        .arg("--study-name")
        .arg(study_name)
        .arg("--storage")
        .arg(storage_url)
        .arg("--skip-if-exists")
        // 「エンコード時間の最小化」と「VMAF スコアの最大化」
        .arg("--directions")
        .arg("minimize")
        .arg("maximize")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output()
        .or_fail_with(|e| format!("failed to execute optuna create-study command: {e}"))?;

    output.status.success().or_fail()?;
    Ok(())
}
