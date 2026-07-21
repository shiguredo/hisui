//! Whisper 文字起こしパイプライン。

use std::path::{Path, PathBuf};

use candle_core::Tensor;
use candle_transformers::models::whisper::{Config, LOGPROB_THRESHOLD, NO_SPEECH_THRESHOLD, audio};
use tokenizers::Tokenizer;

pub mod decode;
pub mod multilingual;

use decode::WhisperDecoder;
use multilingual::ResolvedLanguage;

use crate::probability::{LogProbability, Probability};
use crate::text::LanguageCode;

/// Whisper の推論結果。
#[derive(Debug)]
pub struct WhisperTranscript {
    pub text: String,
    pub language: Option<LanguageCode>,
    pub no_speech_prob: Probability,
    pub avg_logprob: LogProbability,
}

impl WhisperTranscript {
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

/// `WhisperDecoder` に mel フィルタと言語解決を載せた PCM 入力の入口。
///
/// 共有可否と `&mut self` を要求する理由は内部の [`WhisperDecoder`] に従う (本型は
/// 薄いラッパで、複数スレッド共有不可の性質を decoder から引き継ぐ)。
#[derive(Debug)]
pub struct WhisperPipeline {
    decoder: WhisperDecoder,
    mel_filters: Vec<f32>,
    candle_device: candle_core::Device,
}

impl WhisperPipeline {
    /// `model_dir` には `config.json`, `tokenizer.json`, `model.safetensors` が必要。
    pub fn load<P: AsRef<Path>>(
        model_dir: P,
        candle_device: candle_core::Device,
    ) -> crate::Result<Self> {
        let model_dir = validate_model_dir(model_dir.as_ref())?;
        let config = load_whisper_config(&model_dir.join("config.json"))?;
        let tokenizer = Tokenizer::from_file(model_dir.join("tokenizer.json"))
            .map_err(|e| crate::Error::new(format!("load tokenizer: {e}")))?;
        let mel_filters = load_mel_filters(config.num_mel_bins)?;
        let decoder = WhisperDecoder::load(
            model_dir.join("model.safetensors"),
            config,
            tokenizer,
            &candle_device,
        )?;
        Ok(Self {
            decoder,
            mel_filters,
            candle_device,
        })
    }

    /// 16 kHz mono PCM を 1 リクエストぶん文字起こしする。
    pub fn transcribe_pcm16k(
        &mut self,
        pcm: &[f32],
        language: &LanguageCode,
    ) -> crate::Result<WhisperTranscript> {
        let config = self.decoder.config();
        let mel = audio::pcm_to_mel(config, pcm, &self.mel_filters);
        let mel_len = mel.len();
        if mel_len == 0 {
            // defensive 分岐: candle の pcm_to_mel は空 PCM でも必ず 1500 frames に
            // パディングして返すため、現状の実装ではここには到達しない。 上流仕様が
            // 変わって空 mel を返しうる場合に備え、「発話が観測されなかった」ことを
            // 表す値で埋める:
            //   - no_speech_prob = 1.0 (発話なしの最大確信)
            //   - avg_logprob = -∞ (トークンが生成されていない = 対数確率の下限)
            //   - language = None (音声から言語を検出する対象が無かった)
            return Ok(WhisperTranscript {
                text: String::new(),
                language: None,
                no_speech_prob: Probability::ONE,
                avg_logprob: LogProbability::NEG_INFINITY,
            });
        }

        let num_mel_bins = config.num_mel_bins;
        let mel = Tensor::from_vec(
            mel,
            (1, num_mel_bins, mel_len / num_mel_bins),
            &self.candle_device,
        )
        .map_err(|e| crate::Error::new(format!("mel tensor: {e}")))?;
        let mel = narrow_mel_for_encoder(&mel)?;

        let resolved_language = self.resolve_language(language)?;
        let result = self
            .decoder
            .decode_chunk(&mel, Some(resolved_language.token_id))?;

        Ok(WhisperTranscript {
            text: result.text.trim().to_owned(),
            language: Some(resolved_language.code),
            no_speech_prob: result.no_speech_prob,
            avg_logprob: result.avg_logprob,
        })
    }

    fn resolve_language(&self, language: &LanguageCode) -> crate::Result<ResolvedLanguage> {
        if !multilingual::is_multilingual_config(self.decoder.config()) {
            return Err(crate::Error::new(format!(
                "language is not supported for non-multilingual whisper model: {language}"
            )));
        }
        let token_id = multilingual::language_token_from_code(self.decoder.tokenizer(), language)?;
        Ok(ResolvedLanguage {
            code: LanguageCode::new(language.get().trim()),
            token_id,
        })
    }
}

/// mel の時間軸を最大 3000 フレームに切り詰める (Whisper encoder の入力長上限)。
///
/// Whisper encoder は mel を内部の conv 層で 1/2 に downsample してから
/// `max_source_positions = 1500` の positional embedding を加えるため、 mel 側は
/// `1500 × 2 = 3000` フレーム (= 30 秒相当) が上限。 candle の `pcm_to_mel` は末尾に
/// パディングを足す都合で mel フレーム数が 3000 を超えることがあるため、ここで明示的に
/// narrow して encoder の入力長を安全側に揃える。
fn narrow_mel_for_encoder(mel: &Tensor) -> crate::Result<Tensor> {
    let (_batch, _bins, seq_len) = mel
        .dims3()
        .map_err(|e| crate::Error::new(format!("mel dims: {e}")))?;
    mel.narrow(2, 0, usize::min(seq_len, 3000))
        .map_err(|e| crate::Error::new(format!("mel narrow: {e}")))
}

/// Whisper encoder に食わせる mel スペクトログラムを作るための mel filter bank を返す。
///
/// candle の `audio::pcm_to_mel` は filter bank を引数で受け取る設計のため、hisui 側で
/// 事前計算済みのバイナリ (`melfilters.bytes`) を同梱している。中身は OpenAI Whisper
/// 標準の「80 mel bins × 201 frequency bins」の f32 行列を little-endian で並べたもの
/// (candle-examples 同梱由来、合計 64320 バイト)。
///
/// mel filter は Whisper のアーキテクチャ (16 kHz サンプリング、FFT サイズ 400 等) から
/// 一意に決まる不変値なので、生成し直したりバージョン管理したりする必要はない。バイナリ
/// のまま同梱している理由は、Rust 側で `&[f32]` リテラル化するとソースが 3〜4 倍に膨れる
/// のに、値は数値なので可読性は向上しないため。
///
/// 128-bin (large-v3 系) は同梱していないため Err にする。tiny / base / small のみ対応。
fn load_mel_filters(num_mel_bins: usize) -> crate::Result<Vec<f32>> {
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
    Ok(mel_bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

/// Hugging Face の Whisper モデルディレクトリを検証する。
fn validate_model_dir(model_dir: &Path) -> crate::Result<PathBuf> {
    for name in ["config.json", "tokenizer.json", "model.safetensors"] {
        let path = model_dir.join(name);
        if !path.is_file() {
            return Err(crate::Error::new(format!(
                "missing {name} in model directory {}",
                model_dir.display()
            )));
        }
    }
    Ok(model_dir.to_path_buf())
}

/// Hugging Face `config.json` の RawJsonValue を candle の `Config` にパースする。
fn parse_whisper_config(
    value: nojson::RawJsonValue<'_, '_>,
) -> std::result::Result<Config, nojson::JsonParseError> {
    Ok(Config {
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

/// `config.json` を nojson で読み込み、candle の `Config` に変換する。
fn load_whisper_config(path: &Path) -> crate::Result<Config> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| crate::Error::new(format!("read {}: {e}", path.display())))?;
    let json = nojson::RawJson::parse(&text)?;
    parse_whisper_config(json.value())
        .map_err(|e| crate::Error::new(format!("parse {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 必須フィールドだけを含む config.json 断片を正しくパースできる。
    #[test]
    fn parse_whisper_config_from_str() {
        let text = r#"{
            "num_mel_bins": 80,
            "max_source_positions": 1500,
            "d_model": 384,
            "encoder_attention_heads": 6,
            "encoder_layers": 4,
            "vocab_size": 51864,
            "max_target_positions": 448,
            "decoder_attention_heads": 6,
            "decoder_layers": 4,
            "suppress_tokens": [1, 2, 3]
        }"#;
        let json = nojson::RawJson::parse(text).expect("JSON をパースできること");
        let config = parse_whisper_config(json.value()).expect("config をパースできること");
        assert_eq!(config.num_mel_bins, 80);
        assert_eq!(config.max_source_positions, 1500);
        assert_eq!(config.suppress_tokens, vec![1, 2, 3]);
    }

    /// 必須フィールドが欠けているとき parse_whisper_config は Err を返す。
    #[test]
    fn parse_whisper_config_rejects_missing_required_field() {
        // num_mel_bins を抜いた config
        let text = r#"{
            "max_source_positions": 1500,
            "d_model": 384,
            "encoder_attention_heads": 6,
            "encoder_layers": 4,
            "vocab_size": 51864,
            "max_target_positions": 448,
            "decoder_attention_heads": 6,
            "decoder_layers": 4
        }"#;
        let json = nojson::RawJson::parse(text).expect("JSON をパースできること");
        assert!(
            parse_whisper_config(json.value()).is_err(),
            "必須フィールド欠落は Err のはず"
        );
    }

    /// 必須フィールドが負値のとき parse_whisper_config は Err を返す (usize 変換失敗)。
    #[test]
    fn parse_whisper_config_rejects_negative_required_field() {
        let text = r#"{
            "num_mel_bins": -1,
            "max_source_positions": 1500,
            "d_model": 384,
            "encoder_attention_heads": 6,
            "encoder_layers": 4,
            "vocab_size": 51864,
            "max_target_positions": 448,
            "decoder_attention_heads": 6,
            "decoder_layers": 4
        }"#;
        let json = nojson::RawJson::parse(text).expect("JSON をパースできること");
        assert!(
            parse_whisper_config(json.value()).is_err(),
            "負値は Err のはず"
        );
    }

    /// 必須フィールドが整数でないとき parse_whisper_config は Err を返す。
    #[test]
    fn parse_whisper_config_rejects_non_integer_required_field() {
        let text = r#"{
            "num_mel_bins": "eighty",
            "max_source_positions": 1500,
            "d_model": 384,
            "encoder_attention_heads": 6,
            "encoder_layers": 4,
            "vocab_size": 51864,
            "max_target_positions": 448,
            "decoder_attention_heads": 6,
            "decoder_layers": 4
        }"#;
        let json = nojson::RawJson::parse(text).expect("JSON をパースできること");
        assert!(
            parse_whisper_config(json.value()).is_err(),
            "非整数値は Err のはず"
        );
    }

    /// suppress_tokens が省略されているとき parse_whisper_config は空 Vec で成功する。
    #[test]
    fn parse_whisper_config_defaults_missing_suppress_tokens_to_empty() {
        let text = r#"{
            "num_mel_bins": 80,
            "max_source_positions": 1500,
            "d_model": 384,
            "encoder_attention_heads": 6,
            "encoder_layers": 4,
            "vocab_size": 51864,
            "max_target_positions": 448,
            "decoder_attention_heads": 6,
            "decoder_layers": 4
        }"#;
        let json = nojson::RawJson::parse(text).expect("JSON をパースできること");
        let config = parse_whisper_config(json.value()).expect("suppress_tokens 省略は Ok のはず");
        assert!(config.suppress_tokens.is_empty(), "省略時は空 Vec が入る");
    }

    /// no_speech_prob > 0.6 かつ avg_logprob < -1.0 で is_likely_no_speech は true。
    #[test]
    fn is_likely_no_speech_true_when_both_thresholds_exceeded() {
        let transcript = WhisperTranscript {
            text: String::new(),
            language: None,
            no_speech_prob: Probability::new(0.7).expect("有効"),
            avg_logprob: LogProbability::new(-1.5).expect("有効"),
        };
        assert!(
            transcript.is_likely_no_speech(),
            "両閾値を超えたら true のはず"
        );
    }

    /// no_speech_prob = 0.6 ちょうどは `>` 比較で false (境界の取り違え検出)。
    #[test]
    fn is_likely_no_speech_false_at_no_speech_boundary() {
        let transcript = WhisperTranscript {
            text: String::new(),
            language: None,
            no_speech_prob: Probability::new(0.6).expect("有効"),
            avg_logprob: LogProbability::new(-1.5).expect("有効"),
        };
        assert!(
            !transcript.is_likely_no_speech(),
            "no_speech_prob = 0.6 ちょうどは `>` 比較で false のはず"
        );
    }

    /// avg_logprob = -1.0 ちょうどは `<` 比較で false (境界の取り違え検出)。
    #[test]
    fn is_likely_no_speech_false_at_logprob_boundary() {
        let transcript = WhisperTranscript {
            text: String::new(),
            language: None,
            no_speech_prob: Probability::new(0.7).expect("有効"),
            avg_logprob: LogProbability::new(-1.0).expect("有効"),
        };
        assert!(
            !transcript.is_likely_no_speech(),
            "avg_logprob = -1.0 ちょうどは `<` 比較で false のはず"
        );
    }

    /// no_speech_prob だけ閾値超えでは false (両条件を AND で要求)。
    #[test]
    fn is_likely_no_speech_false_when_only_no_speech_exceeded() {
        let transcript = WhisperTranscript {
            text: String::new(),
            language: None,
            no_speech_prob: Probability::new(0.9).expect("有効"),
            avg_logprob: LogProbability::new(-0.5).expect("有効"),
        };
        assert!(
            !transcript.is_likely_no_speech(),
            "no_speech_prob だけ閾値超えでは false のはず"
        );
    }

    /// tempdir を作るヘルパ (tests/test_ml_audio_silero_vad.rs の非 ONNX テストと同じ流儀)。
    fn make_tempdir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hisui_test_whisper_model_dir_{label}_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("tempdir 作成");
        dir
    }

    /// 3 ファイル (config.json / tokenizer.json / model.safetensors) が揃った
    /// ディレクトリは Ok を返す。
    #[test]
    fn validate_model_dir_accepts_complete_directory() {
        let dir = make_tempdir("complete");
        for name in ["config.json", "tokenizer.json", "model.safetensors"] {
            std::fs::write(dir.join(name), b"").expect("空ファイル書き込み");
        }
        let result = validate_model_dir(&dir);
        for name in ["config.json", "tokenizer.json", "model.safetensors"] {
            let _ = std::fs::remove_file(dir.join(name));
        }
        let _ = std::fs::remove_dir(&dir);
        result.expect("3 ファイル揃っていれば Ok のはず");
    }

    /// config.json だけ欠落しているディレクトリは Err、メッセージに欠落名を含む。
    #[test]
    fn validate_model_dir_rejects_missing_config_json() {
        assert_missing_file_is_err("config.json");
    }

    /// tokenizer.json だけ欠落しているディレクトリは Err、メッセージに欠落名を含む。
    #[test]
    fn validate_model_dir_rejects_missing_tokenizer_json() {
        assert_missing_file_is_err("tokenizer.json");
    }

    /// model.safetensors だけ欠落しているディレクトリは Err、メッセージに欠落名を含む。
    #[test]
    fn validate_model_dir_rejects_missing_model_safetensors() {
        assert_missing_file_is_err("model.safetensors");
    }

    /// 指定ファイルだけを欠落させた tempdir を作り、validate_model_dir が Err を返し
    /// エラーメッセージに欠落ファイル名を含むことを検証する共通ヘルパ。
    fn assert_missing_file_is_err(missing_name: &str) {
        let dir = make_tempdir(&format!("missing_{}", missing_name.replace('.', "_")));
        for name in ["config.json", "tokenizer.json", "model.safetensors"] {
            if name == missing_name {
                continue;
            }
            std::fs::write(dir.join(name), b"").expect("空ファイル書き込み");
        }
        let result = validate_model_dir(&dir);
        for name in ["config.json", "tokenizer.json", "model.safetensors"] {
            let _ = std::fs::remove_file(dir.join(name));
        }
        let _ = std::fs::remove_dir(&dir);
        let err = result.expect_err("欠落があれば Err のはず");
        let message = err.display().to_string();
        assert!(
            message.contains(missing_name),
            "エラーメッセージに欠落ファイル名 {missing_name} が含まれるべき: {message}"
        );
    }
}
