//! Whisper モデルの重みロードと greedy decode ループ。
//!
//! `WhisperDecoder` は candle の `Whisper` (encoder / decoder / KV cache) を保持し、
//! SOT / 言語 / task トークンの組み立てと greedy サンプリングを担う。タスクは文字起こし
//! (transcribe) 固定で、タイムスタンプトークンは出力しない (`TextFrame` の時刻は VAD 由来で
//! 埋めるため)。共有可否・可変性の詳細は [`WhisperDecoder`] の doc を参照。

use std::path::Path;

use candle_core::{D, IndexOp, Tensor};
use candle_nn::{VarBuilder, ops::softmax};
use candle_transformers::models::whisper::{
    Config, DTYPE, EOT_TOKEN, NO_SPEECH_TOKENS, NO_TIMESTAMPS_TOKEN, SOT_TOKEN, TRANSCRIBE_TOKEN,
    model::Whisper,
};
use tokenizers::Tokenizer;

use crate::Result;
use crate::probability::{LogProbability, Probability};

/// Whisper tokenizer が扱うトークンの ID (語彙インデックス)。
///
/// 生の `u32` (テンソル indexing や他の数値) と型で分離する。生の値が要る箇所は
/// `get()` で取り出す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TokenId(u32);

impl TokenId {
    pub(super) const fn new(id: u32) -> Self {
        Self(id)
    }

    pub(super) const fn get(self) -> u32 {
        self.0
    }
}

/// 1 チャンクぶんの decode 結果。
///
/// 品質指標は candle 内部と同じ f64 精度の `LogProbability` / `Probability` で保持する。
/// `TextFrame` 向けの f32 変換は上位層で行う。
#[derive(Debug)]
pub(super) struct WhisperDecodedChunk {
    pub(super) text: String,
    pub(super) avg_logprob: LogProbability,
    pub(super) no_speech_prob: Probability,
}

/// Whisper プロトコルで固定されている特殊トークンの一式。
///
/// リクエストごとに変わらないため、ロード時に一度だけ tokenizer から引いて保持する。
/// 言語トークンは per-request に変わるため本構造体には含めず、`decode_chunk` の引数で受ける。
#[derive(Debug)]
struct ProtocolTokens {
    /// SOT (start-of-transcript)。decode 対象トークン列の先頭に積む。
    sot: TokenId,
    /// タスクを「文字起こし」に固定するトークン (`translate` ではなく `transcribe` を選ぶ)。
    transcribe: TokenId,
    /// EOT (end-of-transcript)。このトークンが出た時点で decode ループを終える。
    eot: TokenId,
    /// no_speech トークン。初回 step の logits からこのトークンの確率を取り出し、
    /// 「発話がない確率 (幻覚判定用)」として上位層に返す。
    no_speech: TokenId,
    /// タイムスタンプトークンの出力を抑止するトークン (時刻は VAD 由来で埋めるため不要)。
    no_timestamps: TokenId,
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

/// Whisper 重みと greedy decode ループをまとめた型。
///
/// 内部の `Whisper` は KV cache を持つ可変状態のため `&mut self` を要求し、複数スレッドで
/// 共有できない (Silero の `new_instance` 型の共有もしない)。
#[derive(Debug)]
pub(super) struct WhisperDecoder {
    inner: Whisper,
    config: Config,
    tokenizer: Tokenizer,
    suppress_tokens: Tensor,
    /// Whisper プロトコルの固定特殊トークン (SOT / transcribe / EOT / no_speech / no_timestamps)。
    /// リクエスト間で変わる言語トークンは state に持たず、`decode_chunk` の引数で受ける。
    protocol_tokens: ProtocolTokens,
}

impl WhisperDecoder {
    /// Hugging Face の safetensors 形式 (拡張子 `.safetensors`) の重みと config・tokenizer から
    /// `WhisperDecoder` を組み立てる。
    pub(super) fn load<P: AsRef<Path>>(
        weights_path: P,
        config: Config,
        tokenizer: Tokenizer,
        device: &candle_core::Device,
    ) -> Result<Self> {
        let weights_path = weights_path.as_ref();
        // SAFETY: `from_mmaped_safetensors` は mmap 中にモデルファイルが外部から書き換え
        // られないことを利用者側で保証する必要がある。 hisui はモデルディレクトリを起動時に
        // 1 度ロードして参照するだけで、プロセス中に上書きしないため前提を満たす。
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
        })
    }

    pub(super) fn config(&self) -> &Config {
        &self.config
    }

    pub(super) fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
    }

    /// 1 チャンク (mel スペクトログラム) を decode する。
    ///
    /// `mel` は PCM を「時間 × mel bins」の 2D テンソルに変換したもの (Whisper encoder が
    /// 直接受け付ける入力形式)。上位層で `candle_transformers::models::whisper::audio::pcm_to_mel`
    /// 等で生成する。
    /// `language_token` は多言語モデルなら `Some(<言語トークン>)`、非多言語モデルなら `None` を
    /// 渡す。 リクエスト単位で完結させるため、呼び出し間で state に残さない。
    ///
    /// Whisper decoder が持つ KV (attention key/value) キャッシュは開始時と終了時に本関数内で
    /// クリアするため、呼び出し側は状態管理不要 (前回リクエストの残り state が漏れない)。
    ///
    /// hallucination の可能性がある結果もそのまま返す (text を空にしない)。 破棄判定は
    /// `no_speech_prob` と `avg_logprob` を見て呼び出し側の責務で行う (層としては本関数は
    /// 上位型 `WhisperTranscript` を知らない)。
    pub(super) fn decode_chunk(
        &mut self,
        mel: &Tensor,
        language_token: Option<TokenId>,
    ) -> Result<WhisperDecodedChunk> {
        // 前回リクエストの残 state を持ち込まないため、関数入口で全 block の KV cache を
        // クリアする。 これが本命のリセットで、下記の `encoder.forward(..., true)` と
        // 初回 `decoder.forward(..., i == 0)` の flush 引数は defensive な冗長指定
        // (encoder は self-attn のため実質 no-op、decoder は i == 0 の初回のみ効く)。
        self.inner.reset_kv_cache();
        let audio_features = self
            .inner
            .encoder
            .forward(mel, true)
            .map_err(|e| crate::Error::new(format!("whisper encoder: {e}")))?;

        let sample_len = self.config.max_target_positions / 2;
        let mut sum_logprob = 0f64;
        let mut no_speech_prob_raw: Option<f64> = None;
        let mut tokens = self.build_prefix_tokens(language_token);
        // `avg_logprob` の分母を「生成トークン数 (EOT 含む)」にするため、プレフィックス
        // (SOT / 言語 / transcribe / no_timestamps) の個数をここで固定して控えておく。
        let prefix_len = tokens.len();

        for i in 0..sample_len {
            let tokens_t = tokens_tensor(&tokens, mel.device())?;
            let ys = self
                .inner
                .decoder
                .forward(&tokens_t, &audio_features, i == 0)
                .map_err(|e| crate::Error::new(format!("whisper decoder: {e}")))?;

            if i == 0 {
                no_speech_prob_raw = Some(self.read_no_speech_prob(&ys)?);
            }

            let (next_token, prob) = self.greedy_step(&ys)?;
            tokens.push(next_token);
            // openai/whisper の GreedyDecoder は EOT 生成ステップも sum に含める
            // (`sum_logprobs += current_logprobs * (tokens[:, -1] != self.eot)` は
            // 「直前が EOT でなければ加算」なので、EOT を生成したステップ自身は加算対象)。
            // 定義を一致させるため break 判定より前に加算する。
            sum_logprob += prob.ln();

            if next_token == self.protocol_tokens.eot
                || tokens.len() > self.config.max_target_positions
            {
                break;
            }
        }

        let result = self.finalize_chunk(tokens, prefix_len, sum_logprob, no_speech_prob_raw);
        self.inner.reset_kv_cache();
        result
    }

    /// decode ループ開始時のプレフィックストークン列を組み立てる。
    /// 順序は SOT → (言語トークンがあれば) → transcribe → no_timestamps。
    fn build_prefix_tokens(&self, language_token: Option<TokenId>) -> Vec<TokenId> {
        let mut tokens = vec![self.protocol_tokens.sot];
        if let Some(language_token) = language_token {
            tokens.push(language_token);
        }
        tokens.push(self.protocol_tokens.transcribe);
        tokens.push(self.protocol_tokens.no_timestamps);
        tokens
    }

    /// 初回 step の SOT 位置の logits から no_speech トークンの確率を取り出す (f64 化)。
    fn read_no_speech_prob(&self, ys: &Tensor) -> candle_core::Result<f64> {
        let logits = self.inner.decoder.final_linear(&ys.i(..1)?)?.i(0)?.i(0)?;
        let prob = softmax(&logits, 0)?
            .i(self.protocol_tokens.no_speech.get() as usize)?
            .to_scalar::<f32>()?;
        Ok(f64::from(prob))
    }

    /// 現ステップの logits に suppress_tokens を足して argmax、選ばれたトークンとその確率を返す。
    fn greedy_step(&self, ys: &Tensor) -> candle_core::Result<(TokenId, f64)> {
        let (_, seq_len, _) = ys.dims3()?;
        let logits = self
            .inner
            .decoder
            .final_linear(&ys.i((..1, seq_len - 1..))?)?
            .i(0)?
            .i(0)?;
        let logits = logits.broadcast_add(&self.suppress_tokens)?;

        // greedy サンプリング (温度 0 固定)
        let logits_v: Vec<f32> = logits.to_vec1()?;
        let next_token = logits_v
            .iter()
            .enumerate()
            .max_by(|(_, u), (_, v)| u.total_cmp(v))
            .map(|(i, _)| TokenId::new(i as u32))
            .expect("logits must not be empty");

        let prob = f64::from(
            softmax(&logits, D::Minus1)?
                .i(next_token.get() as usize)?
                .to_scalar::<f32>()?,
        );
        Ok((next_token, prob))
    }

    /// decode ループが確定させたトークン列と累積 logprob から `WhisperDecodedChunk` を組み立てる。
    /// text 復元、`avg_logprob` / `no_speech_prob` の型付き変換 (範囲外・NaN は Err) を担う。
    ///
    /// `prefix_len` は decode 前に積んだプレフィックストークン (SOT / 言語 / transcribe /
    /// no_timestamps) の個数。 `avg_logprob` の分母は openai/whisper と一致させるため
    /// 「生成トークン数 (EOT 含む)」= `tokens.len() - prefix_len` を採る。
    /// `sample_len == 0` などで生成が 1 個も無かった場合は `max(1)` で 0 割を回避し、
    /// `sum_logprob == 0` から `avg_logprob = 0` (= log(1)) を返す。
    fn finalize_chunk(
        &self,
        tokens: Vec<TokenId>,
        prefix_len: usize,
        sum_logprob: f64,
        no_speech_prob_raw: Option<f64>,
    ) -> Result<WhisperDecodedChunk> {
        let raw_tokens: Vec<u32> = tokens.iter().map(|t| t.get()).collect();
        let text = self
            .tokenizer
            .decode(&raw_tokens, true)
            .map_err(|e| crate::Error::new(format!("tokenizer decode: {e}")))?;
        let generated_len = tokens.len().saturating_sub(prefix_len).max(1);
        let avg_logprob_raw = sum_logprob / generated_len as f64;
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
pub(super) fn token_id(tokenizer: &Tokenizer, token: &str) -> Result<TokenId> {
    match tokenizer.token_to_id(token) {
        None => Err(crate::Error::new(format!("no token-id for {token}"))),
        Some(id) => Ok(TokenId::new(id)),
    }
}

/// decode ループで毎ステップ作る「1 × N」形の tokens tensor を組む。
fn tokens_tensor(tokens: &[TokenId], device: &candle_core::Device) -> Result<Tensor> {
    let raw: Vec<u32> = tokens.iter().map(|t| t.get()).collect();
    Tensor::new(raw.as_slice(), device)
        .map_err(|e| crate::Error::new(format!("tokens tensor: {e}")))?
        .unsqueeze(0)
        .map_err(|e| crate::Error::new(format!("tokens unsqueeze: {e}")))
}
