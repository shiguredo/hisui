//! Whisper 多言語モデルの言語検出ヘルパー。

use candle_core::{D, IndexOp, Tensor};
use candle_nn::ops::softmax;
use candle_transformers::models::whisper::{self as m, Config};
use tokenizers::Tokenizer;

use super::decode::{WhisperModel, token_id};
use crate::Result;

const LANGUAGES: [(&str, &str); 99] = [
    ("en", "english"),
    ("zh", "chinese"),
    ("de", "german"),
    ("es", "spanish"),
    ("ru", "russian"),
    ("ko", "korean"),
    ("fr", "french"),
    ("ja", "japanese"),
    ("pt", "portuguese"),
    ("tr", "turkish"),
    ("pl", "polish"),
    ("ca", "catalan"),
    ("nl", "dutch"),
    ("ar", "arabic"),
    ("sv", "swedish"),
    ("it", "italian"),
    ("id", "indonesian"),
    ("hi", "hindi"),
    ("fi", "finnish"),
    ("vi", "vietnamese"),
    ("he", "hebrew"),
    ("uk", "ukrainian"),
    ("el", "greek"),
    ("ms", "malay"),
    ("cs", "czech"),
    ("ro", "romanian"),
    ("da", "danish"),
    ("hu", "hungarian"),
    ("ta", "tamil"),
    ("no", "norwegian"),
    ("th", "thai"),
    ("ur", "urdu"),
    ("hr", "croatian"),
    ("bg", "bulgarian"),
    ("lt", "lithuanian"),
    ("la", "latin"),
    ("mi", "maori"),
    ("ml", "malayalam"),
    ("cy", "welsh"),
    ("sk", "slovak"),
    ("te", "telugu"),
    ("fa", "persian"),
    ("lv", "latvian"),
    ("bn", "bengali"),
    ("sr", "serbian"),
    ("az", "azerbaijani"),
    ("sl", "slovenian"),
    ("kn", "kannada"),
    ("et", "estonian"),
    ("mk", "macedonian"),
    ("br", "breton"),
    ("eu", "basque"),
    ("is", "icelandic"),
    ("hy", "armenian"),
    ("ne", "nepali"),
    ("mn", "mongolian"),
    ("bs", "bosnian"),
    ("kk", "kazakh"),
    ("sq", "albanian"),
    ("sw", "swahili"),
    ("gl", "galician"),
    ("mr", "marathi"),
    ("pa", "punjabi"),
    ("si", "sinhala"),
    ("km", "khmer"),
    ("sn", "shona"),
    ("yo", "yoruba"),
    ("so", "somali"),
    ("af", "afrikaans"),
    ("oc", "occitan"),
    ("ka", "georgian"),
    ("be", "belarusian"),
    ("tg", "tajik"),
    ("sd", "sindhi"),
    ("gu", "gujarati"),
    ("am", "amharic"),
    ("yi", "yiddish"),
    ("lo", "lao"),
    ("uz", "uzbek"),
    ("fo", "faroese"),
    ("ht", "haitian creole"),
    ("ps", "pashto"),
    ("tk", "turkmen"),
    ("nn", "nynorsk"),
    ("mt", "maltese"),
    ("sa", "sanskrit"),
    ("lb", "luxembourgish"),
    ("my", "myanmar"),
    ("bo", "tibetan"),
    ("tl", "tagalog"),
    ("mg", "malagasy"),
    ("as", "assamese"),
    ("tt", "tatar"),
    ("haw", "hawaiian"),
    ("ln", "lingala"),
    ("ha", "hausa"),
    ("ba", "bashkir"),
    ("jw", "javanese"),
    ("su", "sundanese"),
];

/// 検出された言語。
pub struct DetectedLanguage {
    pub code: String,
    pub token_id: u32,
}

/// 多言語 Whisper の語彙数の下限。多言語モデル (tiny/base/small/medium/large-v1/v2) は 51865、
/// large-v3 系は 51866 で、英語専用 (.en) モデルは 51864。言語トークンは多言語語彙にのみ存在する
/// ため、この値以上を多言語とみなす。
const MULTILINGUAL_VOCAB_SIZE: usize = 51865;

/// 多言語モデルなら true。
pub fn is_multilingual_config(config: &Config) -> bool {
    config.vocab_size >= MULTILINGUAL_VOCAB_SIZE
}

/// ISO 639-1 言語コードを Whisper の言語トークン ID に変換する。
pub fn language_token_from_code(tokenizer: &Tokenizer, code: &str) -> Result<u32> {
    let code = code.trim();
    let token = if code.starts_with("<|") {
        code.to_owned()
    } else {
        format!("<|{code}|>")
    };
    token_id(tokenizer, &token)
}

/// 多言語モデル向けに言語を推定する。
pub fn detect_language(
    model: &mut WhisperModel,
    tokenizer: &Tokenizer,
    mel: &Tensor,
) -> Result<DetectedLanguage> {
    let device = mel.device();
    let language_specs: Vec<(u32, &str, &str)> = LANGUAGES
        .iter()
        .map(|(code, name)| {
            let token_id = language_token_from_code(tokenizer, code)?;
            Ok((token_id, *code, *name))
        })
        .collect::<Result<_>>()?;
    let language_token_ids: Vec<u32> = language_specs.iter().map(|(id, _, _)| *id).collect();

    let sot_token = token_id(tokenizer, m::SOT_TOKEN)?;
    let audio_features = model
        .encoder_forward(mel, true)
        .map_err(|e| crate::Error::new(format!("encoder for language detect: {e}")))?;
    let tokens = Tensor::new(&[[sot_token]], device)
        .map_err(|e| crate::Error::new(format!("sot tensor: {e}")))?;
    let language_token_ids = Tensor::new(language_token_ids.as_slice(), device)
        .map_err(|e| crate::Error::new(format!("language ids tensor: {e}")))?;
    let ys = model
        .decoder_forward(&tokens, &audio_features, true)
        .map_err(|e| crate::Error::new(format!("decoder for language detect: {e}")))?;
    let logits = model
        .decoder_final_linear(&ys.i(..1).map_err(candle_err)?)
        .map_err(candle_err)?
        .i(0)
        .map_err(candle_err)?
        .i(0)
        .map_err(candle_err)?;
    let logits = logits
        .index_select(&language_token_ids, 0)
        .map_err(candle_err)?;
    let probs = softmax(&logits, D::Minus1).map_err(candle_err)?;
    let probs = probs.to_vec1::<f32>().map_err(candle_err)?;

    let (best_index, best_prob) = probs
        .iter()
        .copied()
        .enumerate()
        .max_by(|(_, left), (_, right)| left.total_cmp(right))
        .expect("language candidates must not be empty");
    let (token_id, code, name) = language_specs[best_index];
    tracing::info!("detected language: {name} ({code}), prob={best_prob:.4}");

    Ok(DetectedLanguage {
        code: code.to_owned(),
        token_id,
    })
}

fn candle_err(e: candle_core::Error) -> crate::Error {
    crate::Error::new(format!("candle error: {e}"))
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
}
