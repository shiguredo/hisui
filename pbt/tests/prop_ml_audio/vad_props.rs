//! `src/ml/audio/vad.rs` の `SpeechSegment` に対する PBT。
//!
//! `VadGate` の集約ロジック単体の PBT は SileroVad 実推論と一体化しているため、integration テスト側
//! (`tests/test_ml_audio_silero_vad.rs`) で担当する。本ファイルでは PCM に依存しない範囲のプロパティ
//! (`SpeechSegment::start_time` / `end_time` の Duration 変換) を検証する。

use std::time::Duration;

use hisui::ml::audio::vad::SpeechSegment;
use proptest::prelude::*;

proptest! {
    /// SpeechSegment::start_time / end_time は 1 サンプル = 62_500 ns の対応で丸め誤差ゼロで変換する。
    #[test]
    fn speech_segment_duration_is_lossless(start in 0u64..=1_000_000_000, delta in 0u64..1_000_000) {
        let end = start.saturating_add(delta);
        let seg = SpeechSegment {
            start_sample: start,
            end_sample: end,
            max_probability: 0.0,
        };
        prop_assert_eq!(seg.start_time(), Duration::from_nanos(start * 62_500));
        prop_assert_eq!(seg.end_time(), Duration::from_nanos(end * 62_500));
        prop_assert!(seg.end_time() >= seg.start_time(), "end_time >= start_time が成り立つはず");
    }
}
