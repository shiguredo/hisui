use crate::Result;
use crate::audio::AudioFrame;
use crate::ml::audio::whisper;

/// Whisper 前処理で想定する入力サンプルレート（48 kHz → 16 kHz の 3:1 間引き）
const INPUT_SAMPLE_RATE_HZ: u32 = 48_000;

/// 48 kHz → 16 kHz 用の 6 タップ FIR（3:1 間引き前のローパス、係数和 = 1）
const FIR_48K_TO_16K: [f32; 6] = [0.05, 0.15, 0.3, 0.3, 0.15, 0.05];

const DECIMATION: usize = 3;

/// 48 kHz mono を 16 kHz mono にダウンサンプルする（ストリーミング対応）
struct Decimator48kTo16k {
    delay: [f32; FIR_48K_TO_16K.len()],
    delay_write: usize,
    input_count: u64,
}

impl Decimator48kTo16k {
    fn new() -> Self {
        Self {
            delay: [0.0; FIR_48K_TO_16K.len()],
            delay_write: 0,
            input_count: 0,
        }
    }

    fn push(&mut self, samples: &[f32], out: &mut Vec<f32>) {
        for &sample in samples {
            self.delay[self.delay_write] = sample;
            self.delay_write = (self.delay_write + 1) % FIR_48K_TO_16K.len();
            self.input_count += 1;

            if self.input_count < FIR_48K_TO_16K.len() as u64 {
                continue;
            }
            if (self.input_count - FIR_48K_TO_16K.len() as u64) % DECIMATION as u64 != 0 {
                continue;
            }

            let mut acc = 0.0f32;
            for (i, &coef) in FIR_48K_TO_16K.iter().enumerate() {
                let idx = (self.delay_write + i) % FIR_48K_TO_16K.len();
                acc += coef * self.delay[idx];
            }
            out.push(acc);
        }
    }
}

/// 入力音声を 16 kHz mono f32 にリサンプルし、Whisper 用チャンクを切り出す
pub struct AudioChunkBuffer {
    decimator: Decimator48kTo16k,
    pcm_16k: Vec<f32>,
    chunk_samples: usize,
}

impl AudioChunkBuffer {
    pub fn new(input_sample_rate: u32, chunk_secs: u32) -> Result<Self> {
        if input_sample_rate != INPUT_SAMPLE_RATE_HZ {
            return Err(crate::Error::new(format!(
                "audio ml requires {INPUT_SAMPLE_RATE_HZ} Hz input, got {input_sample_rate} Hz"
            )));
        }

        let chunk_samples = usize::try_from(chunk_secs)
            .map_err(|_| crate::Error::new("chunk_secs is too large"))?
            * whisper::SAMPLE_RATE;

        Ok(Self {
            decimator: Decimator48kTo16k::new(),
            pcm_16k: Vec::new(),
            chunk_samples,
        })
    }

    /// `AudioFrame` を取り込み、チャンクが溜まったら 16 kHz PCM を返す
    pub fn push_frame(&mut self, frame: &AudioFrame) -> Result<Option<Vec<f32>>> {
        let mono = audio_frame_to_mono_f32(frame)?;
        self.decimator.push(&mono, &mut self.pcm_16k);
        if self.pcm_16k.len() >= self.chunk_samples {
            let chunk: Vec<f32> = self.pcm_16k.drain(..self.chunk_samples).collect();
            return Ok(Some(chunk));
        }
        Ok(None)
    }

    /// 残りをフラッシュ（EOS 時）
    pub fn flush(&mut self) -> Option<Vec<f32>> {
        if self.pcm_16k.len() < whisper::SAMPLE_RATE / 2 {
            self.pcm_16k.clear();
            return None;
        }
        Some(std::mem::take(&mut self.pcm_16k))
    }
}

fn audio_frame_to_mono_f32(frame: &AudioFrame) -> Result<Vec<f32>> {
    if frame.format != crate::audio::AudioFormat::I16Be {
        return Err(crate::Error::new(format!(
            "expected I16Be audio, got {}",
            frame.format
        )));
    }
    if !frame.data.len().is_multiple_of(2) {
        return Err(crate::Error::new(format!(
            "invalid I16Be audio data length: {}",
            frame.data.len()
        )));
    }

    let channels = frame.channels.get() as usize;
    let samples: Vec<i16> = frame
        .data
        .chunks_exact(2)
        .map(|c| i16::from_be_bytes([c[0], c[1]]))
        .collect();

    let mono: Vec<f32> = match channels {
        1 => samples
            .into_iter()
            .map(|s| f32::from(s) / 32768.0)
            .collect(),
        2 => samples
            .chunks_exact(2)
            .map(|c| (f32::from(c[0]) + f32::from(c[1])) / 2.0 / 32768.0)
            .collect(),
        n => {
            return Err(crate::Error::new(format!(
                "unsupported channel count for whisper: {n}"
            )));
        }
    };
    Ok(mono)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimator_outputs_one_third_samples() {
        let mut decimator = Decimator48kTo16k::new();
        let input: Vec<f32> = (0..3000).map(|i| i as f32 * 0.001).collect();
        let mut out = Vec::new();
        decimator.push(&input, &mut out);
        // 最初の FIR_LEN-1 サンプルは出力なし、その後 3 入力に 1 出力
        let expected = (input.len() - FIR_48K_TO_16K.len()) / DECIMATION + 1;
        assert_eq!(out.len(), expected);
    }

    #[test]
    fn decimator_preserves_dc_gain() {
        let mut decimator = Decimator48kTo16k::new();
        let input = vec![0.5f32; 300];
        let mut out = Vec::new();
        decimator.push(&input, &mut out);
        assert!(!out.is_empty());
        for sample in &out[10..] {
            assert!((*sample - 0.5).abs() < 0.02, "expected ~0.5, got {sample}");
        }
    }

    #[test]
    fn new_rejects_non_48k_sample_rate() {
        match AudioChunkBuffer::new(44_100, 5) {
            Err(e) => assert!(e.reason.contains("48000")),
            Ok(_) => panic!("expected error for 44100 Hz"),
        }
    }
}
