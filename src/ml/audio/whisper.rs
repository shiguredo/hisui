use std::path::{Path, PathBuf};

use candle_core::Tensor;
use candle_transformers::models::whisper::{self as m, Config, audio};
use tokenizers::Tokenizer;

use super::config::load_whisper_config;
use super::decode::{self, Decoder, Task, WhisperModel};
use super::multilingual;
use crate::Result;

pub use m::SAMPLE_RATE;

pub struct WhisperPipeline {
    decoder: Decoder,
    config: Config,
    mel_filters: Vec<f32>,
    candle_device: candle_core::Device,
    language_detected: bool,
}

impl WhisperPipeline {
    /// `model_dir` には `config.json`, `tokenizer.json`, `model.safetensors` が必要
    pub fn load(
        model_dir: &Path,
        candle_device: candle_core::Device,
        language: Option<String>,
        task: Task,
    ) -> Result<Self> {
        let config_path = model_dir.join("config.json");
        let tokenizer_path = model_dir.join("tokenizer.json");
        let weights_path = model_dir.join("model.safetensors");

        let config = load_whisper_config(&config_path)?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| crate::Error::new(format!("load tokenizer: {e}")))?;

        let language_detected = language.is_some();
        let language_token = match language {
            Some(lang) => Some(language_token_from_name_with_tokenizer(&tokenizer, &lang)?),
            None => None,
        };

        let mel_filters = load_mel_filters(config.num_mel_bins)?;

        let model = WhisperModel::load(&weights_path, config.clone(), &candle_device)?;
        let decoder = Decoder::new(model, tokenizer, &candle_device, language_token, task)?;

        Ok(Self {
            decoder,
            config,
            mel_filters,
            candle_device,
            language_detected,
        })
    }

    pub fn transcribe_pcm16k(&mut self, pcm: &[f32]) -> Result<String> {
        let mel = audio::pcm_to_mel(&self.config, pcm, &self.mel_filters);
        let mel_len = mel.len();
        if mel_len == 0 {
            return Ok(String::new());
        }
        let mel = Tensor::from_vec(
            mel,
            (
                1,
                self.config.num_mel_bins,
                mel_len / self.config.num_mel_bins,
            ),
            &self.candle_device,
        )
        .map_err(|e| crate::Error::new(format!("mel tensor: {e}")))?;

        // 多言語モデルの言語推定は初回チャンクで 1 回だけ行う
        if !self.language_detected && multilingual::is_multilingual_config(&self.config) {
            let tokenizer = self.decoder.tokenizer.clone();
            let language_token = {
                let model = self.decoder.model_mut();
                multilingual::detect_language(model, &tokenizer, &mel)?
            };
            self.decoder.set_language_token(Some(language_token));
            self.language_detected = true;
            self.decoder.reset_kv_cache();
        }

        let result = self.decoder.decode_chunk(&mel)?;
        self.decoder.reset_kv_cache();
        Ok(result.text.trim().to_owned())
    }
}

fn load_mel_filters(num_mel_bins: usize) -> Result<Vec<f32>> {
    let mel_bytes = match num_mel_bins {
        80 => include_bytes!("melfilters.bytes").as_slice(),
        128 => {
            return Err(crate::Error::new(
                "128 mel bins is not bundled in hisui; use 80-bin whisper models (tiny/base/small)",
            ));
        }
        nmel => {
            return Err(crate::Error::new(format!(
                "unexpected whisper num_mel_bins: {nmel}"
            )));
        }
    };
    if !mel_bytes.len().is_multiple_of(4) {
        return Err(crate::Error::new(format!(
            "invalid melfilters.bytes size: {}",
            mel_bytes.len()
        )));
    }
    let mel_filters = mel_bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    Ok(mel_filters)
}

/// `language` 名（例: `en`, `ja`）をトークン ID に変換する
fn language_token_from_name_with_tokenizer(tokenizer: &Tokenizer, language: &str) -> Result<u32> {
    let name = language.trim();
    let token = if name.starts_with("<|") {
        name.to_owned()
    } else {
        format!("<|{name}|>")
    };
    decode::token_id(tokenizer, &token)
}

/// Hugging Face の Whisper モデルディレクトリを検証する
pub fn validate_model_dir(model_dir: &Path) -> Result<PathBuf> {
    for name in ["config.json", "tokenizer.json", "model.safetensors"] {
        let p = model_dir.join(name);
        if !p.is_file() {
            return Err(crate::Error::new(format!(
                "missing {} in model directory {}",
                name,
                model_dir.display()
            )));
        }
    }
    Ok(model_dir.to_path_buf())
}
