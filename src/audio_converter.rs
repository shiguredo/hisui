use std::time::Duration;

use crate::audio::{AudioFormat, AudioFrame, Channels, SampleRate};

#[derive(Debug, Clone, Default)]
pub struct AudioConverterBuilder {
    target_format: Option<AudioFormat>,
    target_sample_rate: Option<SampleRate>,
    target_channels: Option<Channels>,
}

impl AudioConverterBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn format(mut self, format: AudioFormat) -> Self {
        self.target_format = Some(format);
        self
    }

    pub fn sample_rate(mut self, sample_rate: SampleRate) -> Self {
        self.target_sample_rate = Some(sample_rate);
        self
    }

    pub fn channels(mut self, channels: Channels) -> Self {
        self.target_channels = Some(channels);
        self
    }

    pub fn build(self) -> AudioConverter {
        AudioConverter {
            target_format: self.target_format,
            target_sample_rate: self.target_sample_rate,
            target_channels: self.target_channels,
            state: ResampleState::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResampleStateKey {
    input_sample_rate: SampleRate,
    target_sample_rate: SampleRate,
    channels: Channels,
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
    target_sample_rate: Option<SampleRate>,
    target_channels: Option<Channels>,
    state: ResampleState,
}

impl AudioConverter {
    pub fn convert(&mut self, frame: &AudioFrame) -> crate::Result<AudioFrame> {
        let target_format = self.target_format.unwrap_or(frame.format);
        let target_sample_rate = self.target_sample_rate.unwrap_or(frame.sample_rate);
        let target_channels = self.target_channels.unwrap_or(frame.channels);

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
        let mut channels = frame.channels;

        if channels != target_channels {
            if channels.is_mono() && target_channels.is_stereo() {
                interleaved = crate::audio::mono_to_stereo(&interleaved);
                channels = Channels::STEREO;
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
                channels,
            };
            if self.state.key != Some(key) {
                self.reset();
                self.state.key = Some(key);
            }

            let resampled = crate::audio::resample(
                &interleaved,
                &self.state.prev_input_samples,
                frame.sample_rate,
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

        let duration =
            duration_from_samples(interleaved.len(), target_channels, target_sample_rate)?;

        Ok(AudioFrame {
            data: interleaved.iter().flat_map(|v| v.to_be_bytes()).collect(),
            format: target_format,
            channels: target_channels,
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
    if frame.channels.is_stereo() && !sample_count.is_multiple_of(2) {
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
    channels: Channels,
    sample_rate: SampleRate,
) -> crate::Result<Duration> {
    let samples_per_channel = if channels.is_stereo() {
        if !sample_count.is_multiple_of(2) {
            return Err(crate::Error::new("invalid stereo audio sample count"));
        }
        sample_count / 2
    } else {
        sample_count
    };

    Ok(sample_rate.duration_from_samples(samples_per_channel as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn i16be(samples: &[i16]) -> Vec<u8> {
        samples.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    fn frame(samples: &[i16], channels: Channels, sample_rate: SampleRate) -> AudioFrame {
        AudioFrame {
            data: i16be(samples),
            format: AudioFormat::I16Be,
            channels,
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
            .channels(Channels::STEREO)
            .sample_rate(SampleRate::from_u32(48_000).expect("must be valid"))
            .build();
        let input = frame(
            &[1, 2, 3, 4],
            Channels::STEREO,
            SampleRate::from_u32(48_000).expect("must be valid"),
        );

        let output = converter.convert(&input).expect("infallible");

        assert_eq!(output.format, AudioFormat::I16Be);
        assert!(output.channels.is_stereo());
        assert_eq!(output.sample_rate.get(), 48_000);
        assert_eq!(output.data, input.data);
    }

    #[test]
    fn convert_mono_to_stereo() {
        let mut converter = AudioConverterBuilder::new()
            .format(AudioFormat::I16Be)
            .channels(Channels::STEREO)
            .sample_rate(SampleRate::from_u32(48_000).expect("must be valid"))
            .build();
        let input = frame(
            &[1, 2, 3],
            Channels::MONO,
            SampleRate::from_u32(48_000).expect("must be valid"),
        );

        let output = converter.convert(&input).expect("infallible");
        let expected = i16be(&[1, 1, 2, 2, 3, 3]);

        assert!(output.channels.is_stereo());
        assert_eq!(output.data, expected);
    }

    #[test]
    fn convert_sample_rate_44100_to_48000() {
        let mut converter = AudioConverterBuilder::new()
            .format(AudioFormat::I16Be)
            .channels(Channels::STEREO)
            .sample_rate(SampleRate::from_u32(48_000).expect("must be valid"))
            .build();
        let input = frame(
            &[1, 2, 3, 4],
            Channels::STEREO,
            SampleRate::from_u32(44_100).expect("must be valid"),
        );

        let output = converter.convert(&input).expect("infallible");

        assert_eq!(output.sample_rate.get(), 48_000);
        assert!(!output.data.is_empty());
        assert_ne!(output.duration, input.duration);
    }

    #[test]
    fn reset_clears_resample_state() {
        let mut converter = AudioConverterBuilder::new()
            .format(AudioFormat::I16Be)
            .channels(Channels::STEREO)
            .sample_rate(SampleRate::from_u32(48_000).expect("must be valid"))
            .build();
        let input = frame(
            &[1, 2, 3, 4],
            Channels::STEREO,
            SampleRate::from_u32(44_100).expect("must be valid"),
        );

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
            .channels(Channels::MONO)
            .sample_rate(SampleRate::from_u32(48_000).expect("must be valid"))
            .build();
        let input = frame(
            &[1, 2, 3, 4],
            Channels::STEREO,
            SampleRate::from_u32(48_000).expect("must be valid"),
        );

        let err = converter.convert(&input).expect_err("must fail");
        assert!(err.display().contains("stereo to mono"));
    }
}
