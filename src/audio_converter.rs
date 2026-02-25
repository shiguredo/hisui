use std::time::Duration;

use crate::audio::{AudioFormat, AudioFrame};

#[derive(Debug, Clone, Default)]
pub struct AudioConverterBuilder {
    target_format: Option<AudioFormat>,
    target_sample_rate: Option<u16>,
    target_stereo: Option<bool>,
}

impl AudioConverterBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn format(mut self, format: AudioFormat) -> Self {
        self.target_format = Some(format);
        self
    }

    pub fn sample_rate(mut self, sample_rate: u16) -> Self {
        self.target_sample_rate = Some(sample_rate);
        self
    }

    pub fn stereo(mut self, stereo: bool) -> Self {
        self.target_stereo = Some(stereo);
        self
    }

    pub fn build(self) -> AudioConverter {
        AudioConverter {
            target_format: self.target_format,
            target_sample_rate: self.target_sample_rate,
            target_stereo: self.target_stereo,
            state: ResampleState::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResampleStateKey {
    input_sample_rate: u16,
    target_sample_rate: u16,
    stereo: bool,
}

#[derive(Debug, Default)]
struct ResampleState {
    original_samples: u64,
    resampled_samples: u64,
    prev_input_samples: Vec<i16>,
    key: Option<ResampleStateKey>,
}

/// AudioConverter は状態を持つため、複数ストリームで共有せず、ストリーム単位で生成すること。
#[derive(Debug)]
pub struct AudioConverter {
    target_format: Option<AudioFormat>,
    target_sample_rate: Option<u16>,
    target_stereo: Option<bool>,
    state: ResampleState,
}

impl AudioConverter {
    pub fn convert(&mut self, frame: &AudioFrame) -> crate::Result<AudioFrame> {
        let target_format = self.target_format.unwrap_or(frame.format);
        let target_sample_rate = self.target_sample_rate.unwrap_or(frame.sample_rate);
        let target_stereo = self.target_stereo.unwrap_or(frame.stereo);

        if target_sample_rate == 0 {
            return Err(crate::Error::new("invalid target sample rate: 0"));
        }
        if target_format != AudioFormat::I16Be {
            return Err(crate::Error::new(format!(
                "unsupported target audio format: {}",
                target_format
            )));
        }
        if frame.format != AudioFormat::I16Be {
            return Err(crate::Error::new(format!(
                "audio conversion requires I16Be input format, got {}",
                frame.format
            )));
        }

        let mut interleaved = parse_i16be_samples(frame)?;
        let mut stereo = frame.stereo;

        if stereo != target_stereo {
            if !stereo && target_stereo {
                interleaved = crate::audio::mono_to_stereo(&interleaved);
                stereo = true;
            } else {
                return Err(crate::Error::new(
                    "stereo to mono conversion is not supported",
                ));
            }
        }

        if frame.sample_rate != target_sample_rate {
            let key = ResampleStateKey {
                input_sample_rate: frame.sample_rate,
                target_sample_rate,
                stereo,
            };
            if self.state.key != Some(key) {
                self.reset();
                self.state.key = Some(key);
            }

            let resampled = crate::audio::resample(
                &interleaved,
                &self.state.prev_input_samples,
                u32::from(frame.sample_rate),
                self.state.original_samples,
                self.state.resampled_samples,
            )
            .ok_or_else(|| crate::Error::new("audio resample unexpectedly returned none"))?;

            self.state.original_samples += interleaved.len() as u64;
            self.state.resampled_samples += resampled.len() as u64;
            self.state.prev_input_samples = interleaved;
            interleaved = resampled;
        } else {
            self.reset();
        }

        let duration = duration_from_samples(interleaved.len(), target_stereo, target_sample_rate)?;

        Ok(AudioFrame {
            data: interleaved.iter().flat_map(|v| v.to_be_bytes()).collect(),
            format: target_format,
            stereo: target_stereo,
            sample_rate: target_sample_rate,
            timestamp: frame.timestamp,
            duration,
            sample_entry: if target_format == frame.format {
                frame.sample_entry.clone()
            } else {
                None
            },
        })
    }

    pub fn reset(&mut self) {
        self.state = ResampleState::default();
    }
}

fn parse_i16be_samples(frame: &AudioFrame) -> crate::Result<Vec<i16>> {
    if !frame.data.len().is_multiple_of(2) {
        return Err(crate::Error::new("invalid I16Be audio data length"));
    }

    let sample_count = frame.data.len() / 2;
    if frame.stereo && !sample_count.is_multiple_of(2) {
        return Err(crate::Error::new("invalid stereo audio sample count"));
    }

    Ok(frame
        .data
        .chunks_exact(2)
        .map(|chunk| i16::from_be_bytes([chunk[0], chunk[1]]))
        .collect())
}

fn duration_from_samples(
    sample_count: usize,
    stereo: bool,
    sample_rate: u16,
) -> crate::Result<Duration> {
    if sample_rate == 0 {
        return Err(crate::Error::new("invalid sample rate: 0"));
    }

    let samples_per_channel = if stereo {
        if !sample_count.is_multiple_of(2) {
            return Err(crate::Error::new("invalid stereo audio sample count"));
        }
        sample_count / 2
    } else {
        sample_count
    };

    Ok(Duration::from_secs_f64(
        samples_per_channel as f64 / sample_rate as f64,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn i16be(samples: &[i16]) -> Vec<u8> {
        samples.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    fn frame(samples: &[i16], stereo: bool, sample_rate: u16) -> AudioFrame {
        AudioFrame {
            data: i16be(samples),
            format: AudioFormat::I16Be,
            stereo,
            sample_rate,
            timestamp: Duration::from_millis(100),
            duration: Duration::from_millis(20),
            sample_entry: None,
        }
    }

    #[test]
    fn convert_stereo_48khz_keeps_shape() {
        let mut converter = AudioConverterBuilder::new()
            .format(AudioFormat::I16Be)
            .stereo(true)
            .sample_rate(48_000)
            .build();
        let input = frame(&[1, 2, 3, 4], true, 48_000);

        let output = converter.convert(&input).expect("infallible");

        assert_eq!(output.format, AudioFormat::I16Be);
        assert!(output.stereo);
        assert_eq!(output.sample_rate, 48_000);
        assert_eq!(output.data, input.data);
    }

    #[test]
    fn convert_mono_to_stereo() {
        let mut converter = AudioConverterBuilder::new()
            .format(AudioFormat::I16Be)
            .stereo(true)
            .sample_rate(48_000)
            .build();
        let input = frame(&[1, 2, 3], false, 48_000);

        let output = converter.convert(&input).expect("infallible");
        let expected = i16be(&[1, 1, 2, 2, 3, 3]);

        assert!(output.stereo);
        assert_eq!(output.data, expected);
    }

    #[test]
    fn convert_sample_rate_44100_to_48000() {
        let mut converter = AudioConverterBuilder::new()
            .format(AudioFormat::I16Be)
            .stereo(true)
            .sample_rate(48_000)
            .build();
        let input = frame(&[1, 2, 3, 4], true, 44_100);

        let output = converter.convert(&input).expect("infallible");

        assert_eq!(output.sample_rate, 48_000);
        assert!(!output.data.is_empty());
        assert_ne!(output.duration, input.duration);
    }

    #[test]
    fn reset_clears_resample_state() {
        let mut converter = AudioConverterBuilder::new()
            .format(AudioFormat::I16Be)
            .stereo(true)
            .sample_rate(48_000)
            .build();
        let input = frame(&[1, 2, 3, 4], true, 44_100);

        let first = converter.convert(&input).expect("infallible");
        let second = converter.convert(&input).expect("infallible");
        converter.reset();
        let third = converter.convert(&input).expect("infallible");

        assert_ne!(first.data, second.data);
        assert_eq!(first.data, third.data);
    }

    #[test]
    fn reject_stereo_to_mono() {
        let mut converter = AudioConverterBuilder::new()
            .format(AudioFormat::I16Be)
            .stereo(false)
            .sample_rate(48_000)
            .build();
        let input = frame(&[1, 2, 3, 4], true, 48_000);

        let err = converter.convert(&input).expect_err("must fail");
        assert!(err.display().contains("stereo to mono"));
    }
}
