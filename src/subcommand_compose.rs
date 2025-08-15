use std::{num::NonZeroUsize, path::PathBuf, time::Duration};

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{
    composer::Composer,
    layout::Layout,
    stats::{ProcessorStats, Stats},
};

const DEFAULT_LAYOUT_JSON: &str = include_str!("../layout-examples/compose-default.json");

#[derive(Debug)]
struct Args {
    layout_file_path: Option<PathBuf>,
    output_file_path: PathBuf,
    stats_file_path: Option<PathBuf>,
    openh264: Option<PathBuf>,
    no_progress_bar: bool,
    max_cpu_cores: Option<NonZeroUsize>,
    root_dir: PathBuf,
}

impl Args {
    fn parse(raw_args: &mut noargs::RawArgs) -> noargs::Result<Self> {
        Ok(Self {
            layout_file_path: noargs::opt("layout-file")
                .short('l')
                .ty("PATH")
                .env("HISUI_LAYOUT_FILE_PATH")
                .doc(concat!(
                    "合成に使用するレイアウトファイルを指定します\n",
                    "\n",
                    "省略された場合には ",
                    "hisui/layout-examples/compose-default.json の内容が使用されます",
                ))
                .take(raw_args)
                .present_and_then(|a| a.value().parse())?,
            output_file_path: noargs::opt("output-file")
                .short('o')
                .ty("PATH")
                .default("output.mp4")
                .doc(concat!(
                    "合成結果を保存するファイルを指定します\n",
                    "\n",
                    "この引数が未指定の場合には ROOT_DIR 引数で\n",
                    "指定したディレクトリに `output.mp4` という名前で保存されます\n",
                    "\n",
                    "相対パスの場合は ROOT_DIR が起点となります"
                ))
                .take(raw_args)
                .then(|a| a.value().parse())?,
            stats_file_path: noargs::opt("stats-file")
                .short('s')
                .ty("PATH")
                .doc(concat!(
                    "合成中に収集した統計情報 (JSON) を保存するファイルを指定します\n",
                    "\n",
                    "相対パスの場合は ROOT_DIR が起点となります"
                ))
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
            max_cpu_cores: noargs::opt("max-cpu-cores")
                .short('c')
                .ty("INTEGER")
                .env("HISUI_MAX_CPU_CORES")
                .doc(concat!(
                    "合成処理を行うプロセスが使用するコア数の上限を指定します\n",
                    "（未指定時には上限なし）\n",
                    "\n",
                    "NOTE: macOS ではこの引数は無視されます",
                ))
                .take(raw_args)
                .present_and_then(|a| a.value().parse())?,
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
    log::debug!("layout: {layout:?}");

    // 必要に応じて openh264 の共有ライブラリを読み込む
    let openh264_lib = if let Some(path) = args.openh264.as_ref().filter(|_| layout.has_video()) {
        Some(Openh264Library::load(path).or_fail()?)
    } else {
        None
    };

    // 出力ファイルパスを決定
    let output_file_path = args.root_dir.join(args.output_file_path);

    // Composer を作成して設定
    let mut composer = Composer::new(layout);
    composer.openh264_lib = openh264_lib;
    composer.show_progress_bar = !args.no_progress_bar;
    composer.max_cpu_cores = args.max_cpu_cores;
    composer.stats_file_path = args.stats_file_path.map(|path| args.root_dir.join(path));

    // 合成を実行
    let result = composer.compose(&output_file_path).or_fail()?;

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
            if let Some(stats) = result.stats.with_lock(|stats| stats.clone()) {
                print_input_stats_summary(f, &stats)?;
            }
            f.member("output_file_path", &output_file_path)?;
            if let Some(stats) = result.stats.with_lock(|stats| stats.clone()) {
                print_output_stats_summary(f, &stats)?;
                print_time_stats_summary(f, &stats)?;
            }

            Ok(())
        })
    }))
    .or_fail()?;

    Ok(())
}

fn print_input_stats_summary(
    f: &mut nojson::JsonObjectFormatter<'_, '_, '_>,
    stats: &Stats,
) -> std::fmt::Result {
    // NOTE: 個別の reader / decoder の情報を出すと JSON の要素数が可変かつ挙動になる可能性があるので省く
    //（その情報が必要なら stats ファイルを出力して、そっちを参照するのがいい）
    let count = stats
        .processors
        .iter()
        .filter(|s| {
            matches!(
                s,
                ProcessorStats::WebmAudioReader(_) | ProcessorStats::Mp4AudioReader(_)
            )
        })
        .count();
    if count > 0 {
        f.member("input_audio_file_count", count)?;
    }

    let count = stats
        .processors
        .iter()
        .filter(|s| {
            matches!(
                s,
                ProcessorStats::WebmVideoReader(_) | ProcessorStats::Mp4VideoReader(_)
            )
        })
        .count();
    if count > 0 {
        f.member("input_video_file_count", count)?;
    }

    Ok(())
}

fn print_output_stats_summary(
    f: &mut nojson::JsonObjectFormatter<'_, '_, '_>,
    stats: &Stats,
) -> std::fmt::Result {
    let Some(ProcessorStats::Mp4Writer(writer)) = stats
        .processors
        .iter()
        .find(|x| matches!(x, ProcessorStats::Mp4Writer(_)))
    else {
        return Ok(());
    };

    if let Some(codec) = writer.audio_codec.get() {
        f.member("output_audio_codec", codec)?;

        for processor in &stats.processors {
            if let ProcessorStats::AudioEncoder(encoder) = processor {
                f.member("output_audio_encoder_name", encoder.engine)?;
                break;
            }
        }

        f.member(
            "output_audio_duration_seconds",
            writer.total_audio_track_seconds.get(),
        )?;

        let duration = writer.total_audio_track_seconds.get().get();
        if !duration.is_zero() {
            let bitrate = (writer.total_audio_sample_data_byte_size.get() as f32 * 8.0)
                / duration.as_secs_f32();
            f.member("output_audio_bitrate", bitrate as u64)?;
        }
    }
    if let Some(codec) = writer.video_codec.get() {
        f.member("output_video_codec", codec)?;

        for processor in &stats.processors {
            if let ProcessorStats::VideoEncoder(encoder) = processor {
                f.member("output_video_encoder_name", encoder.engine)?;
                break;
            }
        }

        f.member(
            "output_video_duration_seconds",
            writer.total_video_track_seconds.get(),
        )?;

        let duration = writer.total_video_track_seconds.get().get();
        if !duration.is_zero() {
            let bitrate = (writer.total_video_sample_data_byte_size.get() as f32 * 8.0)
                / duration.as_secs_f32();
            f.member("output_video_bitrate", bitrate as u64)?;
        }
    }

    for processor in &stats.processors {
        match processor {
            ProcessorStats::AudioMixer(_mixer) => {}
            ProcessorStats::VideoMixer(mixer) => {
                f.member("output_video_width", mixer.output_video_resolution.width)?;
                f.member("output_video_height", mixer.output_video_resolution.height)?;
            }
            _ => {}
        }
    }

    Ok(())
}

fn print_time_stats_summary(
    f: &mut nojson::JsonObjectFormatter<'_, '_, '_>,
    stats: &Stats,
) -> std::fmt::Result {
    let total_audio_decoder_processing_seconds = stats
        .processors
        .iter()
        .filter_map(|decoder| match decoder {
            ProcessorStats::AudioDecoder(audio_decoder) => {
                Some(audio_decoder.total_processing_seconds.get().get())
            }
            _ => None,
        })
        .sum::<Duration>();
    if !total_audio_decoder_processing_seconds.is_zero() {
        f.member(
            "total_audio_decoder_processing_seconds",
            total_audio_decoder_processing_seconds.as_secs_f64(),
        )?;
    }

    let total_video_decoder_processing_seconds = stats
        .processors
        .iter()
        .filter_map(|decoder| match decoder {
            ProcessorStats::VideoDecoder(video_decoder) => {
                Some(video_decoder.total_processing_seconds.get().get())
            }
            _ => None,
        })
        .sum::<Duration>();
    if !total_video_decoder_processing_seconds.is_zero() {
        f.member(
            "total_video_decoder_processing_seconds",
            total_video_decoder_processing_seconds.as_secs_f64(),
        )?;
    }

    let total_audio_encoder_processing_seconds = stats
        .processors
        .iter()
        .filter_map(|encoder| match encoder {
            ProcessorStats::AudioEncoder(audio_encoder) => {
                Some(audio_encoder.total_processing_seconds.get().get())
            }
            _ => None,
        })
        .sum::<Duration>();
    if !total_audio_encoder_processing_seconds.is_zero() {
        f.member(
            "total_audio_encoder_processing_seconds",
            total_audio_encoder_processing_seconds.as_secs_f64(),
        )?;
    }

    let total_video_encoder_processing_seconds = stats
        .processors
        .iter()
        .filter_map(|encoder| match encoder {
            ProcessorStats::VideoEncoder(video_encoder) => {
                Some(video_encoder.total_processing_seconds.get().get())
            }
            _ => None,
        })
        .sum::<Duration>();
    if !total_video_encoder_processing_seconds.is_zero() {
        f.member(
            "total_video_encoder_processing_seconds",
            total_video_encoder_processing_seconds.as_secs_f64(),
        )?;
    }

    let total_audio_mixer_processing_seconds = stats
        .processors
        .iter()
        .filter_map(|mixer| match mixer {
            ProcessorStats::AudioMixer(audio_mixer) => {
                Some(audio_mixer.total_processing_seconds.get().get())
            }
            _ => None,
        })
        .sum::<Duration>();
    if !total_audio_mixer_processing_seconds.is_zero() {
        f.member(
            "total_audio_mixer_processing_seconds",
            total_audio_mixer_processing_seconds.as_secs_f64(),
        )?;
    }

    let total_video_mixer_processing_seconds = stats
        .processors
        .iter()
        .filter_map(|mixer| match mixer {
            ProcessorStats::VideoMixer(video_mixer) => {
                Some(video_mixer.total_processing_seconds.get().get())
            }
            _ => None,
        })
        .sum::<Duration>();
    if !total_video_mixer_processing_seconds.is_zero() {
        f.member(
            "total_video_mixer_processing_seconds",
            total_video_mixer_processing_seconds.as_secs_f64(),
        )?;
    }

    f.member("elapsed_seconds", stats.elapsed_seconds)?;

    Ok(())
}
