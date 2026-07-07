//! Whisper モデルの重みロードと greedy decode ループ。
//!
//! `WhisperModel` は candle の `Whisper` (encoder / decoder / KV cache) の薄いラッパーで、
//! `WhisperDecoder` が SOT / 言語 / task トークンの組み立てと greedy サンプリングを担う。
//! タスクは文字起こし (transcribe) 固定で、タイムスタンプトークンは出力しない
//! (`TextFrame` の時刻は VAD 由来で埋めるため)。

use candle_core::{D, IndexOp, Tensor};
use candle_nn::{VarBuilder, ops::softmax};
use candle_transformers::models::whisper::{self as m, Config, model::Whisper};
use tokenizers::Tokenizer;

use crate::Result;

/// Whisper モデル本体 (重み + 設定)。
///
/// candle の `Whisper` は KV cache を内部に持つ mutable な状態機のため、
/// 複数スレッドで共有せず、利用者 (worker) ごとに個別ロードする。
pub struct WhisperModel {
    inner: Whisper,
    config: Config,
}

impl WhisperModel {
    /// safetensors 形式の重みファイルをロードする。
    pub fn load(
        weights_path: &std::path::Path,
        config: Config,
        device: &candle_core::Device,
    ) -> Result<Self> {
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], m::DTYPE, device)
                .map_err(|e| crate::Error::new(format!("failed to mmap whisper weights: {e}")))?
        };
        let inner = Whisper::load(&vb, config.clone())
            .map_err(|e| crate::Error::new(format!("failed to load whisper model: {e}")))?;
        Ok(Self { inner, config })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn encoder_forward(&mut self, x: &Tensor, flush: bool) -> candle_core::Result<Tensor> {
        self.inner.encoder.forward(x, flush)
    }

    pub fn decoder_forward(
        &mut self,
        x: &Tensor,
        xa: &Tensor,
        flush: bool,
    ) -> candle_core::Result<Tensor> {
        self.inner.decoder.forward(x, xa, flush)
    }

    pub fn decoder_final_linear(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        self.inner.decoder.final_linear(x)
    }

    pub fn reset_kv_cache(&mut self) {
        self.inner.reset_kv_cache();
    }
}

/// 1 チャンクぶんの decode 結果。
///
/// candle 内部の計算値 (f64) をそのまま保持する。`TextFrame` 向けの f32 変換は上位層で行う。
pub struct WhisperDecodedChunk {
    pub text: String,
    pub avg_logprob: f64,
    pub no_speech_prob: f64,
}

/// greedy decode の実行機。トークン組み立てと抑制トークンの適用を担う。
pub struct WhisperDecoder {
    model: WhisperModel,
    tokenizer: Tokenizer,
    suppress_tokens: Tensor,
    sot_token: u32,
    transcribe_token: u32,
    eot_token: u32,
    no_speech_token: u32,
    no_timestamps_token: u32,
    language_token: Option<u32>,
}

impl WhisperDecoder {
    pub fn new(
        model: WhisperModel,
        tokenizer: Tokenizer,
        device: &candle_core::Device,
    ) -> Result<Self> {
        let suppress_tokens: Vec<f32> = (0..model.config().vocab_size as u32)
            .map(|i| {
                if model.config().suppress_tokens.contains(&i) {
                    f32::NEG_INFINITY
                } else {
                    0.0
                }
            })
            .collect();
        let suppress_tokens = Tensor::new(suppress_tokens.as_slice(), device)
            .map_err(|e| crate::Error::new(format!("suppress_tokens tensor: {e}")))?;

        let sot_token = token_id(&tokenizer, m::SOT_TOKEN)?;
        let transcribe_token = token_id(&tokenizer, m::TRANSCRIBE_TOKEN)?;
        let eot_token = token_id(&tokenizer, m::EOT_TOKEN)?;
        let no_timestamps_token = token_id(&tokenizer, m::NO_TIMESTAMPS_TOKEN)?;
        let no_speech_token = m::NO_SPEECH_TOKENS
            .iter()
            .find_map(|token| token_id(&tokenizer, token).ok())
            .ok_or_else(|| crate::Error::new("unable to find any non-speech token"))?;

        Ok(Self {
            model,
            tokenizer,
            suppress_tokens,
            sot_token,
            transcribe_token,
            eot_token,
            no_speech_token,
            no_timestamps_token,
            language_token: None,
        })
    }

    pub fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
    }

    /// リクエスト単位で言語トークンを設定する。None は言語トークンなし (非多言語モデル)。
    pub fn set_language_token(&mut self, language_token: Option<u32>) {
        self.language_token = language_token;
    }

    pub fn reset_kv_cache(&mut self) {
        self.model.reset_kv_cache();
    }

    /// 1 チャンク (mel) を decode する。
    ///
    /// no_speech ガード: `no_speech_prob > 0.6` かつ `avg_logprob < -1.0` の場合は
    /// 幻覚の可能性が高いため text を空にして返す (品質指標はそのまま返す)。
    pub fn decode_chunk(&mut self, mel: &Tensor) -> Result<WhisperDecodedChunk> {
        let dr = self.decode_segment(mel)?;
        if dr.no_speech_prob > m::NO_SPEECH_THRESHOLD && dr.avg_logprob < m::LOGPROB_THRESHOLD {
            return Ok(WhisperDecodedChunk {
                text: String::new(),
                avg_logprob: dr.avg_logprob,
                no_speech_prob: dr.no_speech_prob,
            });
        }
        Ok(dr)
    }

    fn decode_segment(&mut self, mel: &Tensor) -> Result<WhisperDecodedChunk> {
        let audio_features = self
            .model
            .encoder_forward(mel, true)
            .map_err(|e| crate::Error::new(format!("whisper encoder: {e}")))?;

        let sample_len = self.model.config().max_target_positions / 2;
        let mut sum_logprob = 0f64;
        let mut no_speech_prob = f64::NAN;
        let mut tokens = vec![self.sot_token];
        if let Some(language_token) = self.language_token {
            tokens.push(language_token);
        }
        tokens.push(self.transcribe_token);
        tokens.push(self.no_timestamps_token);

        for i in 0..sample_len {
            let tokens_t = Tensor::new(tokens.as_slice(), mel.device())
                .map_err(|e| crate::Error::new(format!("tokens tensor: {e}")))?
                .unsqueeze(0)
                .map_err(|e| crate::Error::new(format!("tokens unsqueeze: {e}")))?;

            let ys = self
                .model
                .decoder_forward(&tokens_t, &audio_features, i == 0)
                .map_err(|e| crate::Error::new(format!("whisper decoder: {e}")))?;

            // 初回 step の SOT 位置の logits から no_speech 確率を取る
            if i == 0 {
                let logits = self
                    .model
                    .decoder_final_linear(&ys.i(..1).map_err(candle_err)?)
                    .map_err(candle_err)?
                    .i(0)
                    .map_err(candle_err)?
                    .i(0)
                    .map_err(candle_err)?;
                no_speech_prob = softmax(&logits, 0)
                    .map_err(candle_err)?
                    .i(self.no_speech_token as usize)
                    .map_err(candle_err)?
                    .to_scalar::<f32>()
                    .map_err(candle_err)? as f64;
            }

            let (_, seq_len, _) = ys.dims3().map_err(candle_err)?;
            let logits = self
                .model
                .decoder_final_linear(&ys.i((..1, seq_len - 1..)).map_err(candle_err)?)
                .map_err(candle_err)?
                .i(0)
                .map_err(candle_err)?
                .i(0)
                .map_err(candle_err)?;
            let logits = logits
                .broadcast_add(&self.suppress_tokens)
                .map_err(candle_err)?;

            // greedy サンプリング (温度 0 固定)
            let logits_v: Vec<f32> = logits.to_vec1().map_err(candle_err)?;
            let next_token = logits_v
                .iter()
                .enumerate()
                .max_by(|(_, u), (_, v)| u.total_cmp(v))
                .map(|(i, _)| i as u32)
                .expect("logits must not be empty");

            tokens.push(next_token);
            let prob = softmax(&logits, D::Minus1)
                .map_err(candle_err)?
                .i(next_token as usize)
                .map_err(candle_err)?
                .to_scalar::<f32>()
                .map_err(candle_err)? as f64;

            if next_token == self.eot_token
                || tokens.len() > self.model.config().max_target_positions
            {
                break;
            }
            sum_logprob += prob.ln();
        }

        let text = self
            .tokenizer
            .decode(&tokens, true)
            .map_err(|e| crate::Error::new(format!("tokenizer decode: {e}")))?;
        let avg_logprob = sum_logprob / tokens.len().max(1) as f64;

        Ok(WhisperDecodedChunk {
            text,
            avg_logprob,
            no_speech_prob,
        })
    }
}

/// トークン文字列をトークン ID に変換する。
pub fn token_id(tokenizer: &Tokenizer, token: &str) -> Result<u32> {
    match tokenizer.token_to_id(token) {
        None => Err(crate::Error::new(format!("no token-id for {token}"))),
        Some(id) => Ok(id),
    }
}

fn candle_err(e: candle_core::Error) -> crate::Error {
    crate::Error::new(format!("candle error: {e}"))
}
