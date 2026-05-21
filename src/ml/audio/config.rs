use std::path::Path;

use candle_transformers::models::whisper::Config;

use crate::Result;

/// Hugging Face `config.json` から読み取る Whisper 設定
struct WhisperConfigFile {
    num_mel_bins: usize,
    max_source_positions: usize,
    d_model: usize,
    encoder_attention_heads: usize,
    encoder_layers: usize,
    vocab_size: usize,
    max_target_positions: usize,
    decoder_attention_heads: usize,
    decoder_layers: usize,
    suppress_tokens: Vec<u32>,
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for WhisperConfigFile {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            num_mel_bins: parse_required_usize(value, "num_mel_bins")?,
            max_source_positions: parse_required_usize(value, "max_source_positions")?,
            d_model: parse_required_usize(value, "d_model")?,
            encoder_attention_heads: parse_required_usize(value, "encoder_attention_heads")?,
            encoder_layers: parse_required_usize(value, "encoder_layers")?,
            vocab_size: parse_required_usize(value, "vocab_size")?,
            max_target_positions: parse_required_usize(value, "max_target_positions")?,
            decoder_attention_heads: parse_required_usize(value, "decoder_attention_heads")?,
            decoder_layers: parse_required_usize(value, "decoder_layers")?,
            suppress_tokens: parse_optional_u32_array(value, "suppress_tokens")?,
        })
    }
}

impl From<WhisperConfigFile> for Config {
    fn from(file: WhisperConfigFile) -> Self {
        Self {
            num_mel_bins: file.num_mel_bins,
            max_source_positions: file.max_source_positions,
            d_model: file.d_model,
            encoder_attention_heads: file.encoder_attention_heads,
            encoder_layers: file.encoder_layers,
            vocab_size: file.vocab_size,
            max_target_positions: file.max_target_positions,
            decoder_attention_heads: file.decoder_attention_heads,
            decoder_layers: file.decoder_layers,
            suppress_tokens: file.suppress_tokens,
        }
    }
}

fn parse_required_usize(
    value: nojson::RawJsonValue<'_, '_>,
    member: &str,
) -> std::result::Result<usize, nojson::JsonParseError> {
    let member_value = value.to_member(member)?.required()?;
    let n: i64 = member_value.try_into()?;
    usize::try_from(n)
        .map_err(|_| member_value.invalid(format!("{member} must be a non-negative integer")))
}

fn parse_optional_u32_array(
    value: nojson::RawJsonValue<'_, '_>,
    member: &str,
) -> std::result::Result<Vec<u32>, nojson::JsonParseError> {
    let tokens: Option<Vec<u32>> = value.to_member(member)?.try_into()?;
    Ok(tokens.unwrap_or_default())
}

/// `config.json` を nojson で読み込み、candle の `Config` に変換する
pub fn load_whisper_config(path: &Path) -> Result<Config> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| crate::Error::new(format!("read {}: {e}", path.display())))?;
    let json = nojson::RawJson::parse(&text)?;
    let file = WhisperConfigFile::try_from(json.value())
        .map_err(|e| crate::Error::new(format!("parse {}: {e}", path.display())))?;
    Ok(file.into())
}
