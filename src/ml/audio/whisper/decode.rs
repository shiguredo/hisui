//! Whisper モデルの重みロードと greedy decode ループ。
//!
//! `WhisperDecoder` は candle の `Whisper` (encoder / decoder / KV cache) を保持し、
//! SOT / 言語 / task トークンの組み立てと greedy サンプリングを担う。タスクは文字起こし
//! (transcribe) 固定で、タイムスタンプトークンは出力しない (`TextFrame` の時刻は VAD 由来で埋めるため)。
//!
//! candle の `Whisper` は KV cache を内部に持つ mutable な状態機のため、複数スレッドで共有せず、
//! 利用者 (worker) ごとに個別ロードする。

use std::path::Path;

use candle_core::{D, IndexOp, Tensor};
use candle_nn::{VarBuilder, ops::softmax};
use candle_transformers::models::whisper::{
    Config, DTYPE, EOT_TOKEN, LOGPROB_THRESHOLD, NO_SPEECH_THRESHOLD, NO_SPEECH_TOKENS,
    NO_TIMESTAMPS_TOKEN, SOT_TOKEN, TRANSCRIBE_TOKEN, model::Whisper,
};
use tokenizers::Tokenizer;

use crate::Result;
use crate::probability::{LogProbability, Probability};

/// 1 チャンクぶんの decode 結果。
///
/// 品質指標は candle 内部と同じ f64 精度の `LogProbability` / `Probability` で保持する。
/// `TextFrame` 向けの f32 変換は上位層で行う。
pub struct WhisperDecodedChunk {
    pub text: String,
    pub avg_logprob: LogProbability,
    pub no_speech_prob: Probability,
}

impl WhisperDecodedChunk {
    /// 「発話がない (hallucination の可能性が高い)」と判定できるかを返す。
    ///
    /// candle の閾値 `NO_SPEECH_THRESHOLD` (= 0.6) を `no_speech_prob` が上回り、かつ
    /// `LOGPROB_THRESHOLD` (= -1.0) を `avg_logprob` が下回ったときに真。閾値を独自に
    /// 調整したい場合は `no_speech_prob` / `avg_logprob` を直接見て判定する。
    pub fn is_likely_no_speech(&self) -> bool {
        self.no_speech_prob.get() > NO_SPEECH_THRESHOLD
            && self.avg_logprob.get() < LOGPROB_THRESHOLD
    }
}

/// Whisper プロトコルで固定されている特殊トークンの一式。
///
/// リクエストごとに変わらないため、ロード時に一度だけ tokenizer から引いて保持する。
/// 言語トークンは per-request に変わるため本構造体には含めず、`WhisperDecoder.language_token` で
/// 別に持つ。
struct ProtocolTokens {
    /// SOT (start-of-transcript)。decode 対象トークン列の先頭に積む。
    sot: u32,
    /// タスクを「文字起こし」に固定するトークン (`translate` ではなく `transcribe` を選ぶ)。
    transcribe: u32,
    /// EOT (end-of-transcript)。このトークンが出た時点で decode ループを終える。
    eot: u32,
    /// no_speech トークン。初回 step の logits からこのトークンの確率を取り出し、
    /// 「発話がない確率 (幻覚判定用)」として上位層に返す。
    no_speech: u32,
    /// タイムスタンプトークンの出力を抑止するトークン (時刻は VAD 由来で埋めるため不要)。
    no_timestamps: u32,
}

impl ProtocolTokens {
    /// tokenizer から Whisper の 5 個の固定特殊トークンを一括で引いて構築する。
    ///
    /// `no_speech` は `NO_SPEECH_TOKENS` の候補 (`<|nospeech|>` / `<|nocaptions|>` 等) のうち
    /// tokenizer に存在する最初のものを採る。1 つも見つからなければ Err。
    fn from_tokenizer(tokenizer: &Tokenizer) -> Result<Self> {
        let sot = token_id(tokenizer, SOT_TOKEN)?;
        let transcribe = token_id(tokenizer, TRANSCRIBE_TOKEN)?;
        let eot = token_id(tokenizer, EOT_TOKEN)?;
        let no_timestamps = token_id(tokenizer, NO_TIMESTAMPS_TOKEN)?;
        let no_speech = NO_SPEECH_TOKENS
            .iter()
            .find_map(|token| token_id(tokenizer, token).ok())
            .ok_or_else(|| crate::Error::new("unable to find any non-speech token"))?;
        Ok(Self {
            sot,
            transcribe,
            eot,
            no_speech,
            no_timestamps,
        })
    }
}

/// Whisper 重みと greedy decode ループを 1 worker 分の状態としてまとめた型。
///
/// 内部の `Whisper` は KV cache を持つため 1 スレッド専有。並列化は複数の `WhisperDecoder` を
/// 個別ロードして worker プールに配る方式で実現する (Silero の `new_instance` 型の共有はしない)。
pub struct WhisperDecoder {
    inner: Whisper,
    config: Config,
    tokenizer: Tokenizer,
    suppress_tokens: Tensor,
    /// Whisper プロトコルの固定特殊トークン (SOT / transcribe / EOT / no_speech / no_timestamps)。
    protocol_tokens: ProtocolTokens,
    /// 現リクエストで使う言語トークン。多言語モデルでは `Some`、非多言語モデルでは `None`。
    language_token: Option<u32>,
}

impl WhisperDecoder {
    /// Hugging Face の safetensors 形式 (拡張子 `.safetensors`) の重みと config・tokenizer から
    /// `WhisperDecoder` を組み立てる。
    pub fn load<P: AsRef<Path>>(
        weights_path: P,
        config: Config,
        tokenizer: Tokenizer,
        device: &candle_core::Device,
    ) -> Result<Self> {
        let weights_path = weights_path.as_ref();
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DTYPE, device).map_err(|e| {
                crate::Error::new(format!("failed to load whisper safetensors weights: {e}"))
            })?
        };
        let inner = Whisper::load(&vb, config.clone())
            .map_err(|e| crate::Error::new(format!("failed to load whisper model: {e}")))?;

        let suppress_tokens: Vec<f32> = (0..config.vocab_size as u32)
            .map(|i| {
                if config.suppress_tokens.contains(&i) {
                    f32::NEG_INFINITY
                } else {
                    0.0
                }
            })
            .collect();
        let suppress_tokens = Tensor::new(suppress_tokens.as_slice(), device)
            .map_err(|e| crate::Error::new(format!("suppress_tokens tensor: {e}")))?;

        let protocol_tokens = ProtocolTokens::from_tokenizer(&tokenizer)?;

        Ok(Self {
            inner,
            config,
            tokenizer,
            suppress_tokens,
            protocol_tokens,
            language_token: None,
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
    }

    /// リクエスト単位で言語トークンを設定する。None は言語トークンなし (非多言語モデル)。
    pub fn set_language_token(&mut self, language_token: Option<u32>) {
        self.language_token = language_token;
    }

    pub fn reset_kv_cache(&mut self) {
        self.inner.reset_kv_cache();
    }

    /// 1 チャンク (mel) を decode する。
    ///
    /// hallucination の可能性がある結果もそのまま返す (text を空にしない)。呼び出し側は
    /// `WhisperDecodedChunk::is_likely_no_speech` で判定し、必要に応じて破棄する。
    pub fn decode_chunk(&mut self, mel: &Tensor) -> Result<WhisperDecodedChunk> {
        let audio_features = self
            .inner
            .encoder
            .forward(mel, true)
            .map_err(|e| crate::Error::new(format!("whisper encoder: {e}")))?;

        let sample_len = self.config.max_target_positions / 2;
        let mut sum_logprob = 0f64;
        let mut no_speech_prob_raw: Option<f64> = None;
        let mut tokens = vec![self.protocol_tokens.sot];
        if let Some(language_token) = self.language_token {
            tokens.push(language_token);
        }
        tokens.push(self.protocol_tokens.transcribe);
        tokens.push(self.protocol_tokens.no_timestamps);

        for i in 0..sample_len {
            let tokens_t = Tensor::new(tokens.as_slice(), mel.device())
                .map_err(|e| crate::Error::new(format!("tokens tensor: {e}")))?
                .unsqueeze(0)
                .map_err(|e| crate::Error::new(format!("tokens unsqueeze: {e}")))?;

            let ys = self
                .inner
                .decoder
                .forward(&tokens_t, &audio_features, i == 0)
                .map_err(|e| crate::Error::new(format!("whisper decoder: {e}")))?;

            // 初回 step の SOT 位置の logits から no_speech 確率を取る
            if i == 0 {
                let logits = self
                    .inner
                    .decoder
                    .final_linear(&ys.i(..1).map_err(candle_err)?)
                    .map_err(candle_err)?
                    .i(0)
                    .map_err(candle_err)?
                    .i(0)
                    .map_err(candle_err)?;
                no_speech_prob_raw = Some(f64::from(
                    softmax(&logits, 0)
                        .map_err(candle_err)?
                        .i(self.protocol_tokens.no_speech as usize)
                        .map_err(candle_err)?
                        .to_scalar::<f32>()
                        .map_err(candle_err)?,
                ));
            }

            let (_, seq_len, _) = ys.dims3().map_err(candle_err)?;
            let logits = self
                .inner
                .decoder
                .final_linear(&ys.i((..1, seq_len - 1..)).map_err(candle_err)?)
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

            if next_token == self.protocol_tokens.eot
                || tokens.len() > self.config.max_target_positions
            {
                break;
            }
            sum_logprob += prob.ln();
        }

        let text = self
            .tokenizer
            .decode(&tokens, true)
            .map_err(|e| crate::Error::new(format!("tokenizer decode: {e}")))?;
        let avg_logprob_raw = sum_logprob / tokens.len().max(1) as f64;
        let avg_logprob = LogProbability::new(avg_logprob_raw).ok_or_else(|| {
            crate::Error::new(format!(
                "whisper produced non-log-probability avg_logprob: {avg_logprob_raw}"
            ))
        })?;
        // no_speech_prob は初回 step (i == 0) で必ず 1 回だけ設定される。sample_len == 0 の
        // 不正 config でループに入らなかった場合のみ None のまま抜ける。
        let no_speech_prob_raw = no_speech_prob_raw.ok_or_else(|| {
            crate::Error::new("whisper decode: sample_len == 0 (invalid max_target_positions)")
        })?;
        let no_speech_prob = Probability::new(no_speech_prob_raw).ok_or_else(|| {
            crate::Error::new(format!(
                "whisper produced out-of-range no_speech_prob: {no_speech_prob_raw}"
            ))
        })?;

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
