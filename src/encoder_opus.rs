use std::num::NonZeroUsize;

use orfail::OrFail;
use shiguredo_mp4::boxes::{DopsBox, OpusBox, SampleEntry};

use crate::audio::{self, AudioData, AudioFormat, CHANNELS, SAMPLE_RATE};

#[derive(Debug)]
pub struct OpusEncoder {
    inner: shiguredo_opus::Encoder,
    sample_entry: Option<SampleEntry>,
}

impl OpusEncoder {
    pub fn new(bitrate: NonZeroUsize) -> orfail::Result<Self> {
        let inner = shiguredo_opus::Encoder::new(SAMPLE_RATE, CHANNELS as u8, bitrate.get() as u32)
            .or_fail()?;

        // 最初の AudioData に載せるサンプルエントリーを作っておく
        let pre_skip = inner.get_lookahead().or_fail()?;
        let sample_entry = sample_entry(pre_skip);

        Ok(Self {
            inner,
            sample_entry: Some(sample_entry),
        })
    }

    pub fn encode(&mut self, data: &AudioData) -> orfail::Result<AudioData> {
        (data.format == AudioFormat::I16Be).or_fail()?;
        data.stereo.or_fail()?;

        let input = data
            .interleaved_stereo_samples()
            .or_fail()?
            .collect::<Vec<_>>();
        let encoded = self.inner.encode(&input).or_fail()?;

        Ok(AudioData {
            // 固定値
            format: AudioFormat::Opus,
            stereo: true,
            sample_rate: SAMPLE_RATE,

            // 入力の値をそのまま引きつぐ
            source_id: data.source_id.clone(),
            timestamp: data.timestamp,
            duration: data.duration,

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
            output_channel_count: CHANNELS as u8,
            pre_skip,
            input_sample_rate: SAMPLE_RATE as u32,
            output_gain: 0,
        },
        unknown_boxes: Vec::new(),
    })
}
