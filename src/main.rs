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

    let dump_metrics_on_exit: bool = noargs::flag("dump-metrics-on-exit")
        .env("HISUI_DUMP_METRICS_ON_EXIT")
        .doc(concat!(
            "プロセス終了時に内部メトリクスを JSON Lines 形式で標準出力へ 1 行出力します。",
            "標準出力を機械処理する用途では他のサブコマンド出力との混在に注意してください"
        ))
        .take(&mut args)
        .is_present();

    // メトリクスレジストリを main 側で 1 つ作り、`MediaPipeline` を持つ各 subcommand に
    // clone を渡す。`Stats` は内部で `Arc<Mutex<...>>` を共有するため、main 側で保持した
    // ものから末尾で `dump_metrics_to_stdout(&stats)` を呼べば全 processor のメトリクスを
    // 1 行 JSON で書き出せる。
    let stats = hisui::stats::Stats::new();

    // サブコマンドで分岐する
    let matched = hisui::subcommand_inspect::try_run(&mut args, stats.clone())?
        || hisui::subcommand_list_codecs::try_run(&mut args)?
        || hisui::sora::recording_subcommand_compose::try_run(&mut args, stats.clone())?
        || hisui::sora::recording_subcommand_vmaf::try_run(&mut args, stats.clone())?
        || hisui::sora::recording_subcommand_tune::try_run(&mut args)?
        || hisui::subcommand_server::try_run(&mut args, stats.clone())?;

    // フラグ ON かつ subcommand が実際に match し、ヘルプモードでない場合に限り
    // 終了時 dump を出す。`args.finish()` は self を消費するため、help_mode 判定と
    // dump 呼び出しは finish より前に置く。
    let should_dump = dump_metrics_on_exit && matched && !args.metadata().help_mode;
    if should_dump {
        hisui::metrics_dump::dump_metrics_to_stdout(&stats);
    }

    if let Some(help) = args.finish()? {
        print!("{help}");
    }

    Ok(())
}
