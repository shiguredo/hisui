use hisui::{
    command_line_args::{Args, SubCommand},
    logger::Logger,
    runner::Runner,
};

const HELP_FLAG: noargs::FlagSpec = noargs::HELP_FLAG
    .doc("このヘルプメッセージを表示します ('--help' なら詳細、'-h' なら簡易版を表示)");

const VERSION_FLAG: noargs::FlagSpec = noargs::VERSION_FLAG.doc("バージョン番号を表示します");

const VERBOSE_FLAG: noargs::FlagSpec =
    noargs::flag("verbose").doc("警告未満のログメッセージも出力します");

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

    // if noargs::cmd("inspect").take(&mut args).is_present() {
    // } else if noargs::cmd("legacy").take(&mut args).is_present() {
    // }

    let args = Args::parse(args)?;

    if let Some(text) = args.get_help() {
        print!("{text}");
        return Ok(());
    }

    match args.sub_command {
        Some(SubCommand::Inspect {
            input_file,
            decode,
            openh264,
        }) => hisui::subcommand_inspect::run(input_file, decode, openh264)?,
        None => Runner::new(args).run()?,
    }
    Ok(())
}
