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

    /// 2 つの `SharedSampleEntry` が同一の `Arc` を共有しているかを判定する。
    ///
    /// fallback 補完値が直前の正常フレームの sample_entry を Arc 共有で保持できているかの
    /// テスト用途を想定する。`changed_since` の `Arc::ptr_eq` 短絡経路が崩れた場合
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

/// エンコード済みフレーム sample_entry 不変条件の writer 入口での解決結果。
///
/// 圧縮フォーマット（codec_name が `Some`）のフレームに対して、`sample_entry` の有無と
/// 補完値（fallback）の状態から writer 側の処理方針を表現する。
///
/// `T` は `Patched` でのみ使用するが、enum シグネチャ上は呼び出し側で型を統一するため
/// 全バリアントで `T` を持たせている（Pass / Skip 自体は値を持たない）。
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
/// - 圧縮フォーマットでない（`codec_name() == None`）場合は `Pass` を返し、`fallback` も
///   更新しない（生フォーマットは不変条件の対象外で、補完値の連続性に含めない）。
/// - 圧縮フォーマットで `sample_entry: Some` の場合は `fallback` を更新して `Pass` を返す。
/// - 圧縮フォーマットで `sample_entry: None` の場合:
///   - `fallback` が `Some` なら補完済みの新フレームを `Patched` で返す（fallback 自体は
///     直前の正常値を保持し続け、後続の正常フレームで更新されるまで変わらない）。
///   - `fallback` が `None` なら `Skip` を返す（`fallback` は `None` のまま、後続の正常
///     フレームで初めて `Some` になる）。
///
/// 違反パスでは `AudioFrame` 全体（`data: Vec<u8>` 含む）の deep copy が 1 回発生する。
/// 呼び出し側の writer で `Arc::new(patched)` を作る場合や、`WriterCore::handle_input_sample`
/// 経由でさらに deep copy する場合に重ねて発生するが、違反は基本起きない前提でコストは許容する。
///
/// 警告ログ（`tracing::warn!`）は呼び出し側の writer ごとに静的メッセージで出すため、
/// この関数では出力しない。
pub fn resolve_audio_sample_entry(
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
        None => match fallback.as_ref() {
            Some(fb) => {
                // 違反 + fallback あり: 補完済みフレームを生成して返す。
                // fallback の Arc を共有することで下流の `changed_since` 判定が
                // `Arc::ptr_eq` で短絡できる。
                let patched = AudioFrame {
                    sample_entry: Some(fb.clone()),
                    ..sample.clone()
                };
                SampleEntryResolution::Patched(patched)
            }
            None => SampleEntryResolution::Skip,
        },
    }
}

/// 同 [`resolve_audio_sample_entry`] の映像用。挙動と戻り値の意味は同じ。
pub fn resolve_video_sample_entry(
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
        None => match fallback.as_ref() {
            Some(fb) => {
                let patched = VideoFrame {
                    sample_entry: Some(fb.clone()),
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
    use std::time::Duration;

    use shiguredo_mp4::{
        BoxSize, BoxType,
        boxes::{SampleEntry, UnknownBox},
    };

    use super::*;
    use crate::audio::{AudioFormat, Channels, SampleRate};
    use crate::video::{VideoFormat, VideoFrameSize};

    // テスト用の SampleEntry を作る。payload の中身で実体の違いを作り分ける。
    fn make_sample_entry(payload: &[u8]) -> SampleEntry {
        SampleEntry::Unknown(UnknownBox {
            box_type: BoxType::Normal(*b"dumy"),
            box_size: BoxSize::U32(8 + payload.len() as u32),
            payload: payload.to_vec(),
        })
    }

    // 指定したフォーマットと sample_entry でテスト用の `AudioFrame` を作る。
    fn make_audio_frame(
        format: AudioFormat,
        sample_entry: Option<SharedSampleEntry>,
    ) -> AudioFrame {
        AudioFrame {
            data: vec![0x11, 0x22, 0x33],
            format,
            channels: Channels::STEREO,
            sample_rate: SampleRate::HZ_48000,
            timestamp: Duration::ZERO,
            sample_entry,
        }
    }

    // 指定したフォーマットと sample_entry でテスト用の `VideoFrame` を作る。
    fn make_video_frame(
        format: VideoFormat,
        sample_entry: Option<SharedSampleEntry>,
    ) -> VideoFrame {
        VideoFrame {
            data: vec![0x00, 0x00, 0x00, 0x01],
            format,
            keyframe: true,
            size: Some(VideoFrameSize {
                width: 16,
                height: 16,
            }),
            timestamp: Duration::ZERO,
            sample_entry,
        }
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

    #[test]
    fn resolve_audio_passes_through_raw_format_without_touching_fallback() {
        // 生フォーマット（codec_name == None）は不変条件の対象外。
        // sample_entry の有無に関係なく Pass を返し、fallback も更新しない。
        let sample = make_audio_frame(AudioFormat::I16Be, None);
        let mut fallback: Option<SharedSampleEntry> = None;
        let resolution = resolve_audio_sample_entry(&sample, &mut fallback);
        assert!(matches!(resolution, SampleEntryResolution::Pass));
        assert!(
            fallback.is_none(),
            "生フォーマットでは fallback を更新しないこと"
        );
    }

    #[test]
    fn resolve_audio_updates_fallback_with_shared_arc_on_normal_pass() {
        // 通常パス（圧縮 + sample_entry == Some）: fallback が同一 Arc で更新される。
        let entry = SharedSampleEntry::new(make_sample_entry(&[0x01]));
        let sample = make_audio_frame(AudioFormat::Aac, Some(entry.clone()));
        let mut fallback: Option<SharedSampleEntry> = None;
        let resolution = resolve_audio_sample_entry(&sample, &mut fallback);
        assert!(matches!(resolution, SampleEntryResolution::Pass));
        let stored = fallback.expect("通常パスで fallback が Some になること");
        assert!(
            stored.ptr_eq(&entry),
            "fallback は元の sample_entry と同一 Arc を共有していること"
        );
    }

    #[test]
    fn resolve_audio_returns_patched_with_shared_arc_when_fallback_available() {
        // 違反パス（圧縮 + sample_entry == None）+ fallback Some: 補完済みフレームが返る。
        // patched.sample_entry は fallback と同一 Arc を共有していること。
        let fb_entry = SharedSampleEntry::new(make_sample_entry(&[0x02]));
        let sample = make_audio_frame(AudioFormat::Aac, None);
        let mut fallback = Some(fb_entry.clone());
        let resolution = resolve_audio_sample_entry(&sample, &mut fallback);
        let patched = match resolution {
            SampleEntryResolution::Patched(p) => p,
            other => panic!("Patched が返ること（実際: {other:?}）"),
        };
        let patched_entry = patched
            .sample_entry
            .as_ref()
            .expect("補完済みフレームには sample_entry がある");
        assert!(
            patched_entry.ptr_eq(&fb_entry),
            "patched の sample_entry が fallback と同一 Arc を共有していること"
        );
        // fallback 自体は補完で消費しても変化しない（次の正常フレームで更新されるまで保持）。
        let after = fallback.expect("違反パスでも fallback は保持され続けること");
        assert!(after.ptr_eq(&fb_entry));
    }

    #[test]
    fn resolve_audio_returns_skip_when_fallback_unset() {
        // 違反パス + fallback None: Skip を返し、fallback は None のまま。
        let sample = make_audio_frame(AudioFormat::Aac, None);
        let mut fallback: Option<SharedSampleEntry> = None;
        let resolution = resolve_audio_sample_entry(&sample, &mut fallback);
        assert!(matches!(resolution, SampleEntryResolution::Skip));
        assert!(
            fallback.is_none(),
            "fallback 未確立のままなので Skip 後も None のまま"
        );
    }

    #[test]
    fn resolve_video_passes_through_raw_format_without_touching_fallback() {
        // 生フォーマット（I420）は不変条件の対象外。Pass + fallback 未更新。
        let frame = make_video_frame(VideoFormat::I420, None);
        let mut fallback: Option<SharedSampleEntry> = None;
        let resolution = resolve_video_sample_entry(&frame, &mut fallback);
        assert!(matches!(resolution, SampleEntryResolution::Pass));
        assert!(
            fallback.is_none(),
            "生フォーマットでは fallback を更新しないこと"
        );
    }

    #[test]
    fn resolve_video_updates_fallback_with_shared_arc_on_normal_pass() {
        // 通常パス: fallback が同一 Arc で更新される。
        let entry = SharedSampleEntry::new(make_sample_entry(&[0x0A]));
        let frame = make_video_frame(VideoFormat::Av1, Some(entry.clone()));
        let mut fallback: Option<SharedSampleEntry> = None;
        let resolution = resolve_video_sample_entry(&frame, &mut fallback);
        assert!(matches!(resolution, SampleEntryResolution::Pass));
        let stored = fallback.expect("通常パスで fallback が Some になること");
        assert!(
            stored.ptr_eq(&entry),
            "fallback は元の sample_entry と同一 Arc を共有していること"
        );
    }

    #[test]
    fn resolve_video_returns_patched_with_shared_arc_when_fallback_available() {
        // 違反パス + fallback Some: 補完済みフレームが返り、Arc を共有する。
        let fb_entry = SharedSampleEntry::new(make_sample_entry(&[0x0B]));
        let frame = make_video_frame(VideoFormat::Av1, None);
        let mut fallback = Some(fb_entry.clone());
        let resolution = resolve_video_sample_entry(&frame, &mut fallback);
        let patched = match resolution {
            SampleEntryResolution::Patched(p) => p,
            other => panic!("Patched が返ること（実際: {other:?}）"),
        };
        let patched_entry = patched
            .sample_entry
            .as_ref()
            .expect("補完済みフレームには sample_entry がある");
        assert!(
            patched_entry.ptr_eq(&fb_entry),
            "patched の sample_entry が fallback と同一 Arc を共有していること"
        );
        let after = fallback.expect("違反パスでも fallback は保持され続けること");
        assert!(after.ptr_eq(&fb_entry));
    }

    #[test]
    fn resolve_video_returns_skip_when_fallback_unset() {
        // 違反パス + fallback None: Skip を返し、fallback は None のまま。
        let frame = make_video_frame(VideoFormat::Av1, None);
        let mut fallback: Option<SharedSampleEntry> = None;
        let resolution = resolve_video_sample_entry(&frame, &mut fallback);
        assert!(matches!(resolution, SampleEntryResolution::Skip));
        assert!(
            fallback.is_none(),
            "fallback 未確立のままなので Skip 後も None のまま"
        );
    }
}
