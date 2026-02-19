use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::{types::CodecName, video::VideoFrame};

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

    pub fn increment(&self) {
        self.add(1);
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

impl nojson::DisplayJson for SharedAtomicCounter {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(self.get())
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

impl nojson::DisplayJson for SharedAtomicDuration {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(self.get().as_secs_f32())
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

impl<T: Clone + nojson::DisplayJson> nojson::DisplayJson for SharedOption<T> {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(self.get())
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

    /// 分割録画の際にタイムスタンプを調整するためのオフセット時間
    pub track_duration_offset: SharedAtomicDuration,

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
                (self.track_duration_offset.get() + self.total_track_duration.get()).as_secs_f32(),
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

    /// 分割録画の際にタイムスタンプを調整するためのオフセット時間
    pub track_duration_offset: SharedAtomicDuration,

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
                (self.track_duration_offset.get() + self.total_track_duration.get()).as_secs_f32(),
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

    /// 分割録画の際にタイムスタンプを調整するためのオフセット時間
    pub track_duration_offset: SharedAtomicDuration,

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
                (self.track_duration_offset.get() + self.total_track_duration.get()).as_secs_f32(),
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

    /// 分割録画の際にタイムスタンプを調整するためのオフセット時間
    pub track_duration_offset: SharedAtomicDuration,

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
                (self.track_duration_offset.get() + self.total_track_duration.get()).as_secs_f32(),
            )?;
            f.member("start_time_seconds", self.start_time.as_secs_f32())?;
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
