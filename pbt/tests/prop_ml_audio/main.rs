//! `src/ml/audio/` の PBT。
//!
//! buffer / VadGate 用の SpeechSegment ヘルパーに対する不変条件を proptest で検証する。
//! 実 Silero VAD 推論は integration テスト側 (`tests/test_ml_audio_silero_vad.rs`) で担う。
//! resample は audio 領域に移動したため、対応する PBT は `pbt/tests/prop_audio_resample.rs` にある。

mod buffer_props;
mod vad_props;
