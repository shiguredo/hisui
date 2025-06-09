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

    match args.sub_command {
        Some(SubCommand::Inspect { input_file }) => hisui::subcommand_inspect::run(input_file)?,
        None => Runner::new(args).run()?,
    }
    Ok(())
}
