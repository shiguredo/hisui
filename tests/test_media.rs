//! MediaFrame の Text バリアント関連 API を確認する。

use std::time::Duration;

use hisui::audio::{AudioFormat, AudioFrame, Channels, SampleRate};
use hisui::video::{VideoFormat, VideoFrame};
use hisui::{MediaFrame, TextFrame};

/// テスト用の AudioFrame を作るヘルパー。
fn make_audio_frame() -> AudioFrame {
    AudioFrame {
        data: Vec::new(),
        format: AudioFormat::Opus,
        channels: Channels::MONO,
        sample_rate: SampleRate::HZ_48000,
        timestamp: Duration::from_millis(100),
        sample_entry: None,
    }
}

/// テスト用の VideoFrame を作るヘルパー。
fn make_video_frame() -> VideoFrame {
    VideoFrame {
        data: Vec::new(),
        format: VideoFormat::I420,
        keyframe: true,
        size: None,
        timestamp: Duration::from_millis(200),
        sample_entry: None,
    }
}

/// テスト用の TextFrame を作るヘルパー。
fn make_text_frame() -> TextFrame {
    TextFrame {
        start: Duration::from_secs(1),
        end: Duration::from_secs(5),
        text: "こんにちは".to_owned(),
        no_speech_prob: Some(0.1),
        avg_logprob: Some(-0.5),
    }
}

/// MediaFrame::new_text は TextFrame を Arc に包んで Text バリアントを返す。
#[test]
fn new_text_wraps_in_arc() {
    let frame = make_text_frame();
    let media = MediaFrame::new_text(frame.clone());
    let text = media.expect_text().expect("Text バリアントを返す想定");
    assert_eq!(text.text, frame.text);
    assert_eq!(text.start, frame.start);
    assert_eq!(text.end, frame.end);
}

/// MediaFrame::timestamp() は Text 入力で `start` を返す (`end` ではない)。
#[test]
fn timestamp_returns_start_for_text() {
    let media = MediaFrame::new_text(make_text_frame());
    // start=1s, end=5s で明示的に異なる値を渡しているため、`end` を返した場合は値で検出できる
    assert_eq!(media.timestamp(), Duration::from_secs(1));
}

/// MediaFrame::timestamp() は Audio 入力で AudioFrame::timestamp を返す。
#[test]
fn timestamp_returns_timestamp_for_audio() {
    let audio_frame = AudioFrame {
        timestamp: Duration::from_millis(500),
        ..make_audio_frame()
    };
    let media = MediaFrame::new_audio(audio_frame);
    assert_eq!(media.timestamp(), Duration::from_millis(500));
}

/// MediaFrame::timestamp() は Video 入力で VideoFrame::timestamp を返す。
#[test]
fn timestamp_returns_timestamp_for_video() {
    let video_frame = VideoFrame {
        timestamp: Duration::from_millis(700),
        ..make_video_frame()
    };
    let media = MediaFrame::new_video(video_frame);
    assert_eq!(media.timestamp(), Duration::from_millis(700));
}

/// MediaFrame::kind_name() は各バリアントに対応する文字列を返す。
#[test]
fn kind_name_returns_variant_name() {
    let audio = MediaFrame::new_audio(make_audio_frame());
    let video = MediaFrame::new_video(make_video_frame());
    let text = MediaFrame::new_text(make_text_frame());

    assert_eq!(audio.kind_name(), "audio");
    assert_eq!(video.kind_name(), "video");
    assert_eq!(text.kind_name(), "text");
}

/// expect_text は Text 入力で Ok、Audio / Video 入力で厳密なエラーメッセージの Err を返す。
#[test]
fn expect_text_succeeds_only_for_text() {
    let text = MediaFrame::new_text(make_text_frame());
    assert!(text.expect_text().is_ok());

    let audio = MediaFrame::new_audio(make_audio_frame());
    let msg = audio
        .expect_text()
        .expect_err("Audio 入力では Err になる想定")
        .display()
        .to_string();
    assert_eq!(msg, "expected text sample, but got audio sample");

    let video = MediaFrame::new_video(make_video_frame());
    let msg = video
        .expect_text()
        .expect_err("Video 入力では Err になる想定")
        .display()
        .to_string();
    assert_eq!(msg, "expected text sample, but got video sample");
}

/// expect_audio は Text 入力で厳密なエラーメッセージの Err を返す。
#[test]
fn expect_audio_returns_text_kind_error() {
    let text = MediaFrame::new_text(make_text_frame());
    let msg = text
        .expect_audio()
        .expect_err("Text 入力では Err になる想定")
        .display()
        .to_string();
    assert_eq!(msg, "expected audio sample, but got text sample");
}

/// expect_video は Text 入力で厳密なエラーメッセージの Err を返す。
#[test]
fn expect_video_returns_text_kind_error() {
    let text = MediaFrame::new_text(make_text_frame());
    let msg = text
        .expect_video()
        .expect_err("Text 入力では Err になる想定")
        .display()
        .to_string();
    assert_eq!(msg, "expected video sample, but got text sample");
}
