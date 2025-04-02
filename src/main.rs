use hisui::{command_line_args::Args, runner::Runner};

fn main() -> noargs::Result<()> {
    let args = Args::parse(std::env::args())?;

    // ロガーの設定はプロセス生存中に一回だけにする必要があるのでRunner の外で行う
    let default_log_level = if args.verbose { "debug" } else { "warning" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_log_level))
        .init();

    Runner::new(args).run()?;
    Ok(())
}
