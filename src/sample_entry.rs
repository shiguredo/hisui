//! 映像・音声で共有する sample entry の共通型。
//!
//! 各エンコーダが出力フレームに載せる sample entry を `Arc` で包むことで、
//! 毎フレームの値コピーを Arc clone に抑えつつ、変化検知を `Arc::ptr_eq` で
//! 短絡できるようにする。音声・映像のフレーム型（[`crate::audio::AudioFrame`] /
//! [`crate::video::VideoFrame`]）の `sample_entry` フィールドで共通利用する。

use std::sync::Arc;

use shiguredo_mp4::boxes::SampleEntry;

/// 映像・音声で共有する sample entry。
///
/// `Arc` で包むことで毎フレームのコピーを Arc clone に抑え、変化検知を
/// `Arc::ptr_eq` で短絡できる。`SampleEntry` 自体は `PartialEq` / `Eq` を
/// 実装しているため、別 Arc 同士でも実体比較で変化を判定できる。
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

    /// 直前の entry から変化したかを判定する。
    ///
    /// - `prev` が `None`（初回）なら、まだ何も確定していないので変化ありとして `true` を返す。
    ///   これにより writer は最初のフレームで必ず entry を muxer に渡せる
    ///   （muxer はトラックの最初のサンプルに sample entry を要求するため）。
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
    fn 初回は変化ありと判定する() {
        let entry = SharedSampleEntry::new(make_sample_entry(&[0x01]));
        // prev が None のときは、まだ何も確定していないので true を返す。
        assert!(entry.changed_since(None));
    }

    #[test]
    fn 同一_arc_は変化なしと判定する() {
        let entry = SharedSampleEntry::new(make_sample_entry(&[0x01]));
        // clone は Arc を共有するので ptr_eq で短絡して false になる。
        let cloned = entry.clone();
        assert!(!entry.changed_since(Some(&cloned)));
    }

    #[test]
    fn 別_arc_でも実体が同値なら変化なしと判定する() {
        let a = SharedSampleEntry::new(make_sample_entry(&[0x01]));
        // 同じ内容を別々に new するので Arc は別だが実体は等しい。
        let b = SharedSampleEntry::new(make_sample_entry(&[0x01]));
        assert!(!a.changed_since(Some(&b)));
    }

    #[test]
    fn 別_arc_で実体が異なれば変化ありと判定する() {
        let a = SharedSampleEntry::new(make_sample_entry(&[0x01]));
        // payload が異なるので実体比較で相違が出る。
        let b = SharedSampleEntry::new(make_sample_entry(&[0x02]));
        assert!(a.changed_since(Some(&b)));
    }
}
