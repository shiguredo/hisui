//! `src/media.rs` の MediaFrame と `src/text.rs` の TextFrame に対する PBT。
//!
//! MediaFrame の各バリアント (`Audio` / `Video` / `Text`) について、`timestamp()`、
//! `new_text` → `expect_text` の round-trip、`kind_name()` の対応を proptest で範囲網羅的に検証する。

use std::time::Duration;

use hisui::audio::{AudioFormat, AudioFrame, Channels, SampleRate};
use hisui::video::{VideoFormat, VideoFrame};
use hisui::{MediaFrame, TextFrame};
use proptest::prelude::*;

/// 任意の Duration を生成する。
fn arb_duration() -> impl Strategy<Value = Duration> {
    any::<u64>().prop_map(Duration::from_nanos)
}

/// f32 のうち NaN と Inf を除いた有限値を生成する (PartialEq で扱いやすいため)。
fn arb_finite_f32() -> impl Strategy<Value = f32> {
    any::<f32>().prop_filter("NaN と Inf を除外", |x| x.is_finite())
}

/// 任意の TextFrame を生成する。`start <= end` の制約は本 struct が持たないため任意ペア。
fn arb_text_frame() -> impl Strategy<Value = TextFrame> {
    (
        arb_duration(),
        arb_duration(),
        any::<String>(),
        proptest::option::of(any::<String>()),
        proptest::option::of(arb_finite_f32()),
        proptest::option::of(arb_finite_f32()),
    )
        .prop_map(
            |(start, end, text, language, no_speech_prob, avg_logprob)| TextFrame {
                start,
                end,
                text,
                language,
                no_speech_prob,
                avg_logprob,
            },
        )
}

/// 任意の timestamp を持つ AudioFrame を生成する (他フィールドは固定値)。
fn arb_audio_frame() -> impl Strategy<Value = AudioFrame> {
    arb_duration().prop_map(|timestamp| AudioFrame {
        data: Vec::new(),
        format: AudioFormat::Opus,
        channels: Channels::MONO,
        sample_rate: SampleRate::HZ_48000,
        timestamp,
        sample_entry: None,
    })
}

/// 任意の timestamp を持つ VideoFrame を生成する (他フィールドは固定値)。
fn arb_video_frame() -> impl Strategy<Value = VideoFrame> {
    arb_duration().prop_map(|timestamp| VideoFrame {
        data: Vec::new(),
        format: VideoFormat::I420,
        keyframe: true,
        size: None,
        timestamp,
        sample_entry: None,
    })
}

proptest! {
    /// `MediaFrame::new_text(frame).timestamp()` は常に `frame.start` を返す (`end` ではない)。
    #[test]
    fn media_frame_text_timestamp_returns_start(frame in arb_text_frame()) {
        let start = frame.start;
        let media = MediaFrame::new_text(frame);
        prop_assert_eq!(media.timestamp(), start);
    }

    /// `MediaFrame::new_audio(frame).timestamp()` は `frame.timestamp` を返す。
    #[test]
    fn media_frame_audio_timestamp_returns_frame_timestamp(frame in arb_audio_frame()) {
        let timestamp = frame.timestamp;
        let media = MediaFrame::new_audio(frame);
        prop_assert_eq!(media.timestamp(), timestamp);
    }

    /// `MediaFrame::new_video(frame).timestamp()` は `frame.timestamp` を返す。
    #[test]
    fn media_frame_video_timestamp_returns_frame_timestamp(frame in arb_video_frame()) {
        let timestamp = frame.timestamp;
        let media = MediaFrame::new_video(frame);
        prop_assert_eq!(media.timestamp(), timestamp);
    }

    /// `MediaFrame::new_text(frame).expect_text()` で得た TextFrame は全フィールドを保持する。
    #[test]
    fn text_frame_round_trips_via_media_frame(frame in arb_text_frame()) {
        let expected = frame.clone();
        let media = MediaFrame::new_text(frame);
        let restored = media.expect_text().expect("Text バリアントを返す想定");
        prop_assert_eq!(restored.start, expected.start);
        prop_assert_eq!(restored.end, expected.end);
        prop_assert_eq!(&restored.text, &expected.text);
        prop_assert_eq!(&restored.language, &expected.language);
        prop_assert_eq!(restored.no_speech_prob, expected.no_speech_prob);
        prop_assert_eq!(restored.avg_logprob, expected.avg_logprob);
    }

    /// `MediaFrame::kind_name()` は各バリアントに対応する文字列を返す。
    #[test]
    fn kind_name_matches_variant(
        audio in arb_audio_frame(),
        video in arb_video_frame(),
        text in arb_text_frame(),
    ) {
        prop_assert_eq!(MediaFrame::new_audio(audio).kind_name(), "audio");
        prop_assert_eq!(MediaFrame::new_video(video).kind_name(), "video");
        prop_assert_eq!(MediaFrame::new_text(text).kind_name(), "text");
    }
}
