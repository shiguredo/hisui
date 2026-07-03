//! VAD 用の設定構造体。

use std::time::Duration;

use crate::probability::Probability;

/// VadGate の閾値ゲートと発話区間確定条件。
#[derive(Debug, Clone, Copy)]
pub struct VadConfig {
    /// SileroVad の発話確率がこの値以上のチャンクを「発話中」と判定する。
    pub threshold: Probability,
    /// 「発話中」判定が連続してこの時間以上続くと SpeechSegment として確定する。
    pub min_speech: Duration,
    /// 発話中のあと「発話中でない」判定がこの時間以上続くと、直前の SpeechSegment を確定する。
    pub min_silence: Duration,
}

impl Default for VadConfig {
    /// Silero VAD 公式 python wrapper のデフォルト値に揃える。
    fn default() -> Self {
        Self {
            threshold: Probability::new(0.5).expect("0.5 is a valid probability"),
            min_speech: Duration::from_millis(250),
            min_silence: Duration::from_millis(100),
        }
    }
}
