use std::{num::NonZeroUsize, path::PathBuf};

use crate::{types::CodecName, video::FrameRate};

#[derive(Debug, Clone)]
pub struct Args {
    pub version: Option<String>,
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
    pub verbose: bool,
    pub audio_only: bool,
    pub codec_engines: bool,
    pub show_progress_bar: bool,
    pub layout: Option<PathBuf>,
    pub cpu_cores: Option<usize>,
    pub sub_command: Option<SubCommand>,
}

impl Args {
    pub fn parse<I>(args: I) -> noargs::Result<Self>
    where
        I: Iterator<Item = String>,
    {
        let mut args = noargs::RawArgs::new(args);
        args.metadata_mut().app_name = env!("CARGO_PKG_NAME");
        args.metadata_mut().app_description = env!("CARGO_PKG_DESCRIPTION");

        // Hisui がサポートしている引数を処理する
        noargs::HELP_FLAG
            .doc("このヘルプメッセージを表示します ('--help' なら詳細、'-h' なら簡易版を表示)")
            .take_help(&mut args);
        let version = noargs::VERSION_FLAG
            .doc("バージョン番号を表示します")
            .take(&mut args)
            .is_present()
            .then(|| format!("{} {}\n", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")));
        let codec_engines = noargs::flag("codec-engines")
            .doc("利用可能なエンコーダ・デコーダの一覧を JSON 形式で表示します")
            .take(&mut args)
            .is_present();
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
  "video_layout": {
    "max_columns": [ ${ `--max-columns` 引数の値 } ],
    "video_sources": [ ${ 録画メタデータ内の全てのソース } ]
  }
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
            .default("30")
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
            .default("10")
            .doc(concat!(
                "libvpx のエンコードパラメータ\n",
                "\n",
                "`vpx_codec_enc_cfg` 構造体の `rc_min_quantizer` に設定されます\n",
            ))
            .take(&mut args)
            .then(|a| a.value().parse())?;
        let libvpx_max_q = noargs::opt("libvpx-max-q")
            .ty("NON_NEGATIVE_INTEGER")
            .default("50")
            .doc(concat!(
                "libvpx のエンコードパラメータ\n",
                "\n",
                "`vpx_codec_enc_cfg` 構造体の `rc_max_quantizer` に設定されます\n",
            ))
            .take(&mut args)
            .then(|a| a.value().parse())?;
        let out_opus_bit_rate = noargs::opt("out-opus-bit-rate")
            .ty("BPS")
            .default("65536")
            .doc("Opus でエンコードする際のビットレート")
            .take(&mut args)
            .then(|a| a.value().parse())?;
        let out_aac_bit_rate = noargs::opt("out-aac-bit-rate")
            .ty("BPS")
            .default("64000")
            .doc("AAC でエンコードする際のビットレート")
            .take(&mut args)
            .then(|a| a.value().parse())?;
        let show_progress_bar = noargs::opt("show-progress-bar")
            .ty("true|false")
            .default("true")
            .doc("true が指定された場合には合成の進捗を表示します")
            .take(&mut args)
            .then(|a| a.value().parse())?;
        let verbose = noargs::flag("verbose")
            .doc("警告未満のログメッセージも出力します")
            .take(&mut args)
            .is_present();
        let cpu_cores = noargs::opt("cpu-cores")
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
                "[WARN] `--video-codec-engines` is obsolete (please use `--codec-engines` instead)\n"
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

        let sub_command = SubCommand::new(&mut args)?;

        if version.is_none()
            && in_metadata_file.is_none()
            && layout.is_none()
            && !codec_engines
            && sub_command.is_none()
        {
            // 最低限必要な引数が指定されていない場合にはヘルプを表示する
            args.metadata_mut().help_mode = true;
        }

        Ok(Self {
            version,
            codec_engines,
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
            verbose,
            cpu_cores,
            out_stats_file,
            sub_command,
            help: args.finish()?,
        })
    }

    pub fn get_help_or_version(&self) -> Option<&String> {
        self.help.as_ref().or(self.version.as_ref())
    }
}

// TODO(sile): -h の時のサブコマンドのヘルプの見せ方は改善する
//             (e.g., サブコマンドを指定しているかどうかで引数処理を大元で分岐させる）
#[derive(Debug, Clone)]
pub enum SubCommand {
    Inspect { input_file: PathBuf, decode: bool },
}

impl SubCommand {
    fn new(args: &mut noargs::RawArgs) -> noargs::Result<Option<Self>> {
        if args
            .remaining_args()
            .find(|a| matches!(a.1, "inspect"))
            .is_none()
        {
            // サブコマンドなし
            return Ok(None);
        }

        if noargs::cmd("inspect")
            .doc("録画ファイルの情報を取得する")
            .take(args)
            .is_present()
        {
            Ok(Some(Self::Inspect {
                decode: noargs::flag("decode")
                    .doc("指定された場合にはデコードまで行う")
                    .take(args)
                    .is_present(),
                input_file: noargs::arg("INPUT_FILE")
                    .example("/path/to/archive.mp4")
                    .doc("情報取得対象の録画ファイル(.mp4|.webm)")
                    .take(args)
                    .then(|a| a.value().parse())?,
            }))
        } else {
            unreachable!()
        }
    }
}
