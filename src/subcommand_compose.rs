use std::{collections::BTreeSet, num::NonZeroUsize, path::PathBuf};

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{
    composer::Composer,
    layout::{DEFAULT_LAYOUT_JSON, Layout},
    stats::StatsEntry,
};

#[derive(Debug)]
struct Args {
    layout_file_path: Option<PathBuf>,
    output_file_path: Option<PathBuf>,
    stats_file_path: Option<PathBuf>,
    openh264: Option<PathBuf>,
    no_progress_bar: bool,
    worker_threads: NonZeroUsize,
    root_dir: PathBuf,
}

impl Args {
    fn parse(raw_args: &mut noargs::RawArgs) -> noargs::Result<Self> {
        Ok(Self {
            layout_file_path: noargs::opt("layout-file")
                .short('l')
                .ty("PATH")
                .env("HISUI_LAYOUT_FILE_PATH")
                .default("HISUI_REPO/layout-examples/compose-default.jsonc")
                .doc("合成に使用するレイアウトファイルを指定します")
                .take(raw_args)
                .then(crate::arg_utils::parse_non_default_opt)?,
            output_file_path: noargs::opt("output-file")
                .short('o')
                .ty("PATH")
                .default("ROOT_DIR/output.mp4")
                .doc("合成結果を保存するファイルを指定します")
                .take(raw_args)
                .then(crate::arg_utils::parse_non_default_opt)?,
            stats_file_path: noargs::opt("stats-file")
                .short('s')
                .ty("PATH")
                .doc("合成中に収集した統計情報 (JSON) を保存するファイルを指定します")
                .take(raw_args)
                .present_and_then(|a| a.value().parse())?,
            openh264: noargs::opt("openh264")
                .ty("PATH")
                .env("HISUI_OPENH264_PATH")
                .doc("OpenH264 の共有ライブラリのパスを指定します")
                .take(raw_args)
                .present_and_then(|a| a.value().parse())?,
            no_progress_bar: noargs::flag("no-progress-bar")
                .short('P')
                .doc("指定された場合は、合成の進捗を非表示にします")
                .take(raw_args)
                .is_present(),
            worker_threads: noargs::opt("thread-count")
                .short('T')
                .ty("INTEGER")
                .default("1")
                .env("HISUI_THREAD_COUNT")
                .doc(concat!(
                    "合成処理に使用するワーカースレッド数を指定します\n",
                    "\n",
                    "なおこれはあくまでも Hisui 自体が起動するスレッドの数であり、\n",
                    "各エンコーダーやデコーダーが内部で起動するスレッドには関与しません",
                ))
                .take(raw_args)
                .then(|a| a.value().parse())?,
            root_dir: noargs::arg("ROOT_DIR")
                .example("/path/to/archive/RECORDING_ID/")
                .doc(concat!(
                    "合成処理を行う際のルートディレクトリを指定します\n",
                    "\n",
                    "レイアウトファイル内に記載された相対パスの基点は、",
                    "このディレクトリとなります。\n",
                    "また、レイアウト内で、",
                    "このディレクトリの外のファイルが参照された場合にはエラーとなります。"
                ))
                .take(raw_args)
                .then(crate::arg_utils::validate_existing_directory_path)?,
        })
    }
}

pub fn run(mut raw_args: noargs::RawArgs) -> noargs::Result<()> {
    let args = Args::parse(&mut raw_args)?;
    if let Some(help) = raw_args.finish()? {
        print!("{help}");
        return Ok(());
    }

    // レイアウトを準備
    let layout = Layout::from_layout_json_file_or_default(
        args.root_dir.clone(),
        args.layout_file_path.as_deref(),
        DEFAULT_LAYOUT_JSON,
    )
    .or_fail()?;
    tracing::debug!("layout: {layout:?}");

    // 必要に応じて openh264 の共有ライブラリを読み込む
    let openh264_lib = if let Some(path) = args.openh264.as_ref().filter(|_| layout.has_video()) {
        Some(Openh264Library::load(path).or_fail()?)
    } else {
        None
    };

    // 出力ファイルパスを決定
    let output_file_path = args
        .output_file_path
        .unwrap_or_else(|| args.root_dir.join("output.mp4"));

    // Composer を作成して設定
    let mut composer = Composer::new(layout);
    composer.openh264_lib = openh264_lib;
    composer.show_progress_bar = !args.no_progress_bar;
    composer.worker_threads = args.worker_threads;
    composer.stats_file_path = args.stats_file_path;

    // 合成を実行
    let result = composer.compose(&output_file_path).or_fail()?;
    let entries = result
        .stats
        .entries()
        .map_err(|e| orfail::Failure::new(e.to_string()))?;

    if !result.success {
        // エラー発生時は終了コードを変える
        std::process::exit(1);
    }

    crate::json::pretty_print(nojson::json(|f| {
        f.object(|f| {
            if let Some(path) = &args.layout_file_path {
                f.member("layout_file_path", path)?;
            }
            if let Some(path) = &composer.stats_file_path {
                f.member("stats_file_path", path)?;
            }
            f.member("input_root_dir", &args.root_dir)?;
            print_input_stats_summary(f, &entries)?;
            f.member("output_file_path", &output_file_path)?;
            print_output_stats_summary(f, &entries)?;
            print_time_stats_summary(f, &entries, result.elapsed_duration.as_secs_f64())?;

            Ok(())
        })
    }))
    .or_fail()?;

    Ok(())
}

fn print_input_stats_summary(
    f: &mut nojson::JsonObjectFormatter<'_, '_, '_>,
    entries: &[StatsEntry],
) -> std::fmt::Result {
    // NOTE: 個別の reader / decoder の情報を出すと JSON の要素数が可変かつ挙動になる可能性があるので省く
    //（その情報が必要なら stats ファイルを出力して、そっちを参照するのがいい）
    let count = count_processors_by_types(entries, &["mp4_audio_reader", "webm_audio_reader"]);
    if count > 0 {
        f.member("input_audio_source_count", count)?;
    }

    let count = count_processors_by_types(entries, &["mp4_video_reader", "webm_video_reader"]);
    if count > 0 {
        f.member("input_video_source_count", count)?;
    }

    Ok(())
}

fn print_output_stats_summary(
    f: &mut nojson::JsonObjectFormatter<'_, '_, '_>,
    entries: &[StatsEntry],
) -> std::fmt::Result {
    let Some(writer_id) = find_first_processor_id_by_type(entries, "mp4_writer") else {
        return Ok(());
    };

    if let Some(codec) = find_string_metric_by_processor(entries, &writer_id, "audio_codec") {
        f.member("output_audio_codec", codec)?;
        if let Some(engine) = find_first_string_metric_by_type(entries, "audio_encoder", "engine") {
            f.member("output_audio_encode_engine", engine)?;
        }
        if let Some(duration_seconds) =
            find_numeric_metric_by_processor(entries, &writer_id, "total_audio_track_seconds")
        {
            f.member("output_audio_duration_seconds", duration_seconds)?;
            if duration_seconds > 0.0
                && let Some(byte_size) = find_numeric_metric_by_processor(
                    entries,
                    &writer_id,
                    "total_audio_sample_data_byte_size",
                )
            {
                let bitrate = (byte_size * 8.0) / duration_seconds;
                f.member("output_audio_bitrate", bitrate as u64)?;
            }
        }
    }
    if let Some(codec) = find_string_metric_by_processor(entries, &writer_id, "video_codec") {
        f.member("output_video_codec", codec)?;
        if let Some(engine) = find_first_string_metric_by_type(entries, "video_encoder", "engine") {
            f.member("output_video_encode_engine", engine)?;
        }
        if let Some(duration_seconds) =
            find_numeric_metric_by_processor(entries, &writer_id, "total_video_track_seconds")
        {
            f.member("output_video_duration_seconds", duration_seconds)?;
            if duration_seconds > 0.0
                && let Some(byte_size) = find_numeric_metric_by_processor(
                    entries,
                    &writer_id,
                    "total_video_sample_data_byte_size",
                )
            {
                let bitrate = (byte_size * 8.0) / duration_seconds;
                f.member("output_video_bitrate", bitrate as u64)?;
            }
        }
    }

    if let Some(width) =
        find_first_numeric_metric_by_type(entries, "video_mixer", "output_video_width")
    {
        f.member("output_video_width", width as usize)?;
    }
    if let Some(height) =
        find_first_numeric_metric_by_type(entries, "video_mixer", "output_video_height")
    {
        f.member("output_video_height", height as usize)?;
    }

    Ok(())
}

fn print_time_stats_summary(
    f: &mut nojson::JsonObjectFormatter<'_, '_, '_>,
    entries: &[StatsEntry],
    elapsed_seconds: f64,
) -> std::fmt::Result {
    let total_audio_decoder_processing_seconds =
        sum_numeric_metric_by_type(entries, "audio_decoder", "total_processing_seconds");
    if total_audio_decoder_processing_seconds > 0.0 {
        f.member(
            "total_audio_decoder_processing_seconds",
            total_audio_decoder_processing_seconds,
        )?;
    }

    let total_video_decoder_processing_seconds =
        sum_numeric_metric_by_type(entries, "video_decoder", "total_processing_seconds");
    if total_video_decoder_processing_seconds > 0.0 {
        f.member(
            "total_video_decoder_processing_seconds",
            total_video_decoder_processing_seconds,
        )?;
    }

    let total_audio_encoder_processing_seconds =
        sum_numeric_metric_by_type(entries, "audio_encoder", "total_processing_seconds");
    if total_audio_encoder_processing_seconds > 0.0 {
        f.member(
            "total_audio_encoder_processing_seconds",
            total_audio_encoder_processing_seconds,
        )?;
    }

    let total_video_encoder_processing_seconds =
        sum_numeric_metric_by_type(entries, "video_encoder", "total_processing_seconds");
    if total_video_encoder_processing_seconds > 0.0 {
        f.member(
            "total_video_encoder_processing_seconds",
            total_video_encoder_processing_seconds,
        )?;
    }

    let total_audio_mixer_processing_seconds =
        sum_numeric_metric_by_type(entries, "audio_mixer", "total_processing_seconds");
    if total_audio_mixer_processing_seconds > 0.0 {
        f.member(
            "total_audio_mixer_processing_seconds",
            total_audio_mixer_processing_seconds,
        )?;
    }

    let total_video_mixer_processing_seconds =
        sum_numeric_metric_by_type(entries, "video_mixer", "total_processing_seconds");
    if total_video_mixer_processing_seconds > 0.0 {
        f.member(
            "total_video_mixer_processing_seconds",
            total_video_mixer_processing_seconds,
        )?;
    }

    f.member("elapsed_seconds", elapsed_seconds)?;

    Ok(())
}

fn count_processors_by_types(entries: &[StatsEntry], processor_types: &[&str]) -> usize {
    let mut processor_ids = BTreeSet::new();
    for entry in entries {
        if entry.metric_name != "error" {
            continue;
        }
        let Some(processor_type) = label_value(entry, "processor_type") else {
            continue;
        };
        if !processor_types.iter().any(|t| t == &processor_type) {
            continue;
        }
        if let Some(processor_id) = label_value(entry, "processor_id") {
            processor_ids.insert(processor_id.to_owned());
        }
    }
    processor_ids.len()
}

fn label_value<'a>(entry: &'a StatsEntry, name: &str) -> Option<&'a str> {
    entry.labels.get(name).map(String::as_str)
}

fn find_first_processor_id_by_type(entries: &[StatsEntry], processor_type: &str) -> Option<String> {
    entries.iter().find_map(|entry| {
        if label_value(entry, "processor_type") != Some(processor_type) {
            return None;
        }
        label_value(entry, "processor_id").map(ToOwned::to_owned)
    })
}

fn find_string_metric_by_processor(
    entries: &[StatsEntry],
    processor_id: &str,
    metric_name: &str,
) -> Option<String> {
    entries.iter().find_map(|entry| {
        if entry.metric_name != metric_name {
            return None;
        }
        if label_value(entry, "processor_id") != Some(processor_id) {
            return None;
        }
        entry.value.as_string()
    })
}

fn find_numeric_metric_by_processor(
    entries: &[StatsEntry],
    processor_id: &str,
    metric_name: &str,
) -> Option<f64> {
    entries.iter().find_map(|entry| {
        if entry.metric_name != metric_name {
            return None;
        }
        if label_value(entry, "processor_id") != Some(processor_id) {
            return None;
        }
        entry.value.as_numeric_f64()
    })
}

fn find_first_string_metric_by_type(
    entries: &[StatsEntry],
    processor_type: &str,
    metric_name: &str,
) -> Option<String> {
    entries.iter().find_map(|entry| {
        if entry.metric_name != metric_name {
            return None;
        }
        if label_value(entry, "processor_type") != Some(processor_type) {
            return None;
        }
        entry.value.as_string()
    })
}

fn find_first_numeric_metric_by_type(
    entries: &[StatsEntry],
    processor_type: &str,
    metric_name: &str,
) -> Option<f64> {
    entries.iter().find_map(|entry| {
        if entry.metric_name != metric_name {
            return None;
        }
        if label_value(entry, "processor_type") != Some(processor_type) {
            return None;
        }
        entry.value.as_numeric_f64()
    })
}

fn sum_numeric_metric_by_type(
    entries: &[StatsEntry],
    processor_type: &str,
    metric_name: &str,
) -> f64 {
    entries
        .iter()
        .filter(|entry| {
            entry.metric_name == metric_name
                && label_value(entry, "processor_type") == Some(processor_type)
        })
        .filter_map(|entry| entry.value.as_numeric_f64())
        .sum()
}
