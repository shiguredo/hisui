//! 映像・音声で共有するサンプルエントリーの共通型。
//!
//! 各エンコーダが出力フレームに載せるサンプルエントリーを `Arc` で包むことで、
//! フレーム間の受け渡しや writer での前回値保持を Arc clone で安価に行い、
//! 変化検知を `Arc::ptr_eq` で短絡できるようにする。なお muxer へ渡す際は
//! 生の `SampleEntry` を要求するため、その箇所では中身を取り出してコピーする。
//! 音声・映像のフレーム型（[`crate::audio::AudioFrame`] /
//! [`crate::video::VideoFrame`]）の `sample_entry` フィールドで共通利用する。

use std::sync::Arc;

use shiguredo_mp4::boxes::SampleEntry;

/// 映像・音声で共有するサンプルエントリー。
///
/// `Arc` で包むことで、フレーム間の受け渡しや writer での前回値保持を
/// Arc clone で安価に行い、変化検知を `Arc::ptr_eq` で短絡できる。
/// `SampleEntry` 自体は `PartialEq` / `Eq` を実装しているため、
/// 別 Arc 同士でも実体比較で変化を判定できる。
#[derive(Debug, Clone)]
pub struct SharedSampleEntry(Arc<SampleEntry>);

impl SharedSampleEntry {
    /// `SampleEntry` を共有可能な形に包む。
    pub fn new(entry: SampleEntry) -> Self {
        Self(Arc::new(entry))
    }

    /// 内側の `SampleEntry` への参照を返す。
    pub fn get(&self) -> &SampleEntry {
        &self.0
    }

    /// 2 つの `SharedSampleEntry` が同一の `Arc` を共有しているかを判定する。
    ///
    /// `changed_since` の `Arc::ptr_eq` 短絡経路が崩れた場合
    /// （`get().clone()` で再 wrap する実装に書き換わるなど）に検知できるよう、Arc の同一性
    /// だけを観測できる API として用意する。
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// 直前の entry から変化したかを判定する。
    ///
    /// - `prev` が `None`（初回）なら、まだ何も確定していないので変化ありとして `true` を返す。
    ///   これにより writer は最初のフレームで必ず entry を muxer に渡せる
    ///   （muxer はトラックの最初のサンプルにサンプルエントリーを要求するため）。
    /// - 同一の `Arc` を指している場合は `ptr_eq` で短絡し、実体比較を省いて `false` を返す。
    /// - 別 `Arc` の場合だけ `PartialEq` で実体比較し、相違なら `true`・同値なら `false` を返す。
    pub fn changed_since(&self, prev: Option<&SharedSampleEntry>) -> bool {
        match prev {
            None => true,
            Some(prev) => {
                if Arc::ptr_eq(&self.0, &prev.0) {
                    // 同一 Arc なら確実に同値なので実体比較を省く。
                    false
                } else {
                    self.0 != prev.0
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use shiguredo_mp4::{
        BoxSize, BoxType,
        boxes::{SampleEntry, UnknownBox},
    };

    use super::*;

    // テスト用の SampleEntry を作る。payload の中身で実体の違いを作り分ける。
    fn make_sample_entry(payload: &[u8]) -> SampleEntry {
        SampleEntry::Unknown(UnknownBox {
            box_type: BoxType::Normal(*b"dumy"),
            box_size: BoxSize::U32(8 + payload.len() as u32),
            payload: payload.to_vec(),
        })
    }

    #[test]
    fn changed_since_returns_true_for_none_prev() {
        let entry = SharedSampleEntry::new(make_sample_entry(&[0x01]));
        // prev が None のときは、まだ何も確定していないので true を返す。
        assert!(entry.changed_since(None));
    }

    #[test]
    fn changed_since_returns_false_for_same_arc() {
        let entry = SharedSampleEntry::new(make_sample_entry(&[0x01]));
        // clone は Arc を共有するので ptr_eq で短絡して false になる。
        let cloned = entry.clone();
        assert!(!entry.changed_since(Some(&cloned)));
    }

    #[test]
    fn changed_since_returns_false_for_equal_value_in_different_arc() {
        let a = SharedSampleEntry::new(make_sample_entry(&[0x01]));
        // 同じ内容を別々に new するので Arc は別だが実体は等しい。
        let b = SharedSampleEntry::new(make_sample_entry(&[0x01]));
        assert!(!a.changed_since(Some(&b)));
    }

    #[test]
    fn changed_since_returns_true_for_different_value() {
        let a = SharedSampleEntry::new(make_sample_entry(&[0x01]));
        // payload が異なるので実体比較で相違が出る。
        let b = SharedSampleEntry::new(make_sample_entry(&[0x02]));
        assert!(a.changed_since(Some(&b)));
    }

    #[test]
    fn ptr_eq_returns_true_for_clone() {
        // clone した SharedSampleEntry は同一の Arc を指す。
        let a = SharedSampleEntry::new(make_sample_entry(&[0x01]));
        let b = a.clone();
        assert!(a.ptr_eq(&b));
    }

    #[test]
    fn ptr_eq_returns_false_for_separately_constructed() {
        // 同一内容でも別々に new した場合は別の Arc になり、ptr_eq は false。
        let a = SharedSampleEntry::new(make_sample_entry(&[0x01]));
        let b = SharedSampleEntry::new(make_sample_entry(&[0x01]));
        assert!(!a.ptr_eq(&b));
    }
}
