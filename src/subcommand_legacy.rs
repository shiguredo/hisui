use std::{num::NonZeroUsize, path::PathBuf};

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{
    audio::{DEFAULT_AAC_BITRATE, DEFAULT_OPUS_BITRATE},
    composer::{ComposeResult, Composer},
    encoder_libvpx,
    layout::Layout,
    metadata::RecordingMetadata,
    types::CodecName,
    video::FrameRate,
};

pub fn run(args: noargs::RawArgs) -> noargs::Result<()> {
    let args = Args::parse(args)?;
    if let Some(help) = args.get_help() {
        print!("{help}");
        return Ok(());
    }
    Runner::new(args).run().or_fail()?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Args {
    pub help: Option<String>,
    pub in_metadata_file: Option<PathBuf>,
    pub out_video_codec: CodecName,
    pub out_audio_codec: CodecName,
    pub out_video_frame_rate: FrameRate,
    pub out_file: Option<PathBuf>,
    pub out_stats_file: Option<PathBuf>,
    pub max_columns: NonZeroUsize,
    pub libvpx_cq_level: usize,
    pub libvpx_min_q: usize,
    pub libvpx_max_q: usize,
    pub out_opus_bit_rate: NonZeroUsize,
    pub out_aac_bit_rate: NonZeroUsize,
    pub openh264: Option<PathBuf>,
    pub audio_only: bool,
    pub show_progress_bar: bool,
    pub layout: Option<PathBuf>,
    pub cpu_cores: Option<usize>,
}

impl Args {
    pub fn parse(mut args: noargs::RawArgs) -> noargs::Result<Self> {
        let in_metadata_file = noargs::opt("in-metadata-file")
            .short('f')
            .ty("PATH")
            .example("/path/to/report-$RECORDING_ID.json")
            .doc(
                r#"Sora が生成した録画メタデータファイルを指定して合成を実行します

録画メタデータファイル指定時には、以下のレイアウトで合成が行われます:
{
  "resolution": ${ セルサイズを 320x240 として自動で計算 },
  "audio_sources": [ ${ 録画メタデータ内の全てのソース } ],
  "video_layout": {"main": {
    "max_columns": [ ${ `--max-columns` 引数の値 } ],
    "video_sources": [ ${ 録画メタデータ内の全てのソース } ]
  }}
}

NOTE: `--layout` 引数が指定されている場合にはこの引数は無視されます"#,
            )
            .take(&mut args)
            .present_and_then(|a| a.value().parse())?;
        let layout = noargs::opt("layout")
            .ty("PATH")
            .doc("Hisui のレイアウトファイルを指定して合成を実行します")
            .take(&mut args)
            .present_and_then(|a| a.value().parse())?;
        let out_file = noargs::opt("out-file")
            .ty("PATH")
            .doc(concat!(
                "合成結果を保存するファイルのパス\n",
                "\n",
                "この引数が未指定の場合には、 `--in-metadata-file` ないし `--layout` 引数で\n",
                "指定した入力ファイルと同じディレクトリに `output.mp4` という名前で保存されます\n",
                "（ただし、音声のみの場合には `output.mp4a` という名前になります)",
            ))
            .take(&mut args)
            .present_and_then(|a| a.value().parse())?;
        let out_video_codec = noargs::opt("out-video-codec")
            .ty("VP8|VP9|H264|H265|AV1")
            .default("VP9")
            .doc("映像のエンコードコーデック")
            .take(&mut args)
            .then(|a| CodecName::parse_video(a.value()))?;
        let out_audio_codec = noargs::opt("out-audio-codec")
            .ty("Opus|AAC")
            .default("Opus")
            .doc(concat!(
                "音声のエンコードコーデック\n\n",
                "NOTE:\n",
                "  AAC は以下の場合にのみ利用可能です:\n",
                "  - macOS\n",
                "  - FDK-AAC を有効にして自前ビルドした Hisui (`--feature fdk-aac`)\n",
            ))
            .take(&mut args)
            .then(|a| CodecName::parse_audio(a.value()))?;
        let out_video_frame_rate = noargs::opt("out-video-frame-rate")
            .ty("INTEGER|RATIONAL")
            .default("25")
            .doc("合成後の映像のフレームーレート")
            .take(&mut args)
            .then(|a| a.value().parse())?;
        let max_columns = noargs::opt("max-columns")
            .ty("POSITIVE_INTEGER")
            .default("3")
            .doc(concat!(
                "入力映像を配置するグリッドの最大カラム数\n",
                "（レイアウトファイルの \"max_column\" フィールドに対応する引数）\n",
                "\n",
                "NOTE: `--layout` 引数指定時には、この引数は無視されます"
            ))
            .take(&mut args)
            .then(|a| a.value().parse())?;
        let audio_only = noargs::flag("audio-only")
            .doc(concat!(
                "音声のみを合成対象にします\n",
                "\n",
                "NOTE: `--layout` 引数指定時には、この引数は無視されます\n"
            ))
            .take(&mut args)
            .is_present();
        let openh264 = noargs::opt("openh264")
            .ty("PATH")
            .doc(concat!(
                "OpenH264 の共有ライブラリのパス\n",
                "\n",
                "この引数を指定すると OpenH264 を用いて H.264 の",
                "エンコードとデコードが行われるようになります\n",
                "NOTE: H.264 を扱える他のエンジンが存在する場合でも OpenH264 が優先されます\n"
            ))
            .take(&mut args)
            .present_and_then(|a| a.value().parse())?;
        let libvpx_cq_level = noargs::opt("libvpx-cq-level")
            .ty("NON_NEGATIVE_INTEGER")
            .default(encoder_libvpx::DEFAULT_CQ_LEVEL)
            .doc(concat!(
                "libvpx のエンコードパラメータ\n",
                "\n",
                "`vpx_codec_control_(..., VP8E_SET_CQ_LEVEL, ...)` ",
                "関数呼び出しの引数として渡されます\n",
            ))
            .take(&mut args)
            .then(|a| a.value().parse())?;
        let libvpx_min_q = noargs::opt("libvpx-min-q")
            .ty("NON_NEGATIVE_INTEGER")
            .default(encoder_libvpx::DEFAULT_MIN_Q)
            .doc(concat!(
                "libvpx のエンコードパラメータ\n",
                "\n",
                "`vpx_codec_enc_cfg` 構造体の `rc_min_quantizer` に設定されます\n",
            ))
            .take(&mut args)
            .then(|a| a.value().parse())?;
        let libvpx_max_q = noargs::opt("libvpx-max-q")
            .ty("NON_NEGATIVE_INTEGER")
            .default(encoder_libvpx::DEFAULT_MAX_Q)
            .doc(concat!(
                "libvpx のエンコードパラメータ\n",
                "\n",
                "`vpx_codec_enc_cfg` 構造体の `rc_max_quantizer` に設定されます\n",
            ))
            .take(&mut args)
            .then(|a| a.value().parse())?;
        let out_opus_bit_rate = noargs::opt("out-opus-bit-rate")
            .ty("BPS")
            .default(DEFAULT_OPUS_BITRATE)
            .doc("Opus でエンコードする際のビットレート")
            .take(&mut args)
            .then(|a| a.value().parse())?;
        let out_aac_bit_rate = noargs::opt("out-aac-bit-rate")
            .ty("BPS")
            .default(DEFAULT_AAC_BITRATE)
            .doc("AAC でエンコードする際のビットレート")
            .take(&mut args)
            .then(|a| a.value().parse())?;
        let show_progress_bar = noargs::opt("show-progress-bar")
            .ty("true|false")
            .default("true")
            .doc("true が指定された場合には合成の進捗を表示します")
            .take(&mut args)
            .then(|a| a.value().parse())?;
        let cpu_cores = noargs::opt("max-cpu-cores")
            .short('c')
            .ty("INTEGER")
            .doc(concat!(
                "合成処理を行うプロセスが使用するコア数の上限を指定します\n",
                "（未指定時には上限なし）\n",
                "\n",
                "NOTE: macOS ではこの引数は無視されます",
            ))
            .take(&mut args)
            .present_and_then(|a| a.value().parse())?;
        let out_stats_file = noargs::opt("out-stats-file")
            .ty("PATH")
            .doc("合成実行中に集めた統計情報 JSON の出力先ファイル")
            .take(&mut args)
            .present_and_then(|a| a.value().parse())?;

        // 以降は legacy 版のみが対応している引数群
        // （当面は残しておいて、どこかの段階で引数自体を削除する）
        if noargs::flag("video-codec-engines")
            .doc("OBSOLETE: 2025.1.0 以降では指定しても無視されます")
            .take(&mut args)
            .is_present()
        {
            // まだロガーのセットアップが行われていないので eprintln!() で直接出力する
            // （以降も同様）
            eprintln!(
                "[WARN] `--video-codec-engines` is obsolete (please use `list-codecs` command instead)\n"
            );
        }
        if noargs::opt("mp4-muxer")
            .ty("IGNORED")
            .doc("OBSOLETE: 2025.1.0 以降では指定しても無視されます")
            .take(&mut args)
            .is_present()
        {
            eprintln!("[WARN] `--mp4-muxer` is obsolete\n");
        }
        if noargs::opt("dir-for-faststart")
            .ty("IGNORED")
            .doc("OBSOLETE: 2025.1.0 以降では指定しても無視されます")
            .take(&mut args)
            .is_present()
        {
            eprintln!("[WARN] `--dir-for-faststart` is obsolete\n");
        }
        if noargs::opt("out-container")
            .ty("IGNORED")
            .doc("OBSOLETE: 2025.1.0 以降では指定しても無視されます")
            .take(&mut args)
            .is_present()
        {
            eprintln!("[WARN] `--out-container` is obsolete\n");
        }
        if noargs::opt("h264-encoder")
            .ty("IGNORED")
            .doc("OBSOLETE: 2025.1.0 以降では指定しても無視されます")
            .take(&mut args)
            .is_present()
        {
            eprintln!("[WARN] `--h264-encoder` is obsolete\n");
        }

        if in_metadata_file.is_none() && layout.is_none() {
            // 最低限必要な引数が指定されていない場合にはヘルプを表示する
            args.metadata_mut().help_mode = true;
        }

        Ok(Self {
            in_metadata_file,
            layout,
            out_file,
            out_video_codec,
            out_audio_codec,
            out_video_frame_rate,
            max_columns,
            audio_only,
            openh264,
            libvpx_cq_level,
            libvpx_min_q,
            libvpx_max_q,
            out_opus_bit_rate,
            out_aac_bit_rate,
            show_progress_bar,
            cpu_cores,
            out_stats_file,
            help: args.finish()?,
        })
    }

    pub fn get_help(&self) -> Option<&String> {
        self.help.as_ref()
    }
}

#[derive(Debug)]
pub struct Runner {
    args: Args,
}

impl Runner {
    pub fn new(args: Args) -> Self {
        Self { args }
    }

    pub fn run(&mut self) -> orfail::Result<()> {
        // レイアウトを準備
        let mut layout = self.create_layout().or_fail()?;
        log::debug!("layout: {layout:?}");

        // レガシーではエンコードパラメータの JSON 経由での指定には非対応
        layout.encode_params = Default::default();
        layout.encode_params.libvpx_vp8 = Some(shiguredo_libvpx::EncoderConfig {
            max_quantizer: self.args.libvpx_max_q,
            min_quantizer: self.args.libvpx_min_q,
            cq_level: self.args.libvpx_cq_level,
            ..Default::default()
        });
        layout.encode_params.libvpx_vp9 = Some(shiguredo_libvpx::EncoderConfig {
            max_quantizer: self.args.libvpx_max_q,
            min_quantizer: self.args.libvpx_min_q,
            cq_level: self.args.libvpx_cq_level,
            ..Default::default()
        });

        // 必要に応じて openh264 の共有ライブラリを読み込む
        let openh264_lib =
            if let Some(path) = self.args.openh264.as_ref().filter(|_| layout.has_video()) {
                Some(Openh264Library::load(path).or_fail()?)
            } else {
                None
            };

        // 変換後のファイルのパスを決定
        let out_file_path = if let Some(path) = self.args.out_file.clone() {
            path
        } else if !layout.has_video() && layout.has_audio() {
            layout.base_path.join("output.mp4a")
        } else {
            layout.base_path.join("output.mp4")
        };

        // Composer を作成して設定
        let mut composer = Composer::new(layout);
        composer.video_codec = self.args.out_video_codec;
        composer.audio_codec = self.args.out_audio_codec;
        composer.openh264_lib = openh264_lib;
        composer.show_progress_bar = self.args.show_progress_bar;
        composer.max_cpu_cores = self.args.cpu_cores;
        composer.stats_file_path = self.args.out_stats_file.clone();
        composer.out_aac_bit_rate = self.args.out_aac_bit_rate;
        composer.out_opus_bit_rate = self.args.out_opus_bit_rate;

        // 合成を実行
        let ComposeResult { stats: _, success } = composer.compose(&out_file_path).or_fail()?;

        if !success {
            // エラー発生時は終了コードを変える
            std::process::exit(1);
        }

        Ok(())
    }

    fn create_layout(&self) -> orfail::Result<Layout> {
        if let Some(layout_file_path) = &self.args.layout {
            let layout_json = std::fs::read_to_string(layout_file_path)
                .or_fail_with(|e| format!("failed to read {}: {e}", layout_file_path.display()))?;
            let base_path = layout_file_path.parent().or_fail()?.to_path_buf();
            Layout::from_layout_json(
                base_path,
                layout_file_path,
                &layout_json,
                self.args.out_video_frame_rate,
            )
            .or_fail()
        } else if let Some(report_file_path) = &self.args.in_metadata_file {
            let report = RecordingMetadata::from_file(report_file_path).or_fail()?;
            log::debug!("loaded recording report: {report:?}");
            Layout::from_recording_report(
                report_file_path,
                &report,
                self.args.audio_only,
                self.args.max_columns.get(),
                self.args.out_video_frame_rate,
            )
            .or_fail()
        } else {
            // 引数バリデーションによってここには来ない
            unreachable!()
        }
    }
}
