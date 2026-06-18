use std::num::NonZeroUsize;

use crate::{
    audio::{AudioFormat, AudioFrame, Channels, SampleRate, opus::opus_sample_entry},
    sample_entry::SharedSampleEntry,
};

#[derive(Debug)]
pub struct OpusEncoder {
    inner: shiguredo_opus::Encoder,
    sample_entry: SharedSampleEntry,
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

        // 出力フレームに載せるサンプルエントリーを作っておく
        let pre_skip = inner.get_lookahead()?;
        let sample_entry = opus_sample_entry(pre_skip);

        Ok(Self {
            inner,
            sample_entry: SharedSampleEntry::new(sample_entry),
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

            sample_entry: Some(self.sample_entry.clone()),

            // エンコード結果を反映する
            data: encoded.to_vec(),
        })
    }
}
