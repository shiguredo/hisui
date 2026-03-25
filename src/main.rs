use hisui::logger;

fn main() -> noargs::Result<()> {
    let mut args = noargs::raw_args();
    args.metadata_mut().app_name = env!("CARGO_PKG_NAME");
    args.metadata_mut().app_description = env!("CARGO_PKG_DESCRIPTION");

    // 共通系のフラグ引数は先に処理する
    noargs::HELP_FLAG
        .doc("このヘルプメッセージを表示します ('--help' なら詳細、'-h' なら簡易版を表示)")
        .take_help(&mut args);

    if noargs::VERSION_FLAG
        .doc("バージョン番号を表示します")
        .take(&mut args)
        .is_present()
    {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if noargs::flag("verbose")
        .doc("警告未満のログメッセージも出力します")
        .take(&mut args)
        .is_present()
    {
        logger::init(tracing::level_filters::LevelFilter::DEBUG);
    } else {
        logger::init(tracing::level_filters::LevelFilter::WARN);
    };

    // 実験的機能を有効にするかどうか
    let experimental = noargs::flag("experimental")
        .short('x')
        .doc("実験的機能を有効にします")
        .take(&mut args)
        .is_present();

    // サブコマンドで分岐する
    let _ = hisui::subcommand_inspect::try_run(&mut args)?
        || hisui::subcommand_list_codecs::try_run(&mut args)?
        || hisui::sora::recording_subcommand_compose::try_run(&mut args)?
        || hisui::sora::recording_subcommand_vmaf::try_run(&mut args)?
        || hisui::sora::recording_subcommand_tune::try_run(&mut args)?
        || (experimental && hisui::subcommand_obsws::try_run(&mut args)?);

    if let Some(help) = args.finish()? {
        print!("{help}");
    }

    Ok(())
}
