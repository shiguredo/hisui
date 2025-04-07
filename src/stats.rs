use std::{
    collections::BTreeSet,
    path::PathBuf,
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

impl SharedStats {
    pub fn new() -> Self {
        let inner = Arc::new(Mutex::new(Stats::default()));
        Self { inner }
    }

    pub fn with_lock<F>(&self, f: F)
    where
        F: FnOnce(&mut Stats),
    {
        match self.inner.lock() {
            Ok(mut stats) => {
                f(&mut *stats);
            }
            Err(e) => {
                // 統計情報の更新ができなくても致命的ではないので警告に止める
                log::warn!("failed to acqure stats lock: {e}");
                return;
            }
        };
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
}

impl nojson::DisplayJson for Stats {
    fn fmt(&self, _f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        todo!()
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
// TODO: #[serde(into = "f32")]
pub struct Seconds(Duration);

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
// TODO: #[serde(tag = "kind", rename_all = "snake_case")]
pub enum MixerStats {
    Audio(AudioMixerStats),
    Video(VideoMixerStats),
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

    /// 合成処理部分に掛かった時間
    pub total_processing_seconds: Seconds,

    /// エラーで中断したかどうか
    pub error: bool,
}

/// エンコーダー関連の統計情報
#[derive(Debug, Clone)]
// TODO: #[serde(tag = "kind", rename_all = "snake_case")]
pub enum EncoderStats {
    Audio(AudioEncoderStats),
    Video(VideoEncoderStats),
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

/// デコーダー関連の統計情報
#[derive(Debug, Clone)]
// TODO: #[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecoderStats {
    Audio(AudioDecoderStats),
    Video(VideoDecoderStats),
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

/// 入力関連の統計情報
#[derive(Debug, Clone)]
// TODO: #[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReaderStats {
    WebmAudio(WebmAudioReaderStats),
    WebmVideo(WebmVideoReaderStats),
    Mp4Audio(Mp4AudioReaderStats),
    Mp4Video(Mp4VideoReaderStats),
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

/// 出力用のの統計情報
#[derive(Debug, Clone)]
// TODO: #[serde(tag = "kind", rename_all = "snake_case")]
pub enum WriterStats {
    Mp4(Mp4WriterStats),
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

    /// 出力ファイルに含まれる音声トラックの尺
    pub total_audio_track_seconds: Seconds,

    /// 出力ファイルに含まれる映像トラックの尺
    pub total_video_track_seconds: Seconds,

    /// MP4 出力処理部分に掛かった時間
    pub total_processing_seconds: Seconds,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
// TODO: #[serde(into = "String")]
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

impl From<VideoResolution> for String {
    fn from(value: VideoResolution) -> Self {
        format!("{}x{}", value.width, value.height)
    }
}
