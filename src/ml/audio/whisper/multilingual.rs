//! Whisper の多言語モデル判定と、指定言語コードから言語トークンへの解決ヘルパー。

use candle_transformers::models::whisper::Config;
use tokenizers::Tokenizer;

use super::decode::{TokenId, token_id};
use crate::Result;
use crate::text::LanguageCode;

/// 多言語 Whisper の語彙数の下限。多言語モデル (tiny/base/small/medium/large-v1/v2) は 51865、
/// large-v3 系は 51866 で、英語専用 (.en) モデルは 51864。言語トークンは多言語語彙にのみ存在する
/// ため、この値以上を多言語とみなす。
const MULTILINGUAL_VOCAB_SIZE: usize = 51865;

/// 多言語モデルなら true。
pub(super) fn is_multilingual_config(config: &Config) -> bool {
    config.vocab_size >= MULTILINGUAL_VOCAB_SIZE
}

/// ISO 639-1 言語コードを Whisper の言語トークン ID に変換する。
pub(super) fn language_token_from_code(
    tokenizer: &Tokenizer,
    code: &LanguageCode,
) -> Result<TokenId> {
    let token = build_language_token_string(code);
    token_id(tokenizer, &token)
}

/// 言語コードから Whisper 用トークン文字列 (`<|xx|>` 形式) を組み立てる。
///
/// 入力 code の前後空白は trim する。 既に `<|` で始まっていれば trim 済みの文字列を
/// そのまま返し、そうでなければ `<|{code}|>` で包む (二重包みを避ける)。
/// トークンが tokenizer に存在するかは呼び出し側で検証する。
fn build_language_token_string(code: &LanguageCode) -> String {
    let code = code.as_str().trim();
    if code.starts_with("<|") {
        code.to_owned()
    } else {
        format!("<|{code}|>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// vocab_size 以外を whisper-tiny 相当の固定値で埋めた Config を組み立てる。
    /// 多言語判定は vocab_size のみに依存するため、他フィールドの値は結果に影響しない。
    fn config_with_vocab_size(vocab_size: usize) -> Config {
        Config {
            num_mel_bins: 80,
            max_source_positions: 1500,
            d_model: 384,
            encoder_attention_heads: 6,
            encoder_layers: 4,
            vocab_size,
            max_target_positions: 448,
            decoder_attention_heads: 6,
            decoder_layers: 4,
            suppress_tokens: Vec::new(),
        }
    }

    /// 英語専用 (.en) モデルの vocab_size (51864) は多言語と判定しない。
    #[test]
    fn english_only_config_is_not_multilingual() {
        assert!(
            !is_multilingual_config(&config_with_vocab_size(51864)),
            "英語専用モデルは非多言語と判定されるべき"
        );
    }

    /// 多言語モデル (tiny/base/small/medium/large-v1/v2) の vocab_size (51865) は多言語と判定する。
    #[test]
    fn multilingual_config_is_multilingual() {
        assert!(
            is_multilingual_config(&config_with_vocab_size(51865)),
            "多言語モデルは多言語と判定されるべき"
        );
    }

    /// large-v3 系の vocab_size (51866) も多言語と判定する。
    #[test]
    fn large_v3_config_is_multilingual() {
        assert!(
            is_multilingual_config(&config_with_vocab_size(51866)),
            "large-v3 系も多言語と判定されるべき"
        );
    }

    /// 素の言語コードは `<|` と `|>` で包まれる。
    #[test]
    fn build_language_token_string_wraps_bare_code() {
        let token = build_language_token_string(&LanguageCode::new("ja"));
        assert_eq!(token, "<|ja|>");
    }

    /// 既に `<|xx|>` 形式で渡された場合は二重包みせずそのまま返す。
    #[test]
    fn build_language_token_string_passes_through_wrapped_code() {
        let token = build_language_token_string(&LanguageCode::new("<|haw|>"));
        assert_eq!(token, "<|haw|>");
    }

    /// 前後の空白は trim され、素の場合のみ `<|` と `|>` で包まれる。
    #[test]
    fn build_language_token_string_trims_whitespace_before_wrapping() {
        let token = build_language_token_string(&LanguageCode::new("  en  "));
        assert_eq!(token, "<|en|>");
    }

    /// trim 後に `<|` で始まる形式もそのまま返す (前後空白ありでも二重包みしない)。
    #[test]
    fn build_language_token_string_trims_whitespace_of_wrapped_code() {
        let token = build_language_token_string(&LanguageCode::new("  <|zh|>  "));
        assert_eq!(token, "<|zh|>");
    }
}
