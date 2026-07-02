//! VAD 用の設定構造体。

/// VadGate の閾値ゲートと発話区間確定条件。
#[derive(Debug, Clone, Copy)]
pub struct VadConfig {
    /// SileroVad の発話確率がこの値以上のチャンクを「発話中」と判定する。
    pub threshold: f32,
    /// 「発話中」判定が連続してこの時間 (ミリ秒) 以上続くと SpeechSegment として確定する。
    pub min_speech_ms: u32,
    /// 発話中のあと「発話中でない」判定がこの時間 (ミリ秒) 以上続くと、直前の SpeechSegment を確定する。
    pub min_silence_ms: u32,
}

impl Default for VadConfig {
    /// Silero VAD 公式 python wrapper のデフォルト値に揃える。
    fn default() -> Self {
        Self {
            threshold: 0.5,
            min_speech_ms: 250,
            min_silence_ms: 100,
        }
    }
}
