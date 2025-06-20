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
const LEGACY_COMMAND: noargs::CmdSpec =
    noargs::cmd("legacy").doc("レガシー Hisui との互換性維持用のコマンドです（省略可能）");

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
    } else if LEGACY_COMMAND.take(&mut args).is_present() {
        hisui::subcommand_legacy::run(args)?;
    } else if args.metadata().help_mode {
        // help_mode=true なので `Ok(None)` が返されることはない
        let help = args.finish()?.expect("infallible");
        print!("{help}");
        return Ok(());
    } else {
        // サブコマンドが指定されておらず、ヘルプ表示モードでもないなら
        // legacy コマンド指定の場合と同じ挙動にする
        hisui::subcommand_legacy::run(args)?;
    }

    Ok(())
}
