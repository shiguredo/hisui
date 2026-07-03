//! `src/ml/audio/vad.rs` の integration テスト。
//!
//! VadGate 全体テストは SileroVad の実推論が必要になるため `tests/test_ml_audio_silero_vad.rs` に
//! 集約する。本ファイルでは実推論を経由しない SpeechSegment のヘルパー動作を検証する。

#![cfg(feature = "candle")]

use std::time::Duration;

use hisui::ml::audio::vad::SpeechSegment;
use hisui::probability::Probability;

/// `SpeechSegment::start_time` / `end_time` は 16 kHz サンプル通し番号を丸め誤差ゼロで Duration に変換する。
#[test]
fn speech_segment_time_helpers_match_expected_duration() {
    // start=1 秒 (16000 サンプル)、end=5 秒 (80000 サンプル)、確率 0.9
    let seg = SpeechSegment {
        start_sample: 16000,
        end_sample: 80000,
        max_probability: Probability::new(0.9).expect("0.9 は有効"),
    };
    assert_eq!(seg.start_time(), Duration::from_secs(1));
    assert_eq!(seg.end_time(), Duration::from_secs(5));
}

/// 1 サンプル = 62_500 ns で厳密に一致する (16 kHz は 1_000_000_000 の約数)。
#[test]
fn speech_segment_time_helpers_have_no_rounding_error() {
    let seg = SpeechSegment {
        start_sample: 1,
        end_sample: 3,
        max_probability: Probability::new(0.0).expect("0.0 は有効"),
    };
    // 1 サンプル分 = 62_500 ns、3 サンプル分 = 187_500 ns
    assert_eq!(seg.start_time(), Duration::from_nanos(62_500));
    assert_eq!(seg.end_time(), Duration::from_nanos(187_500));
}
