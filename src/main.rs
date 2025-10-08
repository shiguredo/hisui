use hisui::logger::Logger;

const HELP_FLAG: noargs::FlagSpec = noargs::HELP_FLAG
    .doc("このヘルプメッセージを表示します ('--help' なら詳細、'-h' なら簡易版を表示)");
const VERSION_FLAG: noargs::FlagSpec = noargs::VERSION_FLAG.doc("バージョン番号を表示します");
const VERBOSE_FLAG: noargs::FlagSpec =
    noargs::flag("verbose").doc("警告未満のログメッセージも出力します");

const INSPECT_COMMAND: noargs::CmdSpec =
    noargs::cmd("inspect").doc("録画ファイルの情報を取得します");
const LIST_CODECS_COMMAND: noargs::CmdSpec =
    noargs::cmd("list-codecs").doc("利用可能なコーデック一覧を表示します");
const COMPOSE_COMMAND: noargs::CmdSpec = noargs::cmd("compose").doc("録画ファイルの合成を行います");
const VMAF_COMMAND: noargs::CmdSpec =
    noargs::cmd("vmaf").doc("VMAF を用いた映像エンコード品質の評価を行います");
const TUNE_COMMAND: noargs::CmdSpec =
    noargs::cmd("tune").doc("Optuna を用いた映像エンコードパラメーターの調整を行います");
const PIPELINE_COMMAND: noargs::CmdSpec =
    noargs::cmd("pipeline").doc("ユーザー定義のパイプラインを実行します（実験的機能）");

fn main() -> noargs::Result<()> {
    let mut args = noargs::raw_args();
    args.metadata_mut().app_name = env!("CARGO_PKG_NAME");
    args.metadata_mut().app_description = env!("CARGO_PKG_DESCRIPTION");

    // 共通系のフラグ引数は先に処理する
    HELP_FLAG.take_help(&mut args);

    if VERSION_FLAG.take(&mut args).is_present() {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if VERBOSE_FLAG.take(&mut args).is_present() {
        Logger::init(log::LevelFilter::Debug)?;
    } else {
        Logger::init(log::LevelFilter::Warn)?;
    };

    // サブコマンドで分岐する
    if INSPECT_COMMAND.take(&mut args).is_present() {
        hisui::subcommand_inspect::run(args)?;
    } else if LIST_CODECS_COMMAND.take(&mut args).is_present() {
        hisui::subcommand_list_codecs::run(args)?;
    } else if COMPOSE_COMMAND.take(&mut args).is_present() {
        hisui::subcommand_compose::run(args)?;
    } else if VMAF_COMMAND.take(&mut args).is_present() {
        hisui::subcommand_vmaf::run(args)?;
    } else if TUNE_COMMAND.take(&mut args).is_present() {
        hisui::subcommand_tune::run(args)?;
    } else if PIPELINE_COMMAND.take(&mut args).is_present() {
        hisui::subcommand_pipeline::run(args)?;
    } else if let Some(help) = args.finish()? {
        print!("{help}");
    }

    Ok(())
}
