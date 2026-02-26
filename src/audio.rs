use std::time::Duration;

use shiguredo_mp4::{
    FixedPointNumber,
    boxes::{AudioSampleEntryFields, SampleEntry},
};

use crate::types::CodecName;

// エンコードパラメーターのデフォルト値
pub const DEFAULT_BITRATE: usize = 65536;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Channels(u8);

impl Channels {
    pub const MONO: Self = Self(1);
    pub const STEREO: Self = Self(2);

    pub fn from_u8(value: u8) -> crate::Result<Self> {
        match value {
            1 => Ok(Self::MONO),
            2 => Ok(Self::STEREO),
            _ => Err(crate::Error::new(format!(
                "unsupported audio channel count: {value}"
            ))),
        }
    }

    pub fn from_u16(value: u16) -> crate::Result<Self> {
        let value = u8::try_from(value)
            .map_err(|_| crate::Error::new(format!("unsupported audio channel count: {value}")))?;
        Self::from_u8(value)
    }

    pub const fn get(self) -> u8 {
        self.0
    }

    pub const fn is_mono(self) -> bool {
        self.0 == Self::MONO.0
    }

    pub const fn is_stereo(self) -> bool {
        self.0 == Self::STEREO.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SampleRate(u32);

impl SampleRate {
    pub const HZ_48000: Self = Self(48_000);

    pub fn from_u16(value: u16) -> crate::Result<Self> {
        Self::from_u32(u32::from(value))
    }

    pub fn from_u32(value: u32) -> crate::Result<Self> {
        if value == 0 {
            return Err(crate::Error::new("unsupported audio sample rate: 0"));
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub fn as_u16(self) -> crate::Result<u16> {
        u16::try_from(self.0)
            .map_err(|_| crate::Error::new(format!("unsupported audio sample rate: {}", self.0)))
    }

    pub fn duration_from_samples(self, samples_per_channel: u64) -> Duration {
        Duration::from_secs(samples_per_channel) / self.get()
    }
}

#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub data: Vec<u8>,
    pub format: AudioFormat,
    pub channels: Channels,
    pub sample_rate: SampleRate,
    pub timestamp: Duration,
    pub duration: Duration,
    pub sample_entry: Option<SampleEntry>,
}

impl AudioFrame {
    pub fn is_stereo(&self) -> bool {
        self.channels.is_stereo()
    }

    pub fn is_mono(&self) -> bool {
        self.channels.is_mono()
    }

    pub fn stereo_samples(&self) -> crate::Result<impl '_ + Iterator<Item = (i16, i16)>> {
        if self.format != AudioFormat::I16Be {
            return Err(crate::Error::new(format!(
                "expected I16Be format, got {}",
                self.format
            )));
        }
        if !self.channels.is_stereo() {
            return Err(crate::Error::new("expected stereo audio data"));
        }

        let samples = self.data.chunks_exact(4).map(|c| {
            (
                i16::from_be_bytes([c[0], c[1]]),
                i16::from_be_bytes([c[2], c[3]]),
            )
        });
        Ok(samples)
    }

    pub fn interleaved_stereo_samples(&self) -> crate::Result<impl '_ + Iterator<Item = i16>> {
        if self.format != AudioFormat::I16Be {
            return Err(crate::Error::new(format!(
                "expected I16Be format, got {}",
                self.format
            )));
        }
        if !self.channels.is_stereo() {
            return Err(crate::Error::new("expected stereo audio data"));
        }

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
        channelcount: u16::from(Channels::STEREO.get()),
        samplesize: 16,
        samplerate: FixedPointNumber::new(
            SampleRate::HZ_48000
                .as_u16()
                .expect("default sample rate must fit into u16"),
            0,
        ),
    }
}

pub fn resample(
    pcm_data: &[i16],              // 現在のフレームのオリジナルの音声データ（入力）
    prev_pcm_data: &[i16],         // 前フレームの音声データ（フレーム境界での補間に使用）
    input_sample_rate: SampleRate, // 入力サンプルレート。出力は SampleRate::HZ_48000 固定
    original_samples: u64,         // これまでに処理された pcm_data.len() の累計
    resampled_samples: u64,        // これまでに出力されたリサンプリング後のサンプル数の累計
) -> Option<Vec<i16>> {
    if input_sample_rate == SampleRate::HZ_48000 {
        return None;
    }

    let ratio = SampleRate::HZ_48000.get() as f64 / input_sample_rate.get() as f64;
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

#[cfg(test)]
mod tests {
    use super::{Channels, SampleRate};

    #[test]
    fn channels_constants_are_valid() {
        assert!(Channels::MONO.is_mono());
        assert!(Channels::STEREO.is_stereo());
        assert_eq!(Channels::MONO.get(), 1);
        assert_eq!(Channels::STEREO.get(), 2);
    }

    #[test]
    fn channels_from_u8_accepts_mono_and_stereo() {
        assert_eq!(Channels::from_u8(1).expect("must be mono"), Channels::MONO);
        assert_eq!(
            Channels::from_u8(2).expect("must be stereo"),
            Channels::STEREO
        );
    }

    #[test]
    fn channels_from_u16_accepts_mono_and_stereo() {
        assert_eq!(Channels::from_u16(1).expect("must be mono"), Channels::MONO);
        assert_eq!(
            Channels::from_u16(2).expect("must be stereo"),
            Channels::STEREO
        );
    }

    #[test]
    fn channels_rejects_unsupported_values() {
        assert!(Channels::from_u8(0).is_err());
        assert!(Channels::from_u8(3).is_err());
        assert!(Channels::from_u16(0).is_err());
        assert!(Channels::from_u16(3).is_err());
    }

    #[test]
    fn sample_rate_from_u16_accepts_non_zero_values() {
        assert_eq!(
            SampleRate::from_u16(48_000).expect("must be valid").get(),
            48_000
        );
    }

    #[test]
    fn sample_rate_from_u32_accepts_non_zero_values() {
        assert_eq!(
            SampleRate::from_u32(96_000).expect("must be valid").get(),
            96_000
        );
    }

    #[test]
    fn sample_rate_rejects_zero() {
        assert!(SampleRate::from_u16(0).is_err());
        assert!(SampleRate::from_u32(0).is_err());
    }

    #[test]
    fn sample_rate_as_u16_rejects_large_values() {
        assert_eq!(
            SampleRate::from_u32(48_000)
                .expect("must be valid")
                .as_u16()
                .expect("must fit"),
            48_000
        );
        assert!(
            SampleRate::from_u32(96_000)
                .expect("must be valid")
                .as_u16()
                .is_err()
        );
    }
}
