use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::{
    metadata::SourceId,
    types::{CodecName, EngineName},
    video::VideoFrame,
};

#[derive(Debug, Clone)]
pub struct SharedStats {
    inner: Arc<Mutex<Stats>>,
}

impl Default for SharedStats {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedStats {
    pub fn new() -> Self {
        let inner = Arc::new(Mutex::new(Stats::default()));
        Self { inner }
    }

    pub fn with_lock<F, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&mut Stats) -> T,
    {
        match self.inner.lock() {
            Ok(mut stats) => Some(f(&mut stats)),
            Err(e) => {
                // 統計情報の更新ができなくても致命的ではないので警告に止める
                log::warn!("failed to acqure stats lock: {e}");
                None
            }
        }
    }

    pub fn save(&self, output_file_path: &Path) {
        self.with_lock(|stats| {
            let json = nojson::json(|f| {
                f.set_indent_size(2);
                f.set_spacing(true);
                f.value(&stats)
            })
            .to_string();
            if let Err(e) = std::fs::write(output_file_path, json) {
                // 統計が出力できなくても全体を失敗扱いにはしない
                log::warn!(
                    "failed to write stats JSON: path={}, reason={e}",
                    output_file_path.display()
                );
            }
        });
    }
}

#[derive(Debug, Default, Clone)]
pub struct Stats {
    /// 全体の合成に要した実時間
    pub elapsed_seconds: Seconds,

    /// 入力関連の統計情報
    pub readers: Vec<ReaderStats>,

    /// デコーダー関連の統計情報
    pub decoders: Vec<DecoderStats>,

    /// 合成関連の統計情報
    pub mixers: Vec<MixerStats>,

    /// エンコーダー関連の統計情報
    pub encoders: Vec<EncoderStats>,

    /// 出力関連の統計情報
    pub writers: Vec<WriterStats>,

    /// TODO: doc
    pub processors: Vec<ProcessorStats>,
}

impl nojson::DisplayJson for Stats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("elapsed_seconds", self.elapsed_seconds)?;
            f.member(
                "processors",
                nojson::array(|f| {
                    for processor in &self.processors {
                        f.element(processor)?;
                    }
                    for processor in &self.readers {
                        f.element(processor)?;
                    }
                    for processor in &self.decoders {
                        f.element(processor)?;
                    }
                    for processor in &self.mixers {
                        f.element(processor)?;
                    }
                    for processor in &self.encoders {
                        f.element(processor)?;
                    }
                    for processor in &self.writers {
                        f.element(processor)?;
                    }
                    Ok(())
                }),
            )?;
            Ok(())
        })
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Seconds(Duration);

impl nojson::DisplayJson for Seconds {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        write!(f.inner_mut(), "{}", self.0.as_secs_f32())
    }
}

impl Seconds {
    pub fn new(elapsed: Duration) -> Self {
        Self(elapsed)
    }

    pub fn elapsed<F, T>(f: F) -> (T, Self)
    where
        F: FnOnce() -> T,
    {
        let start_time = Instant::now();
        let value = f();
        (value, Self::new(start_time.elapsed()))
    }

    pub fn try_elapsed<F, T>(f: F) -> orfail::Result<(T, Self)>
    where
        F: FnOnce() -> orfail::Result<T>,
    {
        let start_time = Instant::now();
        let value = f()?;
        Ok((value, Self::new(start_time.elapsed())))
    }

    pub const fn get(self) -> Duration {
        self.0
    }
}

impl std::ops::AddAssign<Duration> for Seconds {
    fn add_assign(&mut self, rhs: Duration) {
        self.0 += rhs;
    }
}

impl std::ops::AddAssign for Seconds {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl From<Seconds> for f32 {
    fn from(value: Seconds) -> Self {
        value.0.as_secs_f32()
    }
}

#[derive(Debug, Clone)]
pub enum ProcessorStats {
    Mp4AudioReader(Mp4AudioReaderStats),
    Mp4VideoReader(Mp4VideoReaderStats),
    WebmAudioReader(WebmAudioReaderStats),
    WebmVideoReader(WebmVideoReaderStats),
}

impl nojson::DisplayJson for ProcessorStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            ProcessorStats::Mp4AudioReader(stats) => stats.fmt(f),
            ProcessorStats::Mp4VideoReader(stats) => stats.fmt(f),
            ProcessorStats::WebmAudioReader(stats) => stats.fmt(f),
            ProcessorStats::WebmVideoReader(stats) => stats.fmt(f),
        }
    }
}

#[derive(Debug, Clone)]
pub enum MixerStats {
    Audio(AudioMixerStats),
    Video(VideoMixerStats),
}

impl nojson::DisplayJson for MixerStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            MixerStats::Audio(stats) => stats.fmt(f),
            MixerStats::Video(stats) => stats.fmt(f),
        }
    }
}

/// `AudioMixer` 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct AudioMixerStats {
    /// ミキサーの入力 `AudioData` の数
    pub total_input_audio_data_count: u64,

    /// ミキサーが生成した `AudioData` の数
    pub total_output_audio_data_count: u64,

    /// ミキサーが生成した `AudioData` の合計尺
    pub total_output_audio_data_seconds: Seconds,

    /// ミキサーが生成したサンプルの合計数
    pub total_output_sample_count: u64,

    /// ミキサーによって無音補完されたサンプルの合計数
    pub total_output_filled_sample_count: u64,

    /// 出力から除去されたサンプルの合計数
    pub total_trimmed_sample_count: u64,

    /// 合成処理部分に掛かった時間
    pub total_processing_seconds: Seconds,

    /// エラーで中断したかどうか
    pub error: bool,
}

impl nojson::DisplayJson for AudioMixerStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "audio_mixer")?;
            f.member(
                "total_input_audio_data_count",
                self.total_input_audio_data_count,
            )?;
            f.member(
                "total_output_audio_data_count",
                self.total_output_audio_data_count,
            )?;
            f.member(
                "total_output_audio_data_seconds",
                self.total_output_audio_data_seconds,
            )?;
            f.member("total_output_sample_count", self.total_output_sample_count)?;
            f.member(
                "total_output_filled_sample_count",
                self.total_output_filled_sample_count,
            )?;
            f.member(
                "total_trimmed_sample_count",
                self.total_trimmed_sample_count,
            )?;
            f.member("total_processing_seconds", self.total_processing_seconds)?;
            f.member("error", self.error)?;
            Ok(())
        })
    }
}

/// `VideoMixer` 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct VideoMixerStats {
    /// 合成後の映像の解像度
    pub output_video_resolution: VideoResolution,

    /// ミキサーの入力 `VideoFrame` の数
    pub total_input_video_frame_count: u64,

    /// ミキサーが生成した `VideoFrame` の数
    pub total_output_video_frame_count: u64,

    /// ミキサーが生成した `VideoFrame` の合計尺
    pub total_output_video_frame_seconds: Seconds,

    /// 出力から除去された映像フレームの合計数
    pub total_trimmed_video_frame_count: u64,

    /// 合成を省略して前フレームの尺を延長したフレームの数
    pub total_extended_video_frame_count: u64,

    /// 合成処理部分に掛かった時間
    pub total_processing_seconds: Seconds,

    /// エラーで中断したかどうか
    pub error: bool,
}

impl nojson::DisplayJson for VideoMixerStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "video_mixer")?;
            f.member("output_video_resolution", self.output_video_resolution)?;
            f.member(
                "total_input_video_frame_count",
                self.total_input_video_frame_count,
            )?;
            f.member(
                "total_output_video_frame_count",
                self.total_output_video_frame_count,
            )?;
            f.member(
                "total_output_video_frame_seconds",
                self.total_output_video_frame_seconds,
            )?;
            f.member(
                "total_trimmed_video_frame_count",
                self.total_trimmed_video_frame_count,
            )?;
            f.member(
                "total_extended_video_frame_count",
                self.total_extended_video_frame_count,
            )?;
            f.member("total_processing_seconds", self.total_processing_seconds)?;
            f.member("error", self.error)?;
            Ok(())
        })
    }
}

/// エンコーダー関連の統計情報
#[derive(Debug, Clone)]
pub enum EncoderStats {
    Audio(AudioEncoderStats),
    Video(VideoEncoderStats),
}

impl nojson::DisplayJson for EncoderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            EncoderStats::Audio(stats) => stats.fmt(f),
            EncoderStats::Video(stats) => stats.fmt(f),
        }
    }
}

/// 音声エンコーダー用の統計情報
#[derive(Debug, Default, Clone)]
pub struct AudioEncoderStats {
    /// エンコーダーの種類
    pub engine: Option<EngineName>,

    /// コーデック
    pub codec: Option<CodecName>,

    /// エンコーダーで処理された `AudioData` の数
    pub total_audio_data_count: u64,

    /// 処理部分に掛かった時間
    pub total_processing_seconds: Seconds,

    /// エラーで中断したかどうか
    pub error: bool,
}

impl nojson::DisplayJson for AudioEncoderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "audio_encoder")?;
            f.member("engine", self.engine)?;
            f.member("codec", self.codec)?;
            f.member("total_audio_data_count", self.total_audio_data_count)?;
            f.member("total_processing_seconds", self.total_processing_seconds)?;
            f.member("error", self.error)?;
            Ok(())
        })
    }
}

/// 映像エンコーダー用の統計情報
#[derive(Debug, Default, Clone)]
pub struct VideoEncoderStats {
    /// エンコーダーの種類
    pub engine: Option<EngineName>,

    /// コーデック
    pub codec: Option<CodecName>,

    /// エンコード対象の `VideoFrame` の数
    pub total_input_video_frame_count: u64,

    /// 実際にエンコードされた `VideoFrame` の数
    pub total_output_video_frame_count: u64,

    /// 処理部分に掛かった時間
    pub total_processing_seconds: Seconds,

    /// エラーで中断したかどうか
    pub error: bool,
}

impl nojson::DisplayJson for VideoEncoderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "video_encoder")?;
            f.member("engine", self.engine)?;
            f.member("codec", self.codec)?;
            f.member(
                "total_input_video_frame_count",
                self.total_input_video_frame_count,
            )?;
            f.member(
                "total_output_video_frame_count",
                self.total_output_video_frame_count,
            )?;
            f.member("total_processing_seconds", self.total_processing_seconds)?;
            f.member("error", self.error)?;
            Ok(())
        })
    }
}

/// デコーダー関連の統計情報
#[derive(Debug, Clone)]
pub enum DecoderStats {
    Audio(AudioDecoderStats),
    Video(VideoDecoderStats),
}

impl nojson::DisplayJson for DecoderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            DecoderStats::Audio(stats) => stats.fmt(f),
            DecoderStats::Video(stats) => stats.fmt(f),
        }
    }
}

/// 音声デコーダー用の統計情報
#[derive(Debug, Default, Clone)]
pub struct AudioDecoderStats {
    /// 入力ソースの ID
    pub source_id: Option<SourceId>,

    /// デコーダーの種類
    pub engine: Option<EngineName>,

    /// コーデック
    pub codec: Option<CodecName>,

    /// デコーダーで処理された `AudioData` の数
    pub total_audio_data_count: u64,

    /// 処理部分に掛かった時間
    pub total_processing_seconds: Seconds,

    /// エラーで中断したかどうか
    pub error: bool,
}

impl nojson::DisplayJson for AudioDecoderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "audio_decoder")?;
            f.member("source_id", &self.source_id)?;
            f.member("engine", self.engine)?;
            f.member("codec", self.codec)?;
            f.member("total_audio_data_count", self.total_audio_data_count)?;
            f.member("total_processing_seconds", self.total_processing_seconds)?;
            f.member("error", self.error)?;
            Ok(())
        })
    }
}

/// 映像デコーダー用の統計情報
#[derive(Debug, Default, Clone)]
pub struct VideoDecoderStats {
    /// 入力ソースの ID
    pub source_id: Option<SourceId>,

    /// デコーダーの種類
    pub engine: Option<EngineName>,

    /// コーデック
    pub codec: Option<CodecName>,

    /// デコード対象の `VideoFrame` の数
    pub total_input_video_frame_count: u64,

    /// デコードされた `VideoFrame` の数
    pub total_output_video_frame_count: u64,

    /// 処理部分に掛かった時間
    pub total_processing_seconds: Seconds,

    /// 解像度リスト
    pub resolutions: BTreeSet<VideoResolution>,

    /// エラーで中断したかどうか
    pub error: bool,
}

impl nojson::DisplayJson for VideoDecoderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "video_decoder")?;
            f.member("source_id", &self.source_id)?;
            f.member("engine", self.engine)?;
            f.member("codec", self.codec)?;
            f.member(
                "total_input_video_frame_count",
                self.total_input_video_frame_count,
            )?;
            f.member(
                "total_output_video_frame_count",
                self.total_output_video_frame_count,
            )?;
            f.member("total_processing_seconds", self.total_processing_seconds)?;
            f.member("resolutions", &self.resolutions)?;
            f.member("error", self.error)?;
            Ok(())
        })
    }
}

/// 入力関連の統計情報
#[derive(Debug, Clone)]
pub enum ReaderStats {
    WebmAudio(WebmAudioReaderStats),
    WebmVideo(WebmVideoReaderStats),
    Mp4Audio(Mp4AudioReaderStats),
    Mp4Video(Mp4VideoReaderStats),
}

impl nojson::DisplayJson for ReaderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            ReaderStats::WebmAudio(stats) => stats.fmt(f),
            ReaderStats::WebmVideo(stats) => stats.fmt(f),
            ReaderStats::Mp4Audio(stats) => stats.fmt(f),
            ReaderStats::Mp4Video(stats) => stats.fmt(f),
        }
    }
}

/// `Mp4AudioReader` 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct Mp4AudioReaderStats {
    /// 入力ファイルのパス
    pub input_file: PathBuf,

    /// 音声コーデック
    pub codec: Option<CodecName>,

    /// Mp4 のサンプルの数
    pub total_sample_count: u64,

    /// 入力ファイルに含まれる音声トラックの尺
    pub total_track_seconds: Seconds,

    /// 入力処理部分に掛かった時間
    pub total_processing_seconds: Seconds,

    /// エラーで中断したかどうか
    pub error: bool,
}

impl nojson::DisplayJson for Mp4AudioReaderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "mp4_audio_reader")?;
            f.member("input_file", &self.input_file)?;
            f.member("codec", self.codec)?;
            f.member("total_sample_count", self.total_sample_count)?;
            f.member("total_track_seconds", self.total_track_seconds)?;
            f.member("total_processing_seconds", self.total_processing_seconds)?;
            f.member("error", self.error)?;
            Ok(())
        })
    }
}

/// `Mp4VideoReader` 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct Mp4VideoReaderStats {
    /// 入力ファイルのパス
    pub input_file: PathBuf,

    /// 映像コーデック
    pub codec: Option<CodecName>,

    /// 映像の解像度（途中で変わった場合は複数になる）
    pub resolutions: Vec<(u16, u16)>,

    /// Mp4 のサンプルの数
    pub total_sample_count: u64,

    /// 入力ファイルに含まれる映像トラックの尺
    pub total_track_seconds: Seconds,

    /// 入力処理部分に掛かった時間
    pub total_processing_seconds: Seconds,

    /// エラーで中断したかどうか
    pub error: bool,
}

impl nojson::DisplayJson for Mp4VideoReaderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "mp4_video_reader")?;
            f.member("input_file", &self.input_file)?;
            f.member("codec", self.codec)?;
            f.member(
                "resolutions",
                nojson::json(|f| {
                    f.array(|f| {
                        f.elements(self.resolutions.iter().map(|(w, h)| format!("{w}x{h}")))
                    })
                }),
            )?;
            f.member("total_sample_count", self.total_sample_count)?;
            f.member("total_track_seconds", self.total_track_seconds)?;
            f.member("total_processing_seconds", self.total_processing_seconds)?;
            f.member("error", self.error)?;
            Ok(())
        })
    }
}

/// `WebmAudioReader` 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct WebmAudioReaderStats {
    /// 入力ファイルのパス
    pub input_file: PathBuf,

    /// 音声コーデック
    pub codec: Option<CodecName>,

    /// WebM のクラスターの数
    pub total_cluster_count: u64,

    /// WebM のシンプルブロックの数
    pub total_simple_block_count: u64,

    /// 入力ファイルに含まれる音声トラックの尺
    pub total_track_seconds: Seconds,

    /// 入力処理部分に掛かった時間
    pub total_processing_seconds: Seconds,

    /// エラーで中断したかどうか
    pub error: bool,
}

impl nojson::DisplayJson for WebmAudioReaderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "webm_audio_reader")?;
            f.member("input_file", &self.input_file)?;
            f.member("codec", self.codec)?;
            f.member("total_cluster_count", self.total_cluster_count)?;
            f.member("total_simple_block_count", self.total_simple_block_count)?;
            f.member("total_track_seconds", self.total_track_seconds)?;
            f.member("total_processing_seconds", self.total_processing_seconds)?;
            f.member("error", self.error)?;
            Ok(())
        })
    }
}

/// `WebmVideoReader` 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct WebmVideoReaderStats {
    /// 入力ファイルのパス
    pub input_file: PathBuf,

    /// 映像コーデック
    pub codec: Option<CodecName>,

    /// WebM のクラスターの数
    pub total_cluster_count: u64,

    /// WebM のシンプルブロックの数
    pub total_simple_block_count: u64,

    /// 入力ファイルに含まれる映像トラックの尺
    pub total_track_seconds: Seconds,

    /// 入力処理部分に掛かった時間
    pub total_processing_seconds: Seconds,

    /// エラーで中断したかどうか
    pub error: bool,
}

impl nojson::DisplayJson for WebmVideoReaderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "webm_video_reader")?;
            f.member("input_file", &self.input_file)?;
            f.member("codec", self.codec)?;
            f.member("total_cluster_count", self.total_cluster_count)?;
            f.member("total_simple_block_count", self.total_simple_block_count)?;
            f.member("total_track_seconds", self.total_track_seconds)?;
            f.member("total_processing_seconds", self.total_processing_seconds)?;
            f.member("error", self.error)?;
            Ok(())
        })
    }
}

/// 出力用のの統計情報
#[derive(Debug, Clone)]
pub enum WriterStats {
    Mp4(Mp4WriterStats),
}

impl nojson::DisplayJson for WriterStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            WriterStats::Mp4(stats) => stats.fmt(f),
        }
    }
}

/// `Mp4Writer` 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct Mp4WriterStats {
    /// 音声コーデック
    pub audio_codec: Option<CodecName>,

    /// 映像コーデック
    pub video_codec: Option<CodecName>,

    /// 出力ファイルの初期化時に moov ボックス用に事前に予約した領域のサイズ
    pub reserved_moov_box_size: u64,

    /// 出力ファイルの最終処理時に判明した moov ボックスの実際のサイズ
    pub actual_moov_box_size: u64,

    /// 出力ファイルに含まれる音声チャンクの数
    pub total_audio_chunk_count: u64,

    /// 出力ファイルに含まれる映像チャンクの数
    pub total_video_chunk_count: u64,

    /// 出力ファイルに含まれる音声サンプルの数
    pub total_audio_sample_count: u64,

    /// 出力ファイルに含まれる映像サンプルの数
    pub total_video_sample_count: u64,

    /// 出力ファイルに含まれる音声データのバイト数
    pub total_audio_sample_data_byte_size: u64,

    /// 出力ファイルに含まれる映像データのバイト数
    pub total_video_sample_data_byte_size: u64,

    /// 出力ファイルに含まれる音声トラックの尺
    pub total_audio_track_seconds: Seconds,

    /// 出力ファイルに含まれる映像トラックの尺
    pub total_video_track_seconds: Seconds,

    /// MP4 出力処理部分に掛かった時間
    pub total_processing_seconds: Seconds,
}

impl nojson::DisplayJson for Mp4WriterStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "mp4_writer")?;
            f.member("audio_codec", self.audio_codec)?;
            f.member("video_codec", self.video_codec)?;
            f.member("reserved_moov_box_size", self.reserved_moov_box_size)?;
            f.member("actual_moov_box_size", self.actual_moov_box_size)?;
            f.member("total_audio_chunk_count", self.total_audio_chunk_count)?;
            f.member("total_video_chunk_count", self.total_video_chunk_count)?;
            f.member("total_audio_sample_count", self.total_audio_sample_count)?;
            f.member("total_video_sample_count", self.total_video_sample_count)?;
            f.member(
                "total_audio_sample_data_byte_size",
                self.total_audio_sample_data_byte_size,
            )?;
            f.member(
                "total_video_sample_data_byte_size",
                self.total_video_sample_data_byte_size,
            )?;
            f.member("total_audio_track_seconds", self.total_audio_track_seconds)?;
            f.member("total_video_track_seconds", self.total_video_track_seconds)?;
            f.member("total_processing_seconds", self.total_processing_seconds)?;
            Ok(())
        })
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VideoResolution {
    pub width: usize,
    pub height: usize,
}

impl VideoResolution {
    pub fn new(frame: &VideoFrame) -> Self {
        Self {
            width: frame.width.get(),
            height: frame.height.get(),
        }
    }
}

impl nojson::DisplayJson for VideoResolution {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(format!("{}x{}", self.width, self.height))
    }
}
