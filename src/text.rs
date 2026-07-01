use std::time::Duration;

/// 文字起こし結果や将来のテキストメタデータを表すフレーム。
#[derive(Debug, Clone)]
pub struct TextFrame {
    /// 発話開始時刻 (track 基準、`AudioFrame.timestamp` / `VideoFrame.timestamp` と同じ意味論)
    pub start: Duration,
    /// 発話終了時刻。`start <= end` を呼び出し側が保証する (validation は持たない)
    pub end: Duration,
    /// 文字起こしテキスト等
    pub text: String,
    /// ISO 639-1 (2 文字小文字) の言語コード ("ja" 等)。検出失敗時や言語推定なしの場合は None
    pub language: Option<String>,
    /// Whisper の no_speech_prob (幻覚指標、0.0 - 1.0)。Whisper 以外の生成元では None
    pub no_speech_prob: Option<f32>,
    /// Whisper の平均 log probability (信頼度目安)。Whisper 以外の生成元では None
    pub avg_logprob: Option<f32>,
}
