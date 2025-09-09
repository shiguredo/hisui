use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::{
    metadata::SourceId,
    types::{CodecName, EngineName},
    video::VideoFrame,
};

#[derive(Debug, Default, Clone)]
pub struct Stats {
    /// 全体の合成に要した実時間
    pub elapsed_duration: Duration,

    /// 全体でひとつでもエラーが発生したら true になる
    pub error: SharedAtomicFlag,

    /// 各プロセッサの統計情報
    pub processors: Vec<ProcessorStats>,

    /// プロセッサを実行するワーカースレッドの統計情報
    pub worker_threads: Vec<WorkerThreadStats>,
}

impl Stats {
    pub fn save(&self, output_file_path: &Path) {
        let json = nojson::json(|f| {
            f.set_indent_size(2);
            f.set_spacing(true);
            f.value(self)
        })
        .to_string();
        if let Err(e) = std::fs::write(output_file_path, json) {
            // 統計が出力できなくても全体を失敗扱いにはしない
            log::warn!(
                "failed to write stats JSON: path={}, reason={e}",
                output_file_path.display()
            );
        }
    }
}

impl nojson::DisplayJson for Stats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("elapsed_seconds", self.elapsed_duration.as_secs_f32())?;
            f.member("error", self.error.get())?;
            f.member("processors", &self.processors)?;
            f.member("worker_threads", &self.worker_threads)?;
            Ok(())
        })
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
        total_processing_duration: SharedAtomicDuration,
        error: SharedAtomicFlag,
    },
}

impl ProcessorStats {
    pub fn other(processor_type: &str) -> Self {
        Self::Other {
            processor_type: processor_type.to_owned(),
            total_processing_duration: Default::default(),
            error: Default::default(),
        }
    }

    pub fn total_processing_duration(&self) -> SharedAtomicDuration {
        match self {
            ProcessorStats::Mp4AudioReader(stats) => stats.total_processing_duration.clone(),
            ProcessorStats::Mp4VideoReader(stats) => stats.total_processing_duration.clone(),
            ProcessorStats::WebmAudioReader(stats) => stats.total_processing_duration.clone(),
            ProcessorStats::WebmVideoReader(stats) => stats.total_processing_duration.clone(),
            ProcessorStats::AudioDecoder(stats) => stats.total_processing_duration.clone(),
            ProcessorStats::VideoDecoder(stats) => stats.total_processing_duration.clone(),
            ProcessorStats::AudioMixer(stats) => stats.total_processing_duration.clone(),
            ProcessorStats::VideoMixer(stats) => stats.total_processing_duration.clone(),
            ProcessorStats::AudioEncoder(stats) => stats.total_processing_duration.clone(),
            ProcessorStats::VideoEncoder(stats) => stats.total_processing_duration.clone(),
            ProcessorStats::Mp4Writer(stats) => stats.total_processing_duration.clone(),
            ProcessorStats::Other {
                total_processing_duration,
                ..
            } => total_processing_duration.clone(),
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
                total_processing_duration,
                error,
            } => f.object(|f| {
                f.member("type", processor_type)?;
                f.member(
                    "total_processing_seconds",
                    total_processing_duration.get().as_secs_f32(),
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
    pub total_output_audio_data_duration: SharedAtomicDuration,

    /// ミキサーが生成したサンプルの合計数
    pub total_output_sample_count: SharedAtomicCounter,

    /// ミキサーによって無音補完されたサンプルの合計数
    pub total_output_filled_sample_count: SharedAtomicCounter,

    /// 出力から除去されたサンプルの合計数
    pub total_trimmed_sample_count: SharedAtomicCounter,

    // TODO: 以下のふたつの項目は、個々のプロセッサではなくワーカースレッドが
    // 共通的に処理するものなので個別の統計構造体の外にだした方がいいかもしれない
    /// 合成処理部分に掛かった時間
    pub total_processing_duration: SharedAtomicDuration,

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
                self.total_output_audio_data_duration.get().as_secs_f32(),
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
                self.total_processing_duration.get().as_secs_f32(),
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
    pub total_output_video_frame_duration: SharedAtomicDuration,

    /// 出力から除去された映像フレームの合計数
    pub total_trimmed_video_frame_count: SharedAtomicCounter,

    /// 合成を省略して前フレームの尺を延長したフレームの数
    pub total_extended_video_frame_count: SharedAtomicCounter,

    /// 合成処理部分に掛かった時間
    pub total_processing_duration: SharedAtomicDuration,

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
                self.total_output_video_frame_duration.get().as_secs_f32(),
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
                self.total_processing_duration.get().as_secs_f32(),
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
    pub total_processing_duration: SharedAtomicDuration,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl AudioEncoderStats {
    pub fn new(engine: EngineName, codec: CodecName) -> Self {
        Self {
            engine,
            codec,
            total_audio_data_count: Default::default(),
            total_processing_duration: Default::default(),
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
                self.total_processing_duration.get().as_secs_f32(),
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
    pub total_processing_duration: SharedAtomicDuration,

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
            total_processing_duration: Default::default(),
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
                self.total_processing_duration.get().as_secs_f32(),
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
    pub total_processing_duration: SharedAtomicDuration,

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
                self.total_processing_duration.get().as_secs_f32(),
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
    pub total_processing_duration: SharedAtomicDuration,

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
                self.total_processing_duration.get().as_secs_f32(),
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
pub struct SharedAtomicDuration(SharedAtomicCounter);

impl SharedAtomicDuration {
    pub fn new(n: Duration) -> Self {
        let v = Self::default();
        v.set(n);
        v
    }

    pub fn add(&self, n: Duration) {
        self.0.add(n.as_nanos() as u64);
    }

    pub fn set(&self, n: Duration) {
        self.0.set(n.as_nanos() as u64);
    }

    pub fn get(&self) -> Duration {
        Duration::from_nanos(self.0.get())
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

    pub fn clear(&self) {
        // [NOTE] 同上
        if let Ok(mut v) = self.0.lock() {
            *v = None;
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
    /// 入力ファイルのパスのリスト
    ///
    /// 分割録画の場合には要素の数が複数になる
    pub input_files: Vec<PathBuf>,

    /// 現在処理中の入力ファイル
    pub current_input_file: SharedOption<PathBuf>,

    /// 音声コーデック
    pub codec: Option<CodecName>,

    /// Mp4 のサンプルの数
    pub total_sample_count: SharedAtomicCounter,

    /// 入力ファイルに含まれる音声トラックの尺
    pub total_track_duration: SharedAtomicDuration,

    /// 入力処理部分に掛かった時間
    pub total_processing_duration: SharedAtomicDuration,

    /// ソースの表示開始時刻（オフセッット）
    pub start_time: Duration,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl nojson::DisplayJson for Mp4AudioReaderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "mp4_audio_reader")?;
            f.member("input_files", &self.input_files)?;
            if let Some(path) = self.current_input_file.get() {
                f.member("current_input_file", path)?;
            }
            f.member("codec", self.codec)?;
            f.member("total_sample_count", self.total_sample_count.get())?;
            f.member(
                "total_track_seconds",
                self.total_track_duration.get().as_secs_f32(),
            )?;
            f.member(
                "total_processing_seconds",
                self.total_processing_duration.get().as_secs_f32(),
            )?;
            f.member("start_time_seconds", self.start_time.as_secs_f32())?;
            f.member("error", self.error.get())?;
            Ok(())
        })
    }
}

/// `Mp4VideoReader` 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct Mp4VideoReaderStats {
    /// 入力ファイルのパスのリスト
    ///
    /// 分割録画の場合には要素の数が複数になる
    pub input_files: Vec<PathBuf>,

    /// 現在処理中の入力ファイル
    pub current_input_file: SharedOption<PathBuf>,

    /// 映像コーデック
    pub codec: SharedOption<CodecName>,

    /// 映像の解像度（途中で変わった場合は複数になる）
    pub resolutions: SharedSet<VideoResolution>,

    /// Mp4 のサンプルの数
    pub total_sample_count: SharedAtomicCounter,

    /// 入力ファイルに含まれる映像トラックの尺
    pub total_track_duration: SharedAtomicDuration,

    /// 入力処理部分に掛かった時間
    pub total_processing_duration: SharedAtomicDuration,

    /// ソースの表示開始時刻（オフセッット）
    pub start_time: Duration,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl nojson::DisplayJson for Mp4VideoReaderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "mp4_video_reader")?;
            f.member("input_files", &self.input_files)?;
            if let Some(path) = self.current_input_file.get() {
                f.member("current_input_file", path)?;
            }
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
                self.total_track_duration.get().as_secs_f32(),
            )?;
            f.member(
                "total_processing_seconds",
                self.total_processing_duration.get().as_secs_f32(),
            )?;
            f.member("start_time_seconds", self.start_time.as_secs_f32())?;
            f.member("error", self.error.get())?;
            Ok(())
        })
    }
}

/// `WebmAudioReader` 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct WebmAudioReaderStats {
    /// 入力ファイルのパスのリスト
    ///
    /// 分割録画の場合には要素の数が複数になる
    pub input_files: Vec<PathBuf>,

    /// 現在処理中の入力ファイル
    pub current_input_file: SharedOption<PathBuf>,

    /// 音声コーデック
    pub codec: Option<CodecName>,

    /// WebM のクラスターの数
    pub total_cluster_count: SharedAtomicCounter,

    /// WebM のシンプルブロックの数
    pub total_simple_block_count: SharedAtomicCounter,

    /// 入力ファイルに含まれる音声トラックの尺
    pub total_track_duration: SharedAtomicDuration,

    /// 入力処理部分に掛かった時間
    pub total_processing_duration: SharedAtomicDuration,

    /// ソースの表示開始時刻（オフセッット）
    pub start_time: Duration,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl nojson::DisplayJson for WebmAudioReaderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "webm_audio_reader")?;
            f.member("input_files", &self.input_files)?;
            if let Some(path) = self.current_input_file.get() {
                f.member("current_input_file", path)?;
            }
            f.member("codec", self.codec)?;
            f.member("total_cluster_count", self.total_cluster_count.get())?;
            f.member(
                "total_simple_block_count",
                self.total_simple_block_count.get(),
            )?;
            f.member(
                "total_track_seconds",
                self.total_track_duration.get().as_secs_f32(),
            )?;
            f.member(
                "total_processing_seconds",
                self.total_processing_duration.get().as_secs_f32(),
            )?;
            f.member("start_time_seconds", self.start_time.as_secs_f32())?;
            f.member("error", self.error.get())?;
            Ok(())
        })
    }
}

/// `WebmVideoReader` 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct WebmVideoReaderStats {
    /// 入力ファイルのパスのリスト
    ///
    /// 分割録画の場合には要素の数が複数になる
    pub input_files: Vec<PathBuf>,

    /// 現在処理中の入力ファイル
    pub current_input_file: SharedOption<PathBuf>,

    /// 映像コーデック
    pub codec: SharedOption<CodecName>,

    /// WebM のクラスターの数
    pub total_cluster_count: SharedAtomicCounter,

    /// WebM のシンプルブロックの数
    pub total_simple_block_count: SharedAtomicCounter,

    /// 入力ファイルに含まれる映像トラックの尺
    pub total_track_duration: SharedAtomicDuration,

    /// 入力処理部分に掛かった時間
    pub total_processing_duration: SharedAtomicDuration,

    /// ソースの表示開始時刻（オフセッット）
    pub start_time: Duration,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl nojson::DisplayJson for WebmVideoReaderStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "webm_video_reader")?;
            f.member("input_files", &self.input_files)?;
            if let Some(path) = self.current_input_file.get() {
                f.member("current_input_file", path)?;
            }
            f.member("codec", self.codec.get())?;
            f.member("total_cluster_count", self.total_cluster_count.get())?;
            f.member(
                "total_simple_block_count",
                self.total_simple_block_count.get(),
            )?;
            f.member(
                "total_track_seconds",
                self.total_track_duration.get().as_secs_f32(),
            )?;
            f.member(
                "total_processing_seconds",
                self.total_processing_duration.get().as_secs_f32(),
            )?;
            f.member("start_time_seconds", self.start_time.as_secs_f32())?;
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
    pub total_audio_track_duration: SharedAtomicDuration,

    /// 出力ファイルに含まれる映像トラックの尺
    pub total_video_track_duration: SharedAtomicDuration,

    /// MP4 出力処理部分に掛かった時間
    pub total_processing_duration: SharedAtomicDuration,

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
                self.total_audio_track_duration.get().as_secs_f32(),
            )?;
            f.member(
                "total_video_track_seconds",
                self.total_video_track_duration.get().as_secs_f32(),
            )?;
            f.member(
                "total_processing_seconds",
                self.total_processing_duration.get().as_secs_f32(),
            )?;
            f.member("error", self.error.get())?;
            Ok(())
        })
    }
}

/// `Mp4Writer` 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct WorkerThreadStats {
    /// 担当しているプロセッサの番号（インデックス）リスト
    pub processors: Vec<usize>,

    /// 処理部分に掛かった時間
    pub total_processing_duration: SharedAtomicDuration,

    /// 入出力の待機時間
    pub total_waiting_duration: SharedAtomicDuration,
}

impl nojson::DisplayJson for WorkerThreadStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member(
                "processors",
                nojson::json(|f| {
                    let indent = f.get_indent_size();
                    f.set_indent_size(0);
                    f.value(&self.processors)?;
                    f.set_indent_size(indent);
                    Ok(())
                }),
            )?;
            f.member(
                "total_processing_seconds",
                self.total_processing_duration.get().as_secs_f32(),
            )?;
            f.member(
                "total_waiting_seconds",
                self.total_waiting_duration.get().as_secs_f32(),
            )?;
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
            width: frame.width,
            height: frame.height,
        }
    }
}

impl nojson::DisplayJson for VideoResolution {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(format!("{}x{}", self.width, self.height))
    }
}
