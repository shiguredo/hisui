use std::collections::HashMap;
use std::path::Path;

use candle_core::{DType, Device, Tensor};
use candle_onnx::onnx::ModelProto;

use crate::Result;

/// Silero VAD v5（16 kHz, 512 サンプル / フレーム）
pub struct SileroVad {
    model: ModelProto,
    device: Device,
    frame_size: usize,
    context_size: usize,
    sample_rate: Tensor,
    state: Tensor,
    context: Tensor,
    output_name: String,
    state_output_name: String,
}

impl SileroVad {
    pub fn load(model_path: &Path, device: &Device) -> Result<Self> {
        if !model_path.is_file() {
            return Err(crate::Error::new(format!(
                "silero vad model not found: {}",
                model_path.display()
            )));
        }
        let model = candle_onnx::read_file(model_path)
            .map_err(|e| crate::Error::new(format!("failed to load silero vad onnx: {e}")))?;
        let graph = model
            .graph
            .as_ref()
            .expect("silero vad onnx must have a graph");
        let output_name = graph.output[0].name.clone();
        let state_output_name = graph.output[1].name.clone();

        Ok(Self {
            model,
            device: device.clone(),
            frame_size: 512,
            context_size: 64,
            sample_rate: Tensor::new(16_000_i64, device)
                .map_err(|e| crate::Error::new(format!("silero vad sample_rate tensor: {e}")))?,
            state: Tensor::zeros((2, 1, 128), DType::F32, device)
                .map_err(|e| crate::Error::new(format!("silero vad state tensor: {e}")))?,
            context: Tensor::zeros((1, 64), DType::F32, device)
                .map_err(|e| crate::Error::new(format!("silero vad context tensor: {e}")))?,
            output_name,
            state_output_name,
        })
    }

    pub fn reset(&mut self) -> Result<()> {
        self.state = Tensor::zeros((2, 1, 128), DType::F32, &self.device)
            .map_err(|e| crate::Error::new(format!("silero vad reset state: {e}")))?;
        self.context = Tensor::zeros((1, self.context_size), DType::F32, &self.device)
            .map_err(|e| crate::Error::new(format!("silero vad reset context: {e}")))?;
        Ok(())
    }

    /// チャンクを 1 パスで評価し、平均発話確率と（任意で）発話区間 PCM を返す
    pub fn analyze_chunk(
        &mut self,
        pcm: &[f32],
        extract_above: Option<f32>,
    ) -> Result<(f32, Vec<f32>)> {
        self.reset()?;
        if pcm.len() < self.frame_size {
            return Ok((0.0, Vec::new()));
        }
        let mut sum = 0.0f32;
        let mut count = 0u32;
        let mut speech = Vec::new();
        for frame in pcm.chunks(self.frame_size) {
            if frame.len() < self.frame_size {
                break;
            }
            let prob = self.infer_frame(frame)?;
            sum += prob;
            count += 1;
            if extract_above.is_some_and(|t| prob >= t) {
                speech.extend_from_slice(frame);
            }
        }
        let avg = if count == 0 { 0.0 } else { sum / count as f32 };
        Ok((avg, speech))
    }

    fn infer_frame(&mut self, frame: &[f32]) -> Result<f32> {
        let next_context = Tensor::from_slice(
            &frame[self.frame_size - self.context_size..],
            (1, self.context_size),
            &self.device,
        )
        .map_err(|e| crate::Error::new(format!("silero vad context slice: {e}")))?;
        let chunk = Tensor::from_slice(frame, (1, self.frame_size), &self.device)
            .map_err(|e| crate::Error::new(format!("silero vad frame tensor: {e}")))?;
        let chunk = Tensor::cat(&[&self.context, &chunk], 1)
            .map_err(|e| crate::Error::new(format!("silero vad cat: {e}")))?;

        let inputs = HashMap::from([
            ("input".to_string(), chunk),
            ("sr".to_string(), self.sample_rate.clone()),
            ("state".to_string(), self.state.clone()),
        ]);
        let outputs = candle_onnx::simple_eval(&self.model, inputs)
            .map_err(|e| crate::Error::new(format!("silero vad eval: {e}")))?;

        let speech = outputs
            .get(&self.output_name)
            .expect("silero vad speech output")
            .clone();
        self.state = outputs
            .get(&self.state_output_name)
            .expect("silero vad state output")
            .clone();
        self.context = next_context;

        let prob = speech
            .flatten_all()
            .map_err(|e| crate::Error::new(format!("silero vad flatten: {e}")))?
            .to_vec1::<f32>()
            .map_err(|e| crate::Error::new(format!("silero vad prob vec: {e}")))?;
        Ok(prob[0])
    }
}

/// 既定の Silero VAD ONNX パス（`download_ml_models.sh` と揃える）
pub fn default_model_path(models_root: &Path) -> std::path::PathBuf {
    models_root.join("silero-vad/onnx/model.onnx")
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::Device;

    #[test]
    fn silero_vad_loads_downloaded_model() {
        let path = Path::new("ml-models/silero-vad/onnx/model.onnx");
        if !path.is_file() {
            return;
        }
        let device = Device::Cpu;
        let mut vad = SileroVad::load(path, &device).expect("load silero vad");
        let pcm = vec![0.0f32; 16_000];
        let (avg, _) = vad.analyze_chunk(&pcm, None).expect("analyze");
        assert!(avg >= 0.0 && avg <= 1.0);
    }
}
