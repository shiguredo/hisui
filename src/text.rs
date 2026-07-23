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

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for LanguageCode {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for LanguageCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl nojson::DisplayJson for LanguageCode {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.string(self.as_str())
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
    /// 発話がない確率 (0.0 - 1.0、Whisper 由来の幻覚指標)。指標を提供しない生成元では None
    pub no_speech_prob: Option<f32>,
    /// 平均 log probability (信頼度目安、Whisper 由来)。指標を提供しない生成元では None
    pub avg_logprob: Option<f32>,
}

impl nojson::DisplayJson for TextFrame {
    /// 先頭に `type = "transcript"` を出し、`--emit-exit-metrics` の
    /// `type = "metrics"` 行と JSON LINE stream 上で区別できるようにする。
    /// `Option` フィールドは `Some` のときのみ member を書く (nojson の `Option<T>: DisplayJson`
    /// を素通しすると `None` が `null` として出るが、JSON LINE 出力ではキーごと省略したい)。
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "transcript")?;
            f.member("start", self.start.as_secs_f64())?;
            f.member("end", self.end.as_secs_f64())?;
            f.member("text", &self.text)?;
            if let Some(v) = self.no_speech_prob {
                f.member("no_speech_prob", v)?;
            }
            if let Some(v) = self.avg_logprob {
                f.member("avg_logprob", v)?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// new に &str / String のどちらを渡しても as_str で同じ内容が取り出せる。
    #[test]
    fn new_accepts_str_and_string_and_as_str_returns_it() {
        let from_str = LanguageCode::new("ja");
        assert_eq!(from_str.as_str(), "ja");

        let from_string = LanguageCode::new(String::from("en"));
        assert_eq!(from_string.as_str(), "en");
    }

    /// Display は内部文字列をそのまま出力する (前後に装飾を付けない)。
    #[test]
    fn display_writes_inner_string_verbatim() {
        let code = LanguageCode::new("haw");
        assert_eq!(format!("{code}"), "haw");
    }

    /// PartialEq は内部文字列で比較する (同一文字列で真、異なる文字列で偽)。
    #[test]
    fn partial_eq_compares_inner_string() {
        assert_eq!(LanguageCode::new("ja"), LanguageCode::new("ja"));
        assert_ne!(LanguageCode::new("ja"), LanguageCode::new("en"));
    }

    /// LanguageCode の DisplayJson は内部文字列を JSON string として出力する。
    #[test]
    fn display_json_writes_language_code_as_json_string() {
        let code = LanguageCode::new("ja");
        let json = nojson::json(|f| f.value(&code)).to_string();
        assert_eq!(json, "\"ja\"");
    }

    /// TextFrame の DisplayJson は全フィールド Some のときに全キーを含む JSON object を返す。
    #[test]
    fn display_json_writes_all_members_when_options_are_some() {
        let frame = TextFrame {
            start: Duration::from_millis(500),
            end: Duration::from_millis(2500),
            text: "hello".to_owned(),
            no_speech_prob: Some(0.05),
            avg_logprob: Some(-0.3),
        };
        let json = nojson::json(|f| {
            f.set_indent_size(0);
            f.value(&frame)
        })
        .to_string();
        assert!(json.contains("\"type\":\"transcript\""), "type: {json}");
        assert!(json.contains("\"start\":0.5"), "start: {json}");
        assert!(json.contains("\"end\":2.5"), "end: {json}");
        assert!(json.contains("\"text\":\"hello\""), "text: {json}");
        assert!(
            json.contains("\"no_speech_prob\":0.05"),
            "no_speech_prob: {json}"
        );
        assert!(json.contains("\"avg_logprob\":-0.3"), "avg_logprob: {json}");
    }

    /// TextFrame の DisplayJson は Option 一部だけが Some のとき、Some キーは残し
    /// None キーだけを省略する (Option フィールドが独立に判定されていることを担保する)。
    #[test]
    fn display_json_omits_only_none_keys_when_options_are_mixed() {
        let frame = TextFrame {
            start: Duration::from_millis(200),
            end: Duration::from_millis(1200),
            text: "mixed".to_owned(),
            no_speech_prob: None,
            avg_logprob: Some(-0.5),
        };
        let json = nojson::json(|f| {
            f.set_indent_size(0);
            f.value(&frame)
        })
        .to_string();
        assert!(
            !json.contains("no_speech_prob"),
            "no_speech_prob 省略: {json}"
        );
        assert!(
            json.contains("\"avg_logprob\":-0.5"),
            "avg_logprob 残る: {json}"
        );
        assert!(!json.contains("null"), "null 非出現: {json}");
    }

    /// TextFrame の DisplayJson は key 順序が type / start / end / text /
    /// no_speech_prob / avg_logprob 固定 (JSON LINE スキーマ契約の回帰保護)。
    #[test]
    fn display_json_preserves_key_order() {
        let frame = TextFrame {
            start: Duration::from_millis(500),
            end: Duration::from_millis(2500),
            text: "hello".to_owned(),
            no_speech_prob: Some(0.05),
            avg_logprob: Some(-0.3),
        };
        let json = nojson::json(|f| {
            f.set_indent_size(0);
            f.value(&frame)
        })
        .to_string();
        let idx = |key: &str| json.find(key).expect(key);
        assert!(idx("\"type\"") < idx("\"start\""), "type < start: {json}");
        assert!(idx("\"start\"") < idx("\"end\""), "start < end: {json}");
        assert!(idx("\"end\"") < idx("\"text\""), "end < text: {json}");
        assert!(
            idx("\"text\"") < idx("\"no_speech_prob\""),
            "text < no_speech_prob: {json}"
        );
        assert!(
            idx("\"no_speech_prob\"") < idx("\"avg_logprob\""),
            "no_speech_prob < avg_logprob: {json}"
        );
    }

    /// TextFrame の DisplayJson は Option が None のとき、そのキーごと省略する
    /// (null を出さない = JSON LINE スキーマの要件)。
    #[test]
    fn display_json_omits_none_options() {
        let frame = TextFrame {
            start: Duration::ZERO,
            end: Duration::from_millis(100),
            text: String::new(),
            no_speech_prob: None,
            avg_logprob: None,
        };
        let json = nojson::json(|f| {
            f.set_indent_size(0);
            f.value(&frame)
        })
        .to_string();
        assert!(
            !json.contains("no_speech_prob"),
            "no_speech_prob 省略: {json}"
        );
        assert!(!json.contains("avg_logprob"), "avg_logprob 省略: {json}");
        assert!(!json.contains("null"), "null 非出現: {json}");
    }
}
