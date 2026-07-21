//! Silero VAD v5 ONNX モデルのロードと 512 サンプル単発推論。
//!
//! `SileroVadModel` は ONNX のパース結果と初期 state / context を保持する immutable な型で、
//! プロセス起動時に 1 回だけロードする (`SileroVadModel::load`)。実際の推論と可変 state は
//! `SileroVadModel::new_instance` で得られる `SileroVad` が担う。
//!
//! リアルタイムに複数 track (複数話者) の音声が interleave で流れる用途では、track ごとに
//! `new_instance` で独立した `SileroVad` を作って持つこと。1 つの `SileroVad` を複数 track で
//! 使い回すと LSTM state が混ざり、track 境界前後の判定精度が劣化する。
//!
//! ONNX 入力の内訳:
//! - `input`: `[1, 576]` = 前フレーム末尾 64 サンプル (context) + 新規 512 サンプル (chunk) を cat
//! - `state`: `[2, 1, 128]` f32
//! - `sr`: `[]` i64 = 16000
//!
//! 推論のたびに state は ONNX 出力 (`stateN`) で置き換え、context は新規 chunk の末尾 64 サンプルで
//! 置き換える。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use candle_core::{DType, Device, Tensor};
use candle_onnx::onnx::ModelProto;

use crate::error::Error;
use crate::probability::Probability;

/// Silero VAD v5 が要求する 1 フレームのサンプル数。
const FRAME_SIZE: usize = 512;

/// 前フレームの末尾を持ち回す context のサンプル数。
const CONTEXT_SIZE: usize = 64;

/// Silero VAD v5 が推論に要求するサンプルレート。
const SAMPLE_RATE_HZ: i64 = 16000;

/// Silero VAD v5 ONNX モデルの immutable な本体。
///
/// ONNX パース + 初期 Tensor 生成 (~100 ms オーダー) は `load` で 1 回だけ払う。
/// 1 プロセスで 1 個持てば十分で、複数の推論インスタンスは `new_instance` から生成する。
#[derive(Debug)]
pub struct SileroVadModel {
    model: ModelProto,
    device: Device,
    initial_state: Tensor,
    initial_context: Tensor,
    sample_rate: Tensor,
    output_name: String,
    state_output_name: String,
}

impl SileroVadModel {
    /// ONNX モデルを開き、初期 state / context / sample rate テンソルを device 上に生成する。
    ///
    /// パス不在・ONNX パースエラー・テンソル生成失敗はいずれも `Err` として返す (フォールバックしない)。
    /// 戻り値は `Arc` で共有できる形にする (複数 track 間でモデルを共有し `new_instance` で
    /// インスタンスを派生させるため)。
    pub fn load<P: AsRef<Path>>(model_path: P, device: Device) -> crate::Result<Arc<Self>> {
        let path = model_path.as_ref();
        if !path.is_file() {
            return Err(Error::new(format!(
                "silero VAD model file not found: {}",
                path.display()
            )));
        }
        let model = candle_onnx::read_file(path).map_err(|e| {
            Error::new(format!(
                "failed to parse silero VAD ONNX at {}: {e}",
                path.display()
            ))
        })?;

        let graph = model
            .graph
            .as_ref()
            .ok_or_else(|| Error::new("silero VAD ONNX has no graph"))?;
        if graph.output.len() < 2 {
            return Err(Error::new(format!(
                "silero VAD ONNX must have at least 2 outputs, got {}",
                graph.output.len()
            )));
        }
        let output_name = graph.output[0].name.clone();
        let state_output_name = graph.output[1].name.clone();

        let initial_state = Tensor::zeros((2, 1, 128), DType::F32, &device)?;
        let initial_context = Tensor::zeros((1, CONTEXT_SIZE), DType::F32, &device)?;
        let sample_rate = Tensor::new(SAMPLE_RATE_HZ, &device)?;

        Ok(Arc::new(Self {
            model,
            device,
            initial_state,
            initial_context,
            sample_rate,
            output_name,
            state_output_name,
        }))
    }

    /// 新しい推論インスタンスを生成する。state / context は初期値 (ゼロテンソル) から始まる。
    ///
    /// track / 話者ごとに個別に持つのが基本方針。複数 track が interleave で流れるリアルタイム
    /// 用途でも、それぞれ独立した `SileroVad` を持てば LSTM state が混ざらない。
    pub fn new_instance(self: &Arc<Self>) -> SileroVad {
        SileroVad {
            state: self.initial_state.clone(),
            context: self.initial_context.clone(),
            model: Arc::clone(self),
        }
    }
}

/// Silero VAD v5 ONNX モデルの推論インスタンス。
///
/// 1 つの `SileroVad` は 1 系統の音声ストリーム (1 track / 1 話者) を担当する。track / 話者境界を
/// 跨ぐときは常に `SileroVadModel::new_instance` で別インスタンスを作ること (state を混ぜない)。
#[derive(Debug)]
pub struct SileroVad {
    model: Arc<SileroVadModel>,
    state: Tensor,
    context: Tensor,
}

impl SileroVad {
    /// 512 サンプル (32 ms @ 16 kHz) ちょうどの chunk を受けて発話確率 (0.0 - 1.0) を返す。
    ///
    /// 実装順序:
    /// 1. `input = cat(context, chunk, dim=1)` で `[1, 576]` を組み立てる
    /// 2. ONNX を推論し発話確率と次 state を得る
    /// 3. `state` を新しい state で置換、`context` を **新規 chunk の末尾 64 サンプル** で置換する
    ///    (context は ONNX 出力ではなく chunk 由来である点に注意)
    /// 4. 発話確率を返す (数値誤差で `[0.0, 1.0]` を超えた場合は `Err`)
    pub fn chunk_probability(&mut self, chunk: &[f32]) -> crate::Result<Probability> {
        if chunk.len() != FRAME_SIZE {
            return Err(Error::new(format!(
                "silero VAD requires a chunk of {FRAME_SIZE} samples, got {}",
                chunk.len()
            )));
        }

        let chunk_tensor = Tensor::from_slice(chunk, (1, FRAME_SIZE), &self.model.device)?;
        let input = Tensor::cat(&[&self.context, &chunk_tensor], 1)?;

        let inputs = HashMap::from([
            ("input".to_string(), input),
            ("sr".to_string(), self.model.sample_rate.clone()),
            ("state".to_string(), self.state.clone()),
        ]);
        let outputs = candle_onnx::simple_eval(&self.model.model, inputs)?;

        let speech = outputs.get(&self.model.output_name).ok_or_else(|| {
            Error::new(format!(
                "silero VAD missing output {}",
                self.model.output_name
            ))
        })?;
        let new_state = outputs.get(&self.model.state_output_name).ok_or_else(|| {
            Error::new(format!(
                "silero VAD missing state output {}",
                self.model.state_output_name
            ))
        })?;

        let probability = speech.flatten_all()?.to_vec1::<f32>()?;
        if probability.is_empty() {
            return Err(Error::new(
                "silero VAD speech output is empty after flatten",
            ));
        }
        let raw = f64::from(probability[0]);
        let prob = Probability::new(raw).ok_or_else(|| {
            Error::new(format!(
                "silero VAD produced probability out of [0.0, 1.0] range: {raw}"
            ))
        })?;

        self.state = new_state.clone();
        self.context = Tensor::from_slice(
            &chunk[FRAME_SIZE - CONTEXT_SIZE..],
            (1, CONTEXT_SIZE),
            &self.model.device,
        )?;

        Ok(prob)
    }
}
