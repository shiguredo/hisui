use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
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
                log::warn!("failed to acquire stats lock: {e}");
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

    /// 各プロセッサの統計情報
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
    AudioDecoder(AudioDecoderStats),
    VideoDecoder(VideoDecoderStats),
    AudioMixer(AudioMixerStats),
    VideoMixer(VideoMixerStats),
    AudioEncoder(AudioEncoderStats),
    VideoEncoder(VideoEncoderStats),
    Mp4Writer(Mp4WriterStats),
    Other {
        processor_type: String,
        total_processing_seconds: SharedAtomicSeconds,
        error: SharedAtomicFlag,
    },
}

impl ProcessorStats {
    pub fn other(processor_type: &str) -> Self {
        Self::Other {
            processor_type: processor_type.to_owned(),
            total_processing_seconds: Default::default(),
            error: Default::default(),
        }
    }

    pub fn set_error(&self) {
        match self {
            ProcessorStats::Mp4AudioReader(stats) => stats.error.set(true),
            ProcessorStats::Mp4VideoReader(stats) => stats.error.set(true),
            ProcessorStats::WebmAudioReader(stats) => stats.error.set(true),
            ProcessorStats::WebmVideoReader(stats) => stats.error.set(true),
            ProcessorStats::AudioDecoder(stats) => stats.error.set(true),
            ProcessorStats::VideoDecoder(stats) => stats.error.set(true),
            ProcessorStats::AudioMixer(stats) => stats.error.set(true),
            ProcessorStats::VideoMixer(stats) => stats.error.set(true),
            ProcessorStats::AudioEncoder(stats) => stats.error.set(true),
            ProcessorStats::VideoEncoder(stats) => stats.error.set(true),
            ProcessorStats::Mp4Writer(stats) => stats.error.set(true),
            ProcessorStats::Other { error, .. } => error.set(true),
        }
    }
}

impl nojson::DisplayJson for ProcessorStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            ProcessorStats::Mp4AudioReader(stats) => stats.fmt(f),
            ProcessorStats::Mp4VideoReader(stats) => stats.fmt(f),
            ProcessorStats::WebmAudioReader(stats) => stats.fmt(f),
            ProcessorStats::WebmVideoReader(stats) => stats.fmt(f),
            ProcessorStats::AudioDecoder(stats) => stats.fmt(f),
            ProcessorStats::VideoDecoder(stats) => stats.fmt(f),
            ProcessorStats::AudioMixer(stats) => stats.fmt(f),
            ProcessorStats::VideoMixer(stats) => stats.fmt(f),
            ProcessorStats::AudioEncoder(stats) => stats.fmt(f),
            ProcessorStats::VideoEncoder(stats) => stats.fmt(f),
            ProcessorStats::Mp4Writer(stats) => stats.fmt(f),
            ProcessorStats::Other {
                processor_type,
                total_processing_seconds,
                error,
            } => f.object(|f| {
                f.member("type", processor_type)?;
                f.member(
                    "total_processing_seconds",
                    total_processing_seconds.get_seconds(),
                )?;
                f.member("error", error.get())
            }),
        }
    }
}

/// `AudioMixer` 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct AudioMixerStats {
    /// ミキサーの入力 `AudioData` の数
    pub total_input_audio_data_count: SharedAtomicCounter,

    /// ミキサーが生成した `AudioData` の数
    pub total_output_audio_data_count: SharedAtomicCounter,

    /// ミキサーが生成した `AudioData` の合計尺
    pub total_output_audio_data_seconds: SharedAtomicSeconds,

    /// ミキサーが生成したサンプルの合計数
    pub total_output_sample_count: SharedAtomicCounter,

    /// ミキサーによって無音補完されたサンプルの合計数
    pub total_output_filled_sample_count: SharedAtomicCounter,

    /// 出力から除去されたサンプルの合計数
    pub total_trimmed_sample_count: SharedAtomicCounter,

    /// 合成処理部分に掛かった時間
    pub total_processing_seconds: SharedAtomicSeconds,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl nojson::DisplayJson for AudioMixerStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "audio_mixer")?;
            f.member(
                "total_input_audio_data_count",
                self.total_input_audio_data_count.get(),
            )?;
            f.member(
                "total_output_audio_data_count",
                self.total_output_audio_data_count.get(),
            )?;
            f.member(
                "total_output_audio_data_seconds",
                self.total_output_audio_data_seconds.get_seconds(),
            )?;
            f.member(
                "total_output_sample_count",
                self.total_output_sample_count.get(),
            )?;
            f.member(
                "total_output_filled_sample_count",
                self.total_output_filled_sample_count.get(),
            )?;
            f.member(
                "total_trimmed_sample_count",
                self.total_trimmed_sample_count.get(),
            )?;
            f.member(
                "total_processing_seconds",
                self.total_processing_seconds.get_seconds(),
            )?;
            f.member("error", self.error.get())?;
            Ok(())
        })
    }
}

#[derive(Debug, Default, Clone)]
pub struct VideoMixerStats {
    /// 合成後の映像の解像度
    pub output_video_resolution: VideoResolution,

    /// ミキサーの入力 `VideoFrame` の数
    pub total_input_video_frame_count: SharedAtomicCounter,

    /// ミキサーが生成した `VideoFrame` の数
    pub total_output_video_frame_count: SharedAtomicCounter,

    /// ミキサーが生成した `VideoFrame` の合計尺
    pub total_output_video_frame_seconds: SharedAtomicSeconds,

    /// 出力から除去された映像フレームの合計数
    pub total_trimmed_video_frame_count: SharedAtomicCounter,

    /// 合成を省略して前フレームの尺を延長したフレームの数
    pub total_extended_video_frame_count: SharedAtomicCounter,

    /// 合成処理部分に掛かった時間
    pub total_processing_seconds: SharedAtomicSeconds,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl nojson::DisplayJson for VideoMixerStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "video_mixer")?;
            f.member("output_video_resolution", self.output_video_resolution)?;
            f.member(
                "total_input_video_frame_count",
                self.total_input_video_frame_count.get(),
            )?;
            f.member(
                "total_output_video_frame_count",
                self.total_output_video_frame_count.get(),
            )?;
            f.member(
                "total_output_video_frame_seconds",
                self.total_output_video_frame_seconds.get_seconds(),
            )?;
            f.member(
                "total_trimmed_video_frame_count",
                self.total_trimmed_video_frame_count.get(),
            )?;
            f.member(
                "total_extended_video_frame_count",
                self.total_extended_video_frame_count.get(),
            )?;
            f.member(
                "total_processing_seconds",
                self.total_processing_seconds.get_seconds(),
            )?;
            f.member("error", self.error.get())?;
            Ok(())
        })
    }
}

/// 音声エンコーダー用の統計情報
#[derive(Debug, Clone)]
pub struct AudioEncoderStats {
    /// エンコーダーの種類
    pub engine: EngineName,

    /// コーデック
    pub codec: CodecName,

    /// エンコーダーで処理された `AudioData` の数
    pub total_audio_data_count: SharedAtomicCounter,

    /// 処理部分に掛かった時間
    pub total_processing_seconds: SharedAtomicSeconds,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl AudioEncoderStats {
    pub fn new(engine: EngineName, codec: CodecName) -> Self {
        Self {
            engine,
            codec,
            total_audio_data_count: Default::default(),
            total_processing_seconds: Default::default(),
            error: Default::default(),
        }
    }
}

impl nojson::DisplayJson for AudioEncoderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "audio_encoder")?;
            f.member("engine", self.engine)?;
            f.member("codec", self.codec)?;
            f.member("total_audio_data_count", self.total_audio_data_count.get())?;
            f.member(
                "total_processing_seconds",
                self.total_processing_seconds.get_seconds(),
            )?;
            f.member("error", self.error.get())?;
            Ok(())
        })
    }
}

/// 映像エンコーダー用の統計情報
#[derive(Debug, Clone)]
pub struct VideoEncoderStats {
    /// エンコーダーの種類
    pub engine: EngineName,

    /// コーデック
    pub codec: CodecName,

    /// エンコード対象の `VideoFrame` の数
    pub total_input_video_frame_count: SharedAtomicCounter,

    /// 実際にエンコードされた `VideoFrame` の数
    pub total_output_video_frame_count: SharedAtomicCounter,

    /// 処理部分に掛かった時間
    pub total_processing_seconds: SharedAtomicSeconds,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl VideoEncoderStats {
    pub fn new(engine: EngineName, codec: CodecName) -> Self {
        Self {
            engine,
            codec,
            total_input_video_frame_count: Default::default(),
            total_output_video_frame_count: Default::default(),
            total_processing_seconds: Default::default(),
            error: Default::default(),
        }
    }
}

impl nojson::DisplayJson for VideoEncoderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "video_encoder")?;
            f.member("engine", self.engine)?;
            f.member("codec", self.codec)?;
            f.member(
                "total_input_video_frame_count",
                self.total_input_video_frame_count.get(),
            )?;
            f.member(
                "total_output_video_frame_count",
                self.total_output_video_frame_count.get(),
            )?;
            f.member(
                "total_processing_seconds",
                self.total_processing_seconds.get_seconds(),
            )?;
            f.member("error", self.error.get())?;
            Ok(())
        })
    }
}

/// 音声デコーダー用の統計情報
#[derive(Debug, Default, Clone)]
pub struct AudioDecoderStats {
    /// 入力ソースの ID
    pub source_id: SharedOption<SourceId>,

    /// デコーダーの種類
    pub engine: Option<EngineName>,

    /// コーデック
    pub codec: Option<CodecName>,

    /// デコーダーで処理された `AudioData` の数
    pub total_audio_data_count: SharedAtomicCounter,

    /// 処理部分に掛かった時間
    pub total_processing_seconds: SharedAtomicSeconds,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl nojson::DisplayJson for AudioDecoderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "audio_decoder")?;
            f.member("source_id", self.source_id.get())?;
            f.member("engine", self.engine)?;
            f.member("codec", self.codec)?;
            f.member("total_audio_data_count", self.total_audio_data_count.get())?;
            f.member(
                "total_processing_seconds",
                self.total_processing_seconds.get_seconds(),
            )?;
            f.member("error", self.error.get())?;
            Ok(())
        })
    }
}

/// 映像デコーダー用の統計情報
#[derive(Debug, Default, Clone)]
pub struct VideoDecoderStats {
    /// 入力ソースの ID
    pub source_id: SharedOption<SourceId>,

    /// デコーダーの種類
    pub engine: SharedOption<EngineName>,

    /// コーデック
    pub codec: SharedOption<CodecName>,

    /// デコード対象の `VideoFrame` の数
    pub total_input_video_frame_count: SharedAtomicCounter,

    /// デコードされた `VideoFrame` の数
    pub total_output_video_frame_count: SharedAtomicCounter,

    /// 処理部分に掛かった時間
    pub total_processing_seconds: SharedAtomicSeconds,

    /// 解像度リスト
    pub resolutions: SharedSet<VideoResolution>,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl nojson::DisplayJson for VideoDecoderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "video_decoder")?;
            f.member("source_id", self.source_id.get())?;
            f.member("engine", self.engine.get())?;
            f.member("codec", self.codec.get())?;
            f.member(
                "total_input_video_frame_count",
                self.total_input_video_frame_count.get(),
            )?;
            f.member(
                "total_output_video_frame_count",
                self.total_output_video_frame_count.get(),
            )?;
            f.member(
                "total_processing_seconds",
                self.total_processing_seconds.get_seconds(),
            )?;
            f.member("resolutions", self.resolutions.get())?;
            f.member("error", self.error.get())?;
            Ok(())
        })
    }
}

#[derive(Debug, Default, Clone)]
pub struct SharedAtomicFlag(Arc<AtomicBool>);

impl SharedAtomicFlag {
    pub fn set(&self, v: bool) {
        // 統計情報の更新が複数スレッドから行われることはないので Relaxed で十分
        self.0.store(v, Ordering::Relaxed)
    }

    pub fn get(&self) -> bool {
        // 取得結果が一時的に古くても問題はないので Relaxed で十分
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Default, Clone)]
pub struct SharedAtomicCounter(Arc<AtomicU64>);

impl SharedAtomicCounter {
    pub fn add(&self, n: u64) {
        // 統計情報の更新が複数スレッドから行われることはないので Relaxed で十分
        self.0.fetch_add(n, Ordering::Relaxed);
    }

    pub fn set(&self, n: u64) {
        // 統計情報の更新が複数スレッドから行われることはないので Relaxed で十分
        self.0.store(n, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        // 取得結果が一時的に古くても問題はないので Relaxed で十分
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Default, Clone)]
pub struct SharedAtomicSeconds(SharedAtomicCounter);

impl SharedAtomicSeconds {
    pub fn new(n: Seconds) -> Self {
        let v = Self::default();
        v.set(n);
        v
    }

    pub fn add(&self, n: Seconds) {
        self.0.add(n.get().as_nanos() as u64);
    }

    pub fn set(&self, n: Seconds) {
        self.0.set(n.get().as_nanos() as u64);
    }

    pub fn get_seconds(&self) -> Seconds {
        Seconds(Duration::from_nanos(self.0.get()))
    }

    pub fn get_duration(&self) -> Duration {
        self.get_seconds().get()
    }
}

#[derive(Debug, Clone)]
pub struct SharedOption<T>(Arc<Mutex<Option<T>>>);

impl<T> SharedOption<T> {
    pub fn new(value: Option<T>) -> Self {
        Self(Arc::new(Mutex::new(value)))
    }

    pub fn set(&self, value: T) {
        // [NOTE]
        // ロック獲得に失敗することはまずないはずだし、
        // 失敗しても統計が不正確になるだけで、全体の実行に影響はないので、単に無視している
        // （なおここで警告ログなどを出すと量が多くなりすぎる可能性があるのでやらない）
        if let Ok(mut v) = self.0.lock() {
            *v = Some(value);
        }
    }

    pub fn set_once<F>(&self, f: F)
    where
        F: FnOnce() -> T,
    {
        // [NOTE] 同上
        if let Ok(mut v) = self.0.lock()
            && v.is_none()
        {
            *v = Some(f());
        }
    }
}

impl<T: Clone> SharedOption<T> {
    pub fn get(&self) -> Option<T> {
        if let Ok(v) = self.0.lock() {
            v.clone()
        } else {
            // [NOTE]
            // ロック獲得に失敗することはまずないはずだし、
            // 失敗しても統計が不正確になるだけで、全体の実行に影響はないので、単に None 扱いにしている
            // （なおここで警告ログなどを出すと量が多くなりすぎる可能性があるのでやらない）
            None
        }
    }
}

impl<T> Default for SharedOption<T> {
    fn default() -> Self {
        Self::new(None)
    }
}

#[derive(Debug, Clone)]
pub struct SharedSet<T>(Arc<Mutex<BTreeSet<T>>>);

impl<T: Ord + Eq> SharedSet<T> {
    pub fn insert(&self, value: T) {
        if let Ok(mut v) = self.0.lock() {
            v.insert(value);
        }
    }
}

impl<T: Clone> SharedSet<T> {
    pub fn get(&self) -> BTreeSet<T> {
        if let Ok(v) = self.0.lock() {
            v.clone()
        } else {
            BTreeSet::default()
        }
    }
}

impl<T> Default for SharedSet<T> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(BTreeSet::new())))
    }
}

/// `Mp4AudioReader` 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct Mp4AudioReaderStats {
    /// 入力ファイルのパス
    pub input_file: PathBuf,

    /// 音声コーデック
    pub codec: SharedOption<CodecName>,

    /// Mp4 のサンプルの数
    pub total_sample_count: SharedAtomicCounter,

    /// 入力ファイルに含まれる音声トラックの尺
    pub total_track_seconds: SharedAtomicSeconds,

    /// 入力処理部分に掛かった時間
    pub total_processing_seconds: SharedAtomicSeconds,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl nojson::DisplayJson for Mp4AudioReaderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "mp4_audio_reader")?;
            f.member("input_file", &self.input_file)?;
            f.member("codec", self.codec.get())?;
            f.member("total_sample_count", self.total_sample_count.get())?;
            f.member(
                "total_track_seconds",
                self.total_track_seconds.get_seconds(),
            )?;
            f.member(
                "total_processing_seconds",
                self.total_processing_seconds.get_seconds(),
            )?;
            f.member("error", self.error.get())?;
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
    pub codec: SharedOption<CodecName>,

    /// 映像の解像度（途中で変わった場合は複数になる）
    pub resolutions: SharedSet<VideoResolution>,

    /// Mp4 のサンプルの数
    pub total_sample_count: SharedAtomicCounter,

    /// 入力ファイルに含まれる映像トラックの尺
    pub total_track_seconds: SharedAtomicSeconds,

    /// 入力処理部分に掛かった時間
    pub total_processing_seconds: SharedAtomicSeconds,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl nojson::DisplayJson for Mp4VideoReaderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "mp4_video_reader")?;
            f.member("input_file", &self.input_file)?;
            f.member("codec", self.codec.get())?;
            f.member(
                "resolutions",
                nojson::json(|f| {
                    f.array(|f| {
                        f.elements(
                            self.resolutions
                                .get()
                                .iter()
                                .map(|res| format!("{}x{}", res.width, res.height)),
                        )
                    })
                }),
            )?;
            f.member("total_sample_count", self.total_sample_count.get())?;
            f.member(
                "total_track_seconds",
                self.total_track_seconds.get_seconds(),
            )?;
            f.member(
                "total_processing_seconds",
                self.total_processing_seconds.get_seconds(),
            )?;
            f.member("error", self.error.get())?;
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
    pub total_cluster_count: SharedAtomicCounter,

    /// WebM のシンプルブロックの数
    pub total_simple_block_count: SharedAtomicCounter,

    /// 入力ファイルに含まれる音声トラックの尺
    pub total_track_seconds: SharedAtomicSeconds,

    /// 入力処理部分に掛かった時間
    pub total_processing_seconds: SharedAtomicSeconds,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl nojson::DisplayJson for WebmAudioReaderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "webm_audio_reader")?;
            f.member("input_file", &self.input_file)?;
            f.member("codec", self.codec)?;
            f.member("total_cluster_count", self.total_cluster_count.get())?;
            f.member(
                "total_simple_block_count",
                self.total_simple_block_count.get(),
            )?;
            f.member(
                "total_track_seconds",
                self.total_track_seconds.get_seconds(),
            )?;
            f.member(
                "total_processing_seconds",
                self.total_processing_seconds.get_seconds(),
            )?;
            f.member("error", self.error.get())?;
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
    pub codec: SharedOption<CodecName>,

    /// WebM のクラスターの数
    pub total_cluster_count: SharedAtomicCounter,

    /// WebM のシンプルブロックの数
    pub total_simple_block_count: SharedAtomicCounter,

    /// 入力ファイルに含まれる映像トラックの尺
    pub total_track_seconds: SharedAtomicSeconds,

    /// 入力処理部分に掛かった時間
    pub total_processing_seconds: SharedAtomicSeconds,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl nojson::DisplayJson for WebmVideoReaderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "webm_video_reader")?;
            f.member("input_file", &self.input_file)?;
            f.member("codec", self.codec.get())?;
            f.member("total_cluster_count", self.total_cluster_count.get())?;
            f.member(
                "total_simple_block_count",
                self.total_simple_block_count.get(),
            )?;
            f.member(
                "total_track_seconds",
                self.total_track_seconds.get_seconds(),
            )?;
            f.member(
                "total_processing_seconds",
                self.total_processing_seconds.get_seconds(),
            )?;
            f.member("error", self.error.get())?;
            Ok(())
        })
    }
}

/// `Mp4Writer` 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct Mp4WriterStats {
    /// 音声コーデック
    pub audio_codec: SharedOption<CodecName>,

    /// 映像コーデック
    pub video_codec: SharedOption<CodecName>,

    /// 出力ファイルの初期化時に moov ボックス用に事前に予約した領域のサイズ
    pub reserved_moov_box_size: SharedAtomicCounter,

    /// 出力ファイルの最終処理時に判明した moov ボックスの実際のサイズ
    pub actual_moov_box_size: SharedAtomicCounter,

    /// 出力ファイルに含まれる音声チャンクの数
    pub total_audio_chunk_count: SharedAtomicCounter,

    /// 出力ファイルに含まれる映像チャンクの数
    pub total_video_chunk_count: SharedAtomicCounter,

    /// 出力ファイルに含まれる音声サンプルの数
    pub total_audio_sample_count: SharedAtomicCounter,

    /// 出力ファイルに含まれる映像サンプルの数
    pub total_video_sample_count: SharedAtomicCounter,

    /// 出力ファイルに含まれる音声データのバイト数
    pub total_audio_sample_data_byte_size: SharedAtomicCounter,

    /// 出力ファイルに含まれる映像データのバイト数
    pub total_video_sample_data_byte_size: SharedAtomicCounter,

    /// 出力ファイルに含まれる音声トラックの尺
    pub total_audio_track_seconds: SharedAtomicSeconds,

    /// 出力ファイルに含まれる映像トラックの尺
    pub total_video_track_seconds: SharedAtomicSeconds,

    /// MP4 出力処理部分に掛かった時間
    pub total_processing_seconds: SharedAtomicSeconds,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl nojson::DisplayJson for Mp4WriterStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "mp4_writer")?;
            f.member("audio_codec", self.audio_codec.get())?;
            f.member("video_codec", self.video_codec.get())?;
            f.member("reserved_moov_box_size", self.reserved_moov_box_size.get())?;
            f.member("actual_moov_box_size", self.actual_moov_box_size.get())?;
            f.member(
                "total_audio_chunk_count",
                self.total_audio_chunk_count.get(),
            )?;
            f.member(
                "total_video_chunk_count",
                self.total_video_chunk_count.get(),
            )?;
            f.member(
                "total_audio_sample_count",
                self.total_audio_sample_count.get(),
            )?;
            f.member(
                "total_video_sample_count",
                self.total_video_sample_count.get(),
            )?;
            f.member(
                "total_audio_sample_data_byte_size",
                self.total_audio_sample_data_byte_size.get(),
            )?;
            f.member(
                "total_video_sample_data_byte_size",
                self.total_video_sample_data_byte_size.get(),
            )?;
            f.member(
                "total_audio_track_seconds",
                self.total_audio_track_seconds.get_seconds(),
            )?;
            f.member(
                "total_video_track_seconds",
                self.total_video_track_seconds.get_seconds(),
            )?;
            f.member(
                "total_processing_seconds",
                self.total_processing_seconds.get_seconds(),
            )?;
            f.member("error", self.error.get())?;
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
