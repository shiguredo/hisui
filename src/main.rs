use hisui::logger::Logger;

// 共通引数定義
const HELP_FLAG: noargs::FlagSpec = noargs::HELP_FLAG
    .doc("このヘルプメッセージを表示します ('--help' なら詳細、'-h' なら簡易版を表示)");
const VERSION_FLAG: noargs::FlagSpec = noargs::VERSION_FLAG.doc("バージョン番号を表示します");
const VERBOSE_FLAG: noargs::FlagSpec =
    noargs::flag("verbose").doc("警告未満のログメッセージも出力します");

// サブコマンド定義
const INSPECT_COMMAND: noargs::CmdSpec =
    noargs::cmd("inspect").doc("録画ファイルの情報を取得します");
const LIST_CODECS_COMMAND: noargs::CmdSpec =
    noargs::cmd("list-codecs").doc("利用可能なコーデック一覧を表示します");
const COMPOSE_COMMAND: noargs::CmdSpec = noargs::cmd("compose").doc("録画ファイルの合成を行います");
const VMAF_COMMAND: noargs::CmdSpec =
    noargs::cmd("vmaf").doc("VMAF を用いた映像エンコード品質の評価を行います");
const TUNE_COMMAND: noargs::CmdSpec =
    noargs::cmd("tune").doc("Optuna を用いた映像エンコードパラメーターの調整を行います");

// 以降は実験的なサブコマンドの定義
const PIPELINE_COMMAND: noargs::CmdSpec =
    noargs::cmd("pipeline").doc("ユーザー定義のパイプラインを実行します（実験的機能）");
const RTMP_PUBLISH_COMMAND: noargs::CmdSpec = noargs::cmd("rtmp-publish")
    .doc("指定された入力ファイルを RTMP クライアントとして配信します（実験的機能）");
const RTMP_OUTBOUND_ENDPOINT_COMMAND: noargs::CmdSpec = noargs::cmd("rtmp-outbound-endpoint")
    .doc("指定された入力ファイルを RTMP サーバーとして配信します（実験的機能）");

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

    // 実験的機能を有効にするかどうか
    let experimental = noargs::flag("experimental")
        .short('x')
        .doc("実験的機能を有効にします")
        .take(&mut args)
        .is_present();

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
    } else if experimental && PIPELINE_COMMAND.take(&mut args).is_present() {
        hisui::subcommand_pipeline::run(args)?;
    } else if experimental && RTMP_PUBLISH_COMMAND.take(&mut args).is_present() {
        hisui::subcommand_rtmp_publish::run(args)?;
    } else if experimental && RTMP_OUTBOUND_ENDPOINT_COMMAND.take(&mut args).is_present() {
        hisui::subcommand_rtmp_outbound_endpoint::run(args)?;
    } else if let Some(help) = args.finish()? {
        print!("{help}");
    }

    Ok(())
}
