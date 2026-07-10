use std::time::Duration;

/// 言語識別に使う ISO 639-1 (`ja` / `en` 等) または Whisper 拡張 (`haw` 等) のコード。
///
/// 生の `String` と型で分離することで、他の `String` フィールド (テキスト本体等) と混同しない
/// ようにする。妥当性 (Whisper tokenizer に該当コードが存在するか) は使用側 (`WhisperDecoder`)
/// で検証する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageCode(String);

impl LanguageCode {
    pub fn new(code: impl Into<String>) -> Self {
        Self(code.into())
    }

    pub fn get(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for LanguageCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 文字起こし結果や将来のテキストメタデータを表すフレーム。
#[derive(Debug, Clone)]
pub struct TextFrame {
    /// 発話開始時刻 (track 基準、`AudioFrame.timestamp` / `VideoFrame.timestamp` と同じ意味論)
    pub start: Duration,
    /// 発話終了時刻。`start <= end` を呼び出し側が保証する (validation は持たない)
    pub end: Duration,
    /// 文字起こしテキスト等
    pub text: String,
    /// 言語コード ("ja" 等)。検出失敗時や言語推定なしの場合は None
    pub language: Option<LanguageCode>,
    /// 発話がない確率 (0.0 - 1.0、Whisper 由来の幻覚指標)。指標を提供しない生成元では None
    pub no_speech_prob: Option<f32>,
    /// 平均 log probability (信頼度目安、Whisper 由来)。指標を提供しない生成元では None
    pub avg_logprob: Option<f32>,
}
