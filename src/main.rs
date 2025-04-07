use hisui::{command_line_args::Args, logger::Logger, runner::Runner};

fn main() -> noargs::Result<()> {
    let args = Args::parse(std::env::args())?;

    // ロガーの設定はプロセス生存中に一回だけにする必要があるのでRunner の外で行う
    let log_level = if args.verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Warn
    };
    Logger::init(log_level)?;

    Runner::new(args).run()?;
    Ok(())
}
