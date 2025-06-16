use hisui::{
    command_line_args::{Args, SubCommand},
    logger::Logger,
    runner::Runner,
};

fn main() -> noargs::Result<()> {
    let args = Args::parse(std::env::args())?;

    // ロガーの設定はプロセス生存中に一回だけにする必要があるのでRunner の外で行う
    let log_level = if args.verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Warn
    };
    Logger::init(log_level)?;

    if let Some(text) = args.get_help_or_version() {
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
