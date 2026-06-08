use std::num::NonZeroUsize;

use shiguredo_mp4::boxes::{DopsBox, OpusBox, SampleEntry};

use crate::{
    audio::{self, AudioFormat, AudioFrame, Channels, SampleRate},
    sample_entry::SharedSampleEntry,
};

#[derive(Debug)]
pub struct OpusEncoder {
    inner: shiguredo_opus::Encoder,
    // 全出力フレームに載せる sample entry。Arc 共有なので毎フレームの clone は安価。
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

        // 全出力フレームに載せるサンプルエントリーを作っておく
        let pre_skip = inner.get_lookahead()?;
        let sample_entry = sample_entry(pre_skip);

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

            // 全出力フレームに sample entry を載せる。Arc 共有なので clone は安価。
            // 「最初の 1 フレームだけ載せる」方式だと、writer が最初の entry 付き
            // フレームを取りこぼした際に entry が一度も届かず finalize に失敗するため。
            sample_entry: Some(self.sample_entry.clone()),

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
