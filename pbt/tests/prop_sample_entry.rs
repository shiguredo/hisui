//! `src/sample_entry.rs` の `resolve_audio_sample_entry` / `resolve_video_sample_entry`
//! に対する PBT
//!
//! writer 入口で sample_entry 不変条件違反を検知し、補完値で救済する純粋関数の
//! 性質を `(codec_name 有無 × sample_entry 有無 × fallback 有無)` の組合せで網羅的に検証する。

use std::time::Duration;

use hisui::audio::{AudioFormat, AudioFrame, Channels, SampleRate};
use hisui::sample_entry::{
    SampleEntryResolution, SharedSampleEntry, resolve_audio_sample_entry,
    resolve_video_sample_entry,
};
use hisui::video::{VideoFormat, VideoFrame, VideoFrameSize};
use proptest::prelude::*;
use shiguredo_mp4::{
    BoxSize, BoxType,
    boxes::{SampleEntry, UnknownBox},
};

// テスト用の SampleEntry を作る。payload の中身で実体の違いを作り分ける。
fn make_sample_entry(payload: Vec<u8>) -> SampleEntry {
    SampleEntry::Unknown(UnknownBox {
        box_type: BoxType::Normal(*b"dumy"),
        box_size: BoxSize::U32(8 + payload.len() as u32),
        payload,
    })
}

// codec_name() == None になる音声フォーマットの Strategy（生フォーマットのみ）。
fn raw_audio_format_strategy() -> impl Strategy<Value = AudioFormat> {
    Just(AudioFormat::I16Be)
}

// codec_name() == Some になる音声フォーマットの Strategy（圧縮フォーマットのみ）。
fn encoded_audio_format_strategy() -> impl Strategy<Value = AudioFormat> {
    prop_oneof![Just(AudioFormat::Opus), Just(AudioFormat::Aac)]
}

// codec_name() == None になる映像フォーマットの Strategy（生フォーマットのみ）。
fn raw_video_format_strategy() -> impl Strategy<Value = VideoFormat> {
    prop_oneof![Just(VideoFormat::I420), Just(VideoFormat::I420A)]
}

// codec_name() == Some になる映像フォーマットの Strategy（圧縮フォーマットのみ）。
fn encoded_video_format_strategy() -> impl Strategy<Value = VideoFormat> {
    prop_oneof![
        Just(VideoFormat::H264),
        Just(VideoFormat::H264AnnexB),
        Just(VideoFormat::H265),
        Just(VideoFormat::Vp8),
        Just(VideoFormat::Vp9),
        Just(VideoFormat::Av1),
    ]
}

// 任意の SharedSampleEntry を生成する Strategy。payload の中身は乱数。
fn shared_sample_entry_strategy() -> impl Strategy<Value = SharedSampleEntry> {
    prop::collection::vec(any::<u8>(), 1..16)
        .prop_map(|payload| SharedSampleEntry::new(make_sample_entry(payload)))
}

// フォーマットと sample_entry を指定して `AudioFrame` を作る。
fn build_audio_frame(format: AudioFormat, sample_entry: Option<SharedSampleEntry>) -> AudioFrame {
    AudioFrame {
        data: vec![],
        format,
        channels: Channels::STEREO,
        sample_rate: SampleRate::HZ_48000,
        timestamp: Duration::ZERO,
        sample_entry,
    }
}

// フォーマットと sample_entry を指定して `VideoFrame` を作る。
fn build_video_frame(format: VideoFormat, sample_entry: Option<SharedSampleEntry>) -> VideoFrame {
    VideoFrame {
        data: vec![],
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

proptest! {
    // 性質 P1（音声）: 生フォーマットでは sample_entry / fallback の有無に関わらず Pass を返し、
    // fallback を更新しない（生フォーマットは不変条件の対象外）。
    #[test]
    fn prop_audio_raw_format_always_passes_without_touching_fallback(
        format in raw_audio_format_strategy(),
        sample_entry in prop::option::of(shared_sample_entry_strategy()),
        initial_fallback in prop::option::of(shared_sample_entry_strategy()),
    ) {
        let frame = build_audio_frame(format, sample_entry);
        let mut fallback = initial_fallback.clone();
        let resolution = resolve_audio_sample_entry(&frame, &mut fallback);
        prop_assert!(matches!(resolution, SampleEntryResolution::Pass));
        // fallback が更新されていないことを Arc 同一性で確認する。
        match (&initial_fallback, &fallback) {
            (None, None) => {}
            (Some(before), Some(after)) => prop_assert!(before.ptr_eq(after)),
            _ => prop_assert!(false, "fallback の Some/None 状態が遷移している"),
        }
    }

    // 性質 P2（音声）: 圧縮 + sample_entry == Some なら Pass を返し、fallback が
    // 受信フレームの sample_entry と同一 Arc になる（直前値の保持を Arc 共有で行う）。
    #[test]
    fn prop_audio_encoded_with_sample_entry_passes_and_updates_fallback_with_shared_arc(
        entry in shared_sample_entry_strategy(),
        format in encoded_audio_format_strategy(),
        initial_fallback in prop::option::of(shared_sample_entry_strategy()),
    ) {
        let frame = build_audio_frame(format, Some(entry.clone()));
        let mut fallback = initial_fallback;
        let resolution = resolve_audio_sample_entry(&frame, &mut fallback);
        prop_assert!(matches!(resolution, SampleEntryResolution::Pass));
        let stored = fallback.as_ref().expect("通常パスで fallback が Some になる");
        prop_assert!(stored.ptr_eq(&entry));
    }

    // 性質 P3（音声）: 圧縮 + sample_entry == None + fallback == Some なら Patched を返し、
    // patched.sample_entry が fallback と同一 Arc を共有する。fallback 自体は変化しない。
    #[test]
    fn prop_audio_encoded_without_sample_entry_with_fallback_returns_patched_sharing_arc(
        fb_entry in shared_sample_entry_strategy(),
        format in encoded_audio_format_strategy(),
    ) {
        let frame = build_audio_frame(format, None);
        let mut fallback = Some(fb_entry.clone());
        let resolution = resolve_audio_sample_entry(&frame, &mut fallback);
        let patched = match resolution {
            SampleEntryResolution::Patched(p) => p,
            other => {
                prop_assert!(false, "Patched が返るべき（実際: {:?}）", other);
                unreachable!()
            }
        };
        let patched_entry = patched
            .sample_entry
            .as_ref()
            .expect("patched には sample_entry が載っている");
        prop_assert!(patched_entry.ptr_eq(&fb_entry));
        // fallback 自体は補完で消費しても更新されない（後続の正常フレームまで保持）。
        let after = fallback.as_ref().expect("違反パスでも fallback は保持される");
        prop_assert!(after.ptr_eq(&fb_entry));
    }

    // 性質 P4（音声）: 圧縮 + sample_entry == None + fallback == None なら Skip を返し、
    // fallback は None のまま。
    #[test]
    fn prop_audio_encoded_without_sample_entry_without_fallback_returns_skip(
        format in encoded_audio_format_strategy(),
    ) {
        let frame = build_audio_frame(format, None);
        let mut fallback: Option<SharedSampleEntry> = None;
        let resolution = resolve_audio_sample_entry(&frame, &mut fallback);
        prop_assert!(matches!(resolution, SampleEntryResolution::Skip));
        prop_assert!(fallback.is_none());
    }

    // 性質 P1（映像）: 生フォーマットでは sample_entry / fallback の有無に関わらず Pass + fallback 不変。
    #[test]
    fn prop_video_raw_format_always_passes_without_touching_fallback(
        format in raw_video_format_strategy(),
        sample_entry in prop::option::of(shared_sample_entry_strategy()),
        initial_fallback in prop::option::of(shared_sample_entry_strategy()),
    ) {
        let frame = build_video_frame(format, sample_entry);
        let mut fallback = initial_fallback.clone();
        let resolution = resolve_video_sample_entry(&frame, &mut fallback);
        prop_assert!(matches!(resolution, SampleEntryResolution::Pass));
        match (&initial_fallback, &fallback) {
            (None, None) => {}
            (Some(before), Some(after)) => prop_assert!(before.ptr_eq(after)),
            _ => prop_assert!(false, "fallback の Some/None 状態が遷移している"),
        }
    }

    // 性質 P2（映像）: 圧縮 + sample_entry == Some なら Pass + fallback が同一 Arc で更新。
    #[test]
    fn prop_video_encoded_with_sample_entry_passes_and_updates_fallback_with_shared_arc(
        entry in shared_sample_entry_strategy(),
        format in encoded_video_format_strategy(),
        initial_fallback in prop::option::of(shared_sample_entry_strategy()),
    ) {
        let frame = build_video_frame(format, Some(entry.clone()));
        let mut fallback = initial_fallback;
        let resolution = resolve_video_sample_entry(&frame, &mut fallback);
        prop_assert!(matches!(resolution, SampleEntryResolution::Pass));
        let stored = fallback.as_ref().expect("通常パスで fallback が Some になる");
        prop_assert!(stored.ptr_eq(&entry));
    }

    // 性質 P3（映像）: 圧縮 + None + fallback Some なら Patched + Arc 共有 + fallback 不変。
    #[test]
    fn prop_video_encoded_without_sample_entry_with_fallback_returns_patched_sharing_arc(
        fb_entry in shared_sample_entry_strategy(),
        format in encoded_video_format_strategy(),
    ) {
        let frame = build_video_frame(format, None);
        let mut fallback = Some(fb_entry.clone());
        let resolution = resolve_video_sample_entry(&frame, &mut fallback);
        let patched = match resolution {
            SampleEntryResolution::Patched(p) => p,
            other => {
                prop_assert!(false, "Patched が返るべき（実際: {:?}）", other);
                unreachable!()
            }
        };
        let patched_entry = patched
            .sample_entry
            .as_ref()
            .expect("patched には sample_entry が載っている");
        prop_assert!(patched_entry.ptr_eq(&fb_entry));
        let after = fallback.as_ref().expect("違反パスでも fallback は保持される");
        prop_assert!(after.ptr_eq(&fb_entry));
    }

    // 性質 P4（映像）: 圧縮 + None + fallback None なら Skip + fallback は None のまま。
    #[test]
    fn prop_video_encoded_without_sample_entry_without_fallback_returns_skip(
        format in encoded_video_format_strategy(),
    ) {
        let frame = build_video_frame(format, None);
        let mut fallback: Option<SharedSampleEntry> = None;
        let resolution = resolve_video_sample_entry(&frame, &mut fallback);
        prop_assert!(matches!(resolution, SampleEntryResolution::Skip));
        prop_assert!(fallback.is_none());
    }
}
