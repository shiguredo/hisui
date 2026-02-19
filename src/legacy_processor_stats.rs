use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::video::VideoFrame;

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
