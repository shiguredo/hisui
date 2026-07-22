//! TextFrame の構築と Clone を確認する。

use std::time::Duration;

use hisui::TextFrame;
use hisui::text::LanguageCode;

/// TextFrame の全フィールドが期待通り構築できる。
#[test]
fn text_frame_construction() {
    let frame = TextFrame {
        start: Duration::from_millis(500),
        end: Duration::from_millis(2500),
        text: "こんにちは".to_owned(),
        language: Some(LanguageCode::new("ja")),
        no_speech_prob: Some(0.05),
        avg_logprob: Some(-0.3),
    };

    assert_eq!(frame.start, Duration::from_millis(500));
    assert_eq!(frame.end, Duration::from_millis(2500));
    assert_eq!(frame.text, "こんにちは");
    assert_eq!(
        frame.language.as_ref().map(LanguageCode::as_str),
        Some("ja")
    );
    assert_eq!(frame.no_speech_prob, Some(0.05));
    assert_eq!(frame.avg_logprob, Some(-0.3));
}

/// TextFrame::Clone でフィールド値が保持される。
#[test]
fn text_frame_clone() {
    let original = TextFrame {
        start: Duration::from_millis(0),
        end: Duration::from_millis(100),
        text: "テスト".to_owned(),
        language: None,
        no_speech_prob: None,
        avg_logprob: None,
    };
    let cloned = original.clone();

    assert_eq!(cloned.start, original.start);
    assert_eq!(cloned.end, original.end);
    assert_eq!(cloned.text, original.text);
    assert_eq!(cloned.language, original.language);
    assert_eq!(cloned.no_speech_prob, original.no_speech_prob);
    assert_eq!(cloned.avg_logprob, original.avg_logprob);
}
