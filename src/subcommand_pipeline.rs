use std::path::PathBuf;

use orfail::OrFail;

use crate::media::MediaStreamNameRegistry;
use crate::pipeline::PipelineComponent;
use crate::scheduler::Scheduler;
use crate::stats::Stats;

#[derive(Debug)]
struct Args {
    pipeline_file_path: PathBuf,
}

impl Args {
    fn parse(raw_args: &mut noargs::RawArgs) -> noargs::Result<Self> {
        Ok(Self {
            pipeline_file_path: noargs::opt("pipeline-file")
                .short('p')
                .ty("PATH")
                .env("HISUI_PIPELINE_FILE_PATH")
                .example("/path/to/pipeline.jsonc")
                .doc("実行するパイプラインを定義したJSONファイルのパスを指定します")
                .take(raw_args)
                .then(|a| a.value().parse())?,
        })
    }
}

#[derive(Debug)]
struct PipelineDefinition {
    pipeline: Vec<PipelineComponent>,
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for PipelineDefinition {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let obj = crate::json::JsonObject::new(value)?;
        Ok(Self {
            pipeline: obj.get_required("pipeline")?,
        })
    }
}

pub fn run(mut raw_args: noargs::RawArgs) -> noargs::Result<()> {
    let args = Args::parse(&mut raw_args)?;
    if let Some(help) = raw_args.finish()? {
        print!("{help}");
        return Ok(());
    }

    // パイプライン定義ファイルを読み込み
    let pipeline_def: PipelineDefinition = crate::json::parse_file(&args.pipeline_file_path)?;
    log::debug!("Pipeline definition: {pipeline_def:?}");

    // メディアストリーム名レジストリを初期化
    let mut registry = MediaStreamNameRegistry::new();

    // スケジューラーを作成
    let mut scheduler = Scheduler::new();

    // パイプライン内の各コンポーネントをプロセッサに変換してスケジューラーに登録
    for component in &pipeline_def.pipeline {
        log::debug!("Creating processor for component: {component:?}");
        let processor = component.create_processor(&mut registry).or_fail()?;
        scheduler.register(processor).or_fail()?;
    }

    log::info!("Starting pipeline execution...");
    let start_time = std::time::Instant::now();

    // スケジューラーを実行
    let stats = scheduler.run().or_fail()?;

    let elapsed = start_time.elapsed();
    log::info!(
        "Pipeline execution finished in {:.2}s",
        elapsed.as_secs_f32()
    );

    // 結果を JSON で出力
    crate::json::pretty_print(nojson::json(|f| {
        f.object(|f| {
            f.member("pipeline_file_path", &args.pipeline_file_path)?;
            f.member("elapsed_seconds", elapsed.as_secs_f32())?;
            f.member("component_count", pipeline_def.pipeline.len())?;

            // 統計情報の出力
            print_pipeline_stats_summary(f, &stats)?;

            Ok(())
        })
    }))
    .or_fail()?;

    Ok(())
}

fn print_pipeline_stats_summary(
    f: &mut nojson::JsonObjectFormatter<'_, '_, '_>,
    stats: &Stats,
) -> std::fmt::Result {
    f.member("total_processors", stats.processors.len())?;
    f.member("worker_thread_count", stats.worker_threads.len())?;

    let total_processing_time = stats
        .worker_threads
        .iter()
        .map(|w| w.total_processing_duration.get())
        .sum::<std::time::Duration>();

    if !total_processing_time.is_zero() {
        f.member(
            "total_processing_seconds",
            total_processing_time.as_secs_f64(),
        )?;
    }

    let total_waiting_time = stats
        .worker_threads
        .iter()
        .map(|w| w.total_waiting_duration.get())
        .sum::<std::time::Duration>();

    if !total_waiting_time.is_zero() {
        f.member("total_waiting_seconds", total_waiting_time.as_secs_f64())?;
    }

    Ok(())
}
