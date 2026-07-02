//! Silero VAD v5 ONNX モデルのロードと 512 サンプル単発推論。
//!
//! 入力チャンクは 16 kHz モノラル f32 の 512 サンプル固定。ONNX 入力は
//! `input: [1, 576]` = 前フレーム末尾 64 サンプル (context) + 新規 512 サンプル (chunk) を cat したもの。
//! state は `[2, 1, 128]` f32、sr は `[]` i64 = 16000。
//! 推論のたびに state は ONNX 出力 (`stateN`) で置き換え、context は新規 chunk の末尾 64 サンプルで置き換える。

use std::collections::HashMap;
use std::path::Path;

use candle_core::{DType, Device, Tensor};
use candle_onnx::onnx::ModelProto;

use crate::error::Error;

/// Silero VAD v5 が要求する 1 フレームのサンプル数。
const FRAME_SIZE: usize = 512;

/// 前フレームの末尾を持ち回す context のサンプル数。
const CONTEXT_SIZE: usize = 64;

/// Silero VAD v5 が推論に要求するサンプルレート。
const SAMPLE_RATE_HZ: i64 = 16000;

/// Silero VAD v5 ONNX モデルの単発推論器。
///
/// `chunk_probability` を呼ぶたびに ONNX の出力 (発話確率 / 次 state) を受け取り、次呼び出しに向けて
/// `state` と `context` を更新する。ストリーム切り替え時は `reset` で両者を初期値に戻す。
#[derive(Debug)]
pub struct SileroVad {
    model: ModelProto,
    device: Device,
    /// 初期 state を保持しておき reset で使い回す (Tensor::clone は失敗しない)。
    initial_state: Tensor,
    /// 初期 context を保持しておき reset で使い回す。
    initial_context: Tensor,
    state: Tensor,
    context: Tensor,
    sample_rate: Tensor,
    output_name: String,
    state_output_name: String,
}

impl SileroVad {
    /// ONNX モデルを開いて state / context / sr を device 上で初期化する。
    ///
    /// パス不在・ONNX パースエラー・テンソル生成失敗はいずれも `Err` として返す (フォールバックしない)。
    pub fn load<P: AsRef<Path>>(model_path: P, device: Device) -> crate::Result<Self> {
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

        Ok(Self {
            model,
            device,
            state: initial_state.clone(),
            context: initial_context.clone(),
            initial_state,
            initial_context,
            sample_rate,
            output_name,
            state_output_name,
        })
    }

    /// 512 サンプル (32 ms @ 16 kHz) ちょうどの chunk を受けて発話確率 (0.0 - 1.0) を返す。
    ///
    /// 実装順序:
    /// 1. `input = cat(context, chunk, dim=1)` で `[1, 576]` を組み立てる
    /// 2. ONNX を推論し発話確率と次 state を得る
    /// 3. `state` を新しい state で置換、`context` を **新規 chunk の末尾 64 サンプル** で置換する
    ///    (context は ONNX 出力ではなく chunk 由来である点に注意)
    /// 4. 発話確率を返す
    pub fn chunk_probability(&mut self, chunk: &[f32]) -> crate::Result<f32> {
        if chunk.len() != FRAME_SIZE {
            return Err(Error::new(format!(
                "silero VAD requires a chunk of {FRAME_SIZE} samples, got {}",
                chunk.len()
            )));
        }

        let chunk_tensor = Tensor::from_slice(chunk, (1, FRAME_SIZE), &self.device)?;
        let input = Tensor::cat(&[&self.context, &chunk_tensor], 1)?;

        let inputs = HashMap::from([
            ("input".to_string(), input),
            ("sr".to_string(), self.sample_rate.clone()),
            ("state".to_string(), self.state.clone()),
        ]);
        let outputs = candle_onnx::simple_eval(&self.model, inputs)?;

        let speech = outputs
            .get(&self.output_name)
            .ok_or_else(|| Error::new(format!("silero VAD missing output {}", self.output_name)))?;
        let new_state = outputs.get(&self.state_output_name).ok_or_else(|| {
            Error::new(format!(
                "silero VAD missing state output {}",
                self.state_output_name
            ))
        })?;

        let probability = speech.flatten_all()?.to_vec1::<f32>()?;
        if probability.is_empty() {
            return Err(Error::new(
                "silero VAD speech output is empty after flatten",
            ));
        }

        self.state = new_state.clone();
        self.context = Tensor::from_slice(
            &chunk[FRAME_SIZE - CONTEXT_SIZE..],
            (1, CONTEXT_SIZE),
            &self.device,
        )?;

        Ok(probability[0])
    }

    /// state と context を初期値 (ゼロテンソル) にリセットする。
    ///
    /// 別 track / 別ストリーム切り替え時に呼ぶ。`Tensor::clone` は shallow copy で失敗しない。
    pub fn reset(&mut self) {
        self.state = self.initial_state.clone();
        self.context = self.initial_context.clone();
    }
}
