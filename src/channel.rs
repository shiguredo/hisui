use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub fn sync_channel<T>() -> (SyncSender<T>, Receiver<T>) {
    // Hisui の用途ではチャネルのキューサイズ上限を細かく調整・指定する意味はないので
    // 適当な値をハードコーディングしておく。
    let bound = 5;
    sync_channel_with_bound(bound)
}

pub fn sync_channel_with_bound<T>(bound: usize) -> (SyncSender<T>, Receiver<T>) {
    let (tx, rx) = std::sync::mpsc::sync_channel(bound);
    let tx = SyncSender { tx };
    let rx = Receiver { rx, next: None };
    (tx, rx)
}

#[derive(Debug, Clone)]
pub struct SyncSender<T> {
    tx: std::sync::mpsc::SyncSender<T>,
}

impl<T> SyncSender<T> {
    pub fn send(&self, item: T) -> bool {
        self.tx.send(item).is_ok()
    }
}

#[derive(Debug)]
pub struct Receiver<T> {
    rx: std::sync::mpsc::Receiver<T>,
    next: Option<T>,
}

impl<T> Receiver<T> {
    pub fn peek(&mut self) -> Option<&T> {
        if self.next.is_none() {
            self.next = self.rx.recv().ok();
        }
        self.next.as_ref()
    }

    pub fn recv(&mut self) -> Option<T> {
        if self.next.is_none() {
            self.peek();
        }
        self.next.take()
    }
}

#[derive(Debug, Default, Clone)]
pub struct ErrorFlag(Arc<AtomicBool>);

impl ErrorFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn get(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}
