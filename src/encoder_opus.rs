use std::num::NonZeroUsize;

use shiguredo_mp4::boxes::{DopsBox, OpusBox, SampleEntry};

use crate::audio::{self, AudioFormat, AudioFrame, Channels, SampleRate};

#[derive(Debug)]
pub struct OpusEncoder {
    inner: shiguredo_opus::Encoder,
    sample_entry: Option<SampleEntry>,
}

impl OpusEncoder {
    pub fn new(bitrate: NonZeroUsize) -> crate::Result<Self> {
        let config = shiguredo_opus::EncoderConfig {
            bitrate: Some(bitrate.get() as u32),
            ..shiguredo_opus::EncoderConfig::new(
                u32::from(SampleRate::HZ_48000.as_u16()?),
                Channels::STEREO.get(),
            )
        };
        let inner = shiguredo_opus::Encoder::new(config)?;

        // 最初の AudioFrame に載せるサンプルエントリーを作っておく
        let pre_skip = inner.get_lookahead()?;
        let sample_entry = sample_entry(pre_skip);

        Ok(Self {
            inner,
            sample_entry: Some(sample_entry),
        })
    }

    pub fn encode(&mut self, frame: &AudioFrame) -> crate::Result<AudioFrame> {
        if frame.format != AudioFormat::I16Be {
            return Err(crate::Error::new(format!(
                "expected I16Be format, got {}",
                frame.format
            )));
        }
        if !frame.is_stereo() {
            return Err(crate::Error::new("expected stereo audio data"));
        }

        let input = frame.interleaved_stereo_samples()?.collect::<Vec<_>>();
        let encoded = self.inner.encode(&input)?;

        Ok(AudioFrame {
            // 固定値
            format: AudioFormat::Opus,
            channels: Channels::STEREO,
            sample_rate: SampleRate::HZ_48000,

            // 入力の値をそのまま引きつぐ
            timestamp: frame.timestamp,

            // サンプルエントリーは途中で変わらないので、最初に一回だけ載せる
            sample_entry: self.sample_entry.take(),

            // エンコード結果を反映する
            data: encoded.to_vec(),
        })
    }
}

fn sample_entry(pre_skip: u16) -> SampleEntry {
    SampleEntry::Opus(OpusBox {
        audio: audio::sample_entry_audio_fields(),
        dops_box: DopsBox {
            output_channel_count: Channels::STEREO.get(),
            pre_skip,
            input_sample_rate: SampleRate::HZ_48000.get(),
            output_gain: 0,
        },
        unknown_boxes: Vec::new(),
    })
}
