use std::time::Duration;

use crate::ResultExt;
use shiguredo_mp4::{
    FixedPointNumber,
    boxes::{AudioSampleEntryFields, SampleEntry},
};

use crate::{metadata::SourceId, types::CodecName};

// 現時点では 48000 固定
pub const SAMPLE_RATE: u16 = 48000;

// 現時点ではステレオ固定
pub const CHANNELS: u16 = 2;

// エンコードパラメーターのデフォルト値
pub const DEFAULT_BITRATE: usize = 65536;

#[derive(Debug, Clone)]
pub struct AudioData {
    pub source_id: Option<SourceId>,
    pub data: Vec<u8>,
    pub format: AudioFormat,
    pub stereo: bool,
    pub sample_rate: u16,
    pub timestamp: Duration,
    pub duration: Duration,
    pub sample_entry: Option<SampleEntry>,
}

impl AudioData {
    pub fn stereo_samples(&self) -> crate::Result<impl '_ + Iterator<Item = (i16, i16)>> {
        (self.format == AudioFormat::I16Be).or_fail()?;
        self.stereo.or_fail()?;

        let samples = self.data.chunks_exact(4).map(|c| {
            (
                i16::from_be_bytes([c[0], c[1]]),
                i16::from_be_bytes([c[2], c[3]]),
            )
        });
        Ok(samples)
    }

    pub fn interleaved_stereo_samples(&self) -> crate::Result<impl '_ + Iterator<Item = i16>> {
        (self.format == AudioFormat::I16Be).or_fail()?;
        self.stereo.or_fail()?;

        let samples = self.data.chunks_exact(4).flat_map(|c| {
            [
                i16::from_be_bytes([c[0], c[1]]),
                i16::from_be_bytes([c[2], c[3]]),
            ]
            .into_iter()
        });
        Ok(samples)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    I16Be,
    Opus,
    Aac,
}

impl AudioFormat {
    pub fn codec_name(self) -> Option<CodecName> {
        match self {
            AudioFormat::I16Be => None,
            AudioFormat::Opus => Some(CodecName::Opus),
            AudioFormat::Aac => Some(CodecName::Aac),
        }
    }
}

impl std::fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = self.codec_name().map(|n| n.as_str()).unwrap_or("PCM");
        write!(f, "{name}")
    }
}

pub fn sample_entry_audio_fields() -> AudioSampleEntryFields {
    AudioSampleEntryFields {
        data_reference_index: AudioSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
        channelcount: CHANNELS,
        samplesize: 16,
        samplerate: FixedPointNumber::new(SAMPLE_RATE, 0),
    }
}

pub fn resample(
    pcm_data: &[i16],       // 現在のフレームのオリジナルの音声データ（入力）
    prev_pcm_data: &[i16],  // 前フレームの音声データ（フレーム境界での補間に使用）
    input_sample_rate: u32, // 入力サンプルレート。出力は SAMPLE_RATE 固定
    original_samples: u64,  // これまでに処理された pcm_data.len() の累計
    resampled_samples: u64, // これまでに出力されたリサンプリング後のサンプル数の累計
) -> Option<Vec<i16>> {
    if input_sample_rate == SAMPLE_RATE as u32 {
        return None;
    }

    let ratio = SAMPLE_RATE as f64 / input_sample_rate as f64;
    let total_original_samples = (original_samples + pcm_data.len() as u64) as f64;
    let ideal_resampled_len = (total_original_samples * ratio).floor() as usize;
    let output_len = ideal_resampled_len.saturating_sub(resampled_samples as usize);

    let mut output = Vec::with_capacity(output_len);

    for out_idx in 0..output_len {
        let target_sample = resampled_samples as f64 + out_idx as f64;
        let in_pos_global = target_sample / ratio;
        let in_pos = in_pos_global - original_samples as f64;
        let in_idx = in_pos.floor() as usize;

        if in_idx >= pcm_data.len() {
            // 通常はここに到達しないはずだが、念のためにスキップしておく
            continue;
        }

        let frac = in_pos.fract();

        let sample0 = pcm_data[in_idx] as f64;

        // 補間サンプルを取得
        let sample1 = if in_idx + 1 < pcm_data.len() {
            pcm_data[in_idx + 1] as f64
        } else if !prev_pcm_data.is_empty() {
            // チャンク境界: 次サンプルが現在のチャンクにない場合、前チャンクの最後を使用
            *prev_pcm_data.last().unwrap() as f64
        } else {
            sample0
        };

        let interpolated = sample0 * (1.0 - frac) + sample1 * frac;
        output.push(interpolated.round() as i16);
    }

    Some(output)
}

// モノラルを複製してステレオに変換する
pub fn mono_to_stereo(mono_samples: &[i16]) -> Vec<i16> {
    mono_samples
        .iter()
        .flat_map(|&sample| [sample, sample])
        .collect()
}
