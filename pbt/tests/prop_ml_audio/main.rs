//! `src/ml/audio/` の PBT。
//!
//! resample / buffer / VadGate 用の SpeechSegment ヘルパーに対する不変条件を proptest で検証する。
//! 実 Silero VAD 推論は integration テスト側 (`tests/test_ml_audio_silero_vad.rs`) で担う。

mod buffer_props;
mod resample_props;
mod vad_props;
