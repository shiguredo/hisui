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

/// 多言語モデル向けに言語トークンを推定する
pub fn detect_language(
    model: &mut WhisperModel,
    tokenizer: &Tokenizer,
    mel: &Tensor,
) -> Result<u32> {
    let (_bsize, _, seq_len) = mel
        .dims3()
        .map_err(|e| crate::Error::new(format!("mel dims: {e}")))?;
    let mel = mel
        .narrow(
            2,
            0,
            usize::min(seq_len, model.config().max_source_positions),
        )
        .map_err(|e| crate::Error::new(format!("mel narrow: {e}")))?;
    let device = mel.device();
    let language_token_ids: Vec<u32> = LANGUAGES
        .iter()
        .map(|(t, _)| token_id(tokenizer, &format!("<|{t}|>")))
        .collect::<Result<Vec<_>>>()?;
    let sot_token = token_id(tokenizer, m::SOT_TOKEN)?;
    let audio_features = model
        .encoder_forward(&mel, true)
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
    let mut probs = LANGUAGES.iter().zip(probs.iter()).collect::<Vec<_>>();
    probs.sort_by(|(_, p1), (_, p2)| p2.total_cmp(p1));
    if let Some(((code, name), p)) = probs.first() {
        tracing::info!("detected language: {name} ({code}), prob={p:.4}");
    }
    let language = token_id(tokenizer, &format!("<|{}|>", probs[0].0.0))?;
    Ok(language)
}

fn candle_err(e: candle_core::Error) -> crate::Error {
    crate::Error::new(format!("candle error: {e}"))
}

pub fn is_multilingual_config(config: &Config) -> bool {
    config.vocab_size > 5000
}
