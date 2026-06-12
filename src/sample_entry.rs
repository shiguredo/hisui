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

use crate::audio::AudioFrame;
use crate::video::VideoFrame;

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

/// エンコード済みフレーム sample_entry 不変条件の writer 入口での解決結果。
///
/// 圧縮フォーマット（codec_name が `Some`）のフレームに対して、`sample_entry` の有無と
/// 補完値（fallback）の状態から writer 側の処理方針を表現する。
#[derive(Debug)]
pub enum SampleEntryResolution<T> {
    /// 通常パス。元のフレームをそのまま下流に渡す（既に `sample_entry` を持っているか、
    /// 圧縮対象外で違反検知の対象外）。
    Pass,
    /// 違反パスで補完値が確立済みのため、補完済みフレームに差し替える。
    /// writer は警告ログを出してからこの値を下流に渡す。
    Patched(T),
    /// 違反パスで補完値が未確立のため、当該フレームを skip する。
    /// writer は警告ログを出してから処理を中断する。
    Skip,
}

/// エンコード済みフレーム不変条件（圧縮フレームは常に `sample_entry: Some`）の検知と、
/// fallback 補完値の更新・適用を一括で行う（音声用）。
///
/// - 圧縮フォーマットでない（`codec_name() == None`）場合は `Pass` を返し、何もしない。
/// - 圧縮フォーマットで `sample_entry: Some` の場合は `fallback` を更新して `Pass` を返す。
/// - 圧縮フォーマットで `sample_entry: None` の場合:
///   - `fallback` が `Some` なら補完済みの新フレームを `Patched` で返す。
///   - `fallback` が `None` なら `Skip` を返す。
///
/// 警告ログ（`tracing::warn!`）は呼び出し側の writer ごとに静的メッセージで出すため、
/// この関数では出力しない。
pub fn try_resolve_audio_sample_entry(
    sample: &AudioFrame,
    fallback: &mut Option<SharedSampleEntry>,
) -> SampleEntryResolution<AudioFrame> {
    if sample.format.codec_name().is_none() {
        // 生フォーマットは不変条件の対象外。writer 入口に来ない設計だが防御的に通過させる。
        return SampleEntryResolution::Pass;
    }
    match &sample.sample_entry {
        Some(entry) => {
            // 通常パス: fallback を更新してフレームをそのまま下流に流す。
            *fallback = Some(entry.clone());
            SampleEntryResolution::Pass
        }
        None => match fallback.clone() {
            Some(fb) => {
                // 違反 + fallback あり: 補完済みフレームを生成して返す。
                let patched = AudioFrame {
                    sample_entry: Some(fb),
                    ..sample.clone()
                };
                SampleEntryResolution::Patched(patched)
            }
            None => SampleEntryResolution::Skip,
        },
    }
}

/// 同 [`try_resolve_audio_sample_entry`] の映像用。挙動と戻り値の意味は同じ。
pub fn try_resolve_video_sample_entry(
    frame: &VideoFrame,
    fallback: &mut Option<SharedSampleEntry>,
) -> SampleEntryResolution<VideoFrame> {
    if frame.format.codec_name().is_none() {
        return SampleEntryResolution::Pass;
    }
    match &frame.sample_entry {
        Some(entry) => {
            *fallback = Some(entry.clone());
            SampleEntryResolution::Pass
        }
        None => match fallback.clone() {
            Some(fb) => {
                let patched = VideoFrame {
                    sample_entry: Some(fb),
                    ..frame.clone()
                };
                SampleEntryResolution::Patched(patched)
            }
            None => SampleEntryResolution::Skip,
        },
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
}
