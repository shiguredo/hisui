use std::path::Path;

use crate::Result;

use super::silero_vad::SileroVad;

/// チャンクを Whisper に渡すかどうかの判定
pub enum VadGate {
    Off,
    Energy {
        min_speech_ratio: f32,
        rms_threshold: f32,
    },
    Silero {
        engine: SileroVad,
        /// チャンク平均発話確率の下限
        min_avg_probability: f32,
        /// 発話区間抽出時のフレーム確率しきい値（`trim_speech` 時）
        frame_probability: f32,
        trim_speech: bool,
    },
}

impl VadGate {
    pub fn off() -> Self {
        Self::Off
    }

    pub fn energy(min_speech_ratio: f32, rms_threshold: f32) -> Self {
        Self::Energy {
            min_speech_ratio,
            rms_threshold,
        }
    }

    pub fn silero(
        model_path: &Path,
        device: &candle_core::Device,
        min_avg_probability: f32,
        frame_probability: f32,
        trim_speech: bool,
    ) -> Result<Self> {
        Ok(Self::Silero {
            engine: SileroVad::load(model_path, device)?,
            min_avg_probability,
            frame_probability,
            trim_speech,
        })
    }

    pub fn should_transcribe_chunk(&mut self, pcm: &[f32]) -> Result<bool> {
        match self {
            Self::Off => Ok(true),
            Self::Energy {
                min_speech_ratio,
                rms_threshold,
            } => Ok(chunk_has_speech_energy(
                pcm,
                *min_speech_ratio,
                *rms_threshold,
            )),
            Self::Silero {
                engine,
                min_avg_probability,
                ..
            } => {
                let (avg, _) = engine.analyze_chunk(pcm, None)?;
                Ok(avg >= *min_avg_probability)
            }
        }
    }

    /// Whisper 用 PCM。Silero + trim 時は発話区間のみ。短すぎる場合は `None`
    pub fn pcm_for_whisper(&mut self, pcm: &[f32]) -> Result<Option<Vec<f32>>> {
        const MIN_SPEECH_SAMPLES: usize = 16_000 / 2; // 0.5 秒 @ 16 kHz

        match self {
            Self::Off => Ok(Some(pcm.to_vec())),
            Self::Energy { .. } => {
                if self.should_transcribe_chunk(pcm)? {
                    Ok(Some(pcm.to_vec()))
                } else {
                    Ok(None)
                }
            }
            Self::Silero {
                engine,
                min_avg_probability,
                frame_probability,
                trim_speech,
            } => {
                let extract = (*trim_speech).then_some(*frame_probability);
                let (avg, speech) = engine.analyze_chunk(pcm, extract)?;
                if avg < *min_avg_probability {
                    return Ok(None);
                }
                let pcm_out = if *trim_speech { speech } else { pcm.to_vec() };
                if pcm_out.len() < MIN_SPEECH_SAMPLES {
                    return Ok(None);
                }
                Ok(Some(pcm_out))
            }
        }
    }
}

/// 16 kHz PCM のうち「発話あり」とみなすフレームの割合（0.0〜1.0）
pub fn speech_frame_ratio(pcm: &[f32], frame_samples: usize, rms_threshold: f32) -> f32 {
    if pcm.is_empty() || frame_samples == 0 {
        return 0.0;
    }
    let mut speech_frames = 0u64;
    let mut total_frames = 0u64;
    for frame in pcm.chunks(frame_samples) {
        if frame.is_empty() {
            continue;
        }
        total_frames += 1;
        if frame_rms(frame) >= rms_threshold {
            speech_frames += 1;
        }
    }
    if total_frames == 0 {
        return 0.0;
    }
    speech_frames as f32 / total_frames as f32
}

pub fn chunk_has_speech_energy(pcm: &[f32], min_speech_ratio: f32, rms_threshold: f32) -> bool {
    const FRAME_SAMPLES: usize = 512;
    speech_frame_ratio(pcm, FRAME_SAMPLES, rms_threshold) >= min_speech_ratio
}

fn frame_rms(frame: &[f32]) -> f32 {
    let sum_sq: f32 = frame.iter().map(|s| s * s).sum();
    (sum_sq / frame.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_has_zero_ratio() {
        let pcm = vec![0.0f32; 16_000];
        let ratio = speech_frame_ratio(&pcm, 512, 0.01);
        assert!(ratio < 0.01);
    }

    #[test]
    fn tone_has_high_ratio() {
        let pcm = vec![0.3f32; 16_000];
        let ratio = speech_frame_ratio(&pcm, 512, 0.01);
        assert!(ratio > 0.9);
    }
}
