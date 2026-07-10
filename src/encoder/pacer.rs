//! nvcodec のような callback 完結型 inner で「上位からの投入 (`push_wait`) を
//! callback スレッドの完了 (`pop` + notify) と協調させる」ための最小 API を提供する。
//! feature 非依存で default feature の cargo test でも単体テストが走る。

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// 内部キュー上限で書き手をセルフペーシングする薄い型
///
/// 契約:
///
/// - `push_wait`: キュー長が `limit` 未満になるまで待って `push_back` する。
///   `wait_timeout` (100ms) の再評価は spurious wakeup 対策の `while` ループとは
///   独立した safety net。
/// - `pop`: FIFO 順に 1 件取り出し、 lock 解放後に `notify_one` する。
///   結果が `Some` / `None` のどちらでも notify するため、
///   呼び出し側 (callback Ok / Err 両分岐) が `pop` を呼びさえすれば
///   書き手の `push_wait` は必ず起こされる。
///
/// Mutex ホールドスコープ:
///
/// - `push_wait` は `wait_timeout` の guard 保持中のみ lock を持ち、
///   `push_back` 直後に guard を drop することで caller が Mutex 外で
///   他の副作用 (例: `inner.encode()`) を実行できる。
/// - `pop` は `pop_front` のみ lock 内で実行し、 `notify_one` は lock 解放後に呼ぶ。
///   `notify_one` を lock 内で呼ぶと待機側が起きた直後に lock 待ちで再度 block する
///   性能事故を招くため。
///
/// Poison:
///
/// - `Mutex` / `Condvar` の poison は `.expect(...)` で panic させる (方針)。
///   callback スレッドの panic は encoder プロセス全体を止める前提で、
///   `Pacer` 単体では回復を試みない。
///
/// Send / Sync:
///
/// - `T: Send` なら auto trait で `Pacer<T>: Send + Sync` になる。
#[derive(Debug)]
pub struct Pacer<T> {
    queue: Mutex<VecDeque<T>>,
    cv: Condvar,
    limit: usize,
}

impl<T> Pacer<T> {
    /// 上限 `limit` で新しい `Pacer` を作成する。
    pub fn new(limit: usize) -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            cv: Condvar::new(),
            limit,
        }
    }

    /// キュー長が `limit` 未満になるまで待って `push_back` する。
    ///
    /// `wait_timeout` (100ms) は periodic 再評価用の safety net。
    /// spurious wakeup 対策には `while guard.len() >= limit` の再判定で対応する
    /// (if だと誤起床で条件破りうる)。
    pub fn push_wait(&self, item: T) {
        let mut guard = self.queue.lock().expect("pacer queue lock poisoned");
        while guard.len() >= self.limit {
            let (new_guard, _timeout_result) = self
                .cv
                .wait_timeout(guard, Duration::from_millis(100))
                .expect("pacer queue condvar poisoned");
            guard = new_guard;
        }
        guard.push_back(item);
    }

    /// FIFO 順に 1 件取り出し、 lock 解放後に `notify_one` する。
    ///
    /// 戻り値が `Some` / `None` のどちらでも notify するため、
    /// callback の Ok / Err 両分岐で `pop` を呼びさえすれば書き手は起こされる。
    pub fn pop(&self) -> Option<T> {
        let item = {
            let mut guard = self.queue.lock().expect("pacer queue lock poisoned");
            guard.pop_front()
        };
        // lock 解放後に notify する (lock 内で notify すると待機側が起きた直後に
        // lock 待ちで再度 block する性能事故を招くため)。
        self.cv.notify_one();
        item
    }

    /// 現在のキュー長 (テスト用)。
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.queue.lock().expect("pacer queue lock poisoned").len()
    }

    /// キューが空か (テスト用、 `len` に対する clippy `len_without_is_empty` 対策)。
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.queue
            .lock()
            .expect("pacer queue lock poisoned")
            .is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Instant;

    /// (i) len() >= limit で push_wait が block することを確認する。
    #[test]
    fn push_wait_blocks_when_at_limit() {
        let pacer = Arc::new(Pacer::<u32>::new(2));
        pacer.push_wait(1);
        pacer.push_wait(2);
        assert_eq!(pacer.len(), 2, "上限まで push した状態でキュー長は 2");

        // 別スレッドで push_wait を実行、 これは block するはず
        let pacer_clone = Arc::clone(&pacer);
        let barrier = Arc::new(Barrier::new(2));
        let barrier_clone = Arc::clone(&barrier);
        let handle = thread::spawn(move || {
            barrier_clone.wait();
            pacer_clone.push_wait(3);
        });
        // 書き手が push_wait に入る直前まで同期
        barrier.wait();
        // 少し待って block していることを確認 (完了なら len == 3 のはず)
        thread::sleep(Duration::from_millis(50));
        assert_eq!(pacer.len(), 2, "block 中はキュー長が 2 のまま");

        // pop で 1 個抜いて書き手を起こす
        assert_eq!(pacer.pop(), Some(1));
        handle.join().expect("書き手スレッドが panic");
        assert_eq!(
            pacer.len(),
            2,
            "書き手復帰後にキュー長は 2 (pop 1 + push 3)"
        );
    }

    /// (ii) callback 側 pop で書き手の push_wait が起こされることを確認する。
    #[test]
    fn pop_wakes_waiter_promptly() {
        let pacer = Arc::new(Pacer::<u32>::new(1));
        pacer.push_wait(1);

        let pacer_clone = Arc::clone(&pacer);
        let handle = thread::spawn(move || {
            let start = Instant::now();
            pacer_clone.push_wait(2);
            start.elapsed()
        });

        // 書き手が wait に入る時間を確保
        thread::sleep(Duration::from_millis(20));
        // pop で notify
        assert_eq!(pacer.pop(), Some(1));

        let elapsed = handle.join().expect("書き手スレッドが panic");
        // pop の notify で timeout (100ms) より十分短い時間で復帰する
        assert!(
            elapsed < Duration::from_millis(80),
            "pop の直後に push_wait が復帰する (実測 {:?})",
            elapsed
        );
    }

    /// (iii) wait_timeout 経路で待機継続することを確認する。
    ///       pop されないまま 200ms 以上待っても block 継続 (timeout 2 回以上)。
    #[test]
    fn push_wait_keeps_waiting_across_timeouts() {
        let pacer = Arc::new(Pacer::<u32>::new(1));
        pacer.push_wait(1);

        let pacer_clone = Arc::clone(&pacer);
        let handle = thread::spawn(move || {
            let start = Instant::now();
            pacer_clone.push_wait(2);
            start.elapsed()
        });

        // timeout 100ms が 2 回以上発生する 250ms を待つ
        thread::sleep(Duration::from_millis(250));
        assert_eq!(pacer.len(), 1, "pop していないので書き手は block 継続");

        // 解除
        assert_eq!(pacer.pop(), Some(1));
        let elapsed = handle.join().expect("書き手スレッドが panic");
        // 少なくとも 250ms 以上待機していることを確認
        assert!(
            elapsed >= Duration::from_millis(250),
            "少なくとも 250ms は待機する (実測 {:?})",
            elapsed
        );
    }

    /// (iv) FIFO 順で保持されることを確認する。
    #[test]
    fn preserves_fifo_order() {
        let pacer = Pacer::<u32>::new(10);
        pacer.push_wait(1);
        pacer.push_wait(2);
        pacer.push_wait(3);
        assert_eq!(pacer.pop(), Some(1));
        assert_eq!(pacer.pop(), Some(2));
        assert_eq!(pacer.pop(), Some(3));
        assert_eq!(pacer.pop(), None);
    }

    /// (v) Err 経路シミュレーション: pop() は Some/None のどちらでも notify する。
    ///     nvcodec の callback Err 分岐でも pop を呼びさえすれば書き手は解放される。
    #[test]
    fn pop_notifies_even_after_last_item() {
        let pacer = Arc::new(Pacer::<u32>::new(1));
        pacer.push_wait(1);

        // 書き手を block 状態に置く
        let pacer_clone = Arc::clone(&pacer);
        let handle = thread::spawn(move || {
            pacer_clone.push_wait(2);
        });
        thread::sleep(Duration::from_millis(20));

        // 1 個目 pop (書き手が起こされる)
        assert_eq!(pacer.pop(), Some(1));
        // 書き手が復帰する
        handle.join().expect("書き手スレッドが panic");

        // 追加検証: 2 個目 pop (書き手の push した 2 を取り出す) → キューは空になる
        assert_eq!(pacer.pop(), Some(2));
        // 3 回目の pop は None が返るが、 その後も notify セマンティクスは
        // 保たれる (契約テストとして、 empty 状態の pop でも panic しないことを確認)
        assert_eq!(pacer.pop(), None);

        // 直後にもう一度書き手を走らせて、 empty からの push_wait が block しないことを確認
        pacer.push_wait(3);
        assert_eq!(pacer.pop(), Some(3));
    }
}
