use std::num::NonZeroU8;
use std::time::Duration;

use orfail::OrFail;

use crate::audio::{AudioData, AudioFormat};
use shiguredo_mp4::boxes::SampleEntry;

#[derive(Debug)]
pub struct AudioToolboxDecoder {
    inner: Option<shiguredo_audio_toolbox::Decoder>,
}

impl AudioToolboxDecoder {
    pub fn new() -> orfail::Result<Self> {
        // サンプルレートなどの情報が実際にデータが届くまで不明なので遅延初期化している
        Ok(Self { inner: None })
    }

    pub fn decode(&mut self, data: &AudioData) -> orfail::Result<AudioData> {
        (data.format == AudioFormat::Aac).or_fail()?;

        if self.inner.is_none() {
            let sample_entry = data.sample_entry.as_ref().or_fail()?;
            let (sample_rate, channels) = extract_audio_config(sample_entry)?;
            self.inner =
                Some(shiguredo_audio_toolbox::Decoder::new(sample_rate, channels).or_fail()?);
        }

        let inner = self.inner.as_mut().or_fail()?;
        inner.decode(&data.data).or_fail()?;

        let mut decoded_samples = Vec::new();
        while let Some(samples) = inner.next_decoded_data().or_fail()? {
            decoded_samples.extend(samples);
        }

        let decoded = AudioData {
            source_id: data.source_id.clone(),
            data: decoded_samples
                .iter()
                .flat_map(|v| v.to_be_bytes().into_iter())
                .collect(),
            format: AudioFormat::I16Be,
            stereo: true,
            sample_rate: crate::audio::SAMPLE_RATE,
            timestamp: data.timestamp,
            duration: data.duration,
            sample_entry: None,
        };

        Ok(decoded)
    }

    pub fn finish(&mut self) -> orfail::Result<Option<AudioData>> {
        if let Some(inner) = &mut self.inner {
            inner.finish().or_fail()?;

            let mut decoded_samples = Vec::new();
            while let Some(samples) = inner.next_decoded_data().or_fail()? {
                decoded_samples.extend(samples);
            }

            if decoded_samples.is_empty() {
                return Ok(None);
            }

            let decoded = AudioData {
                source_id: None,
                data: decoded_samples
                    .iter()
                    .flat_map(|v| v.to_be_bytes().into_iter())
                    .collect(),
                format: AudioFormat::I16Be,
                stereo: true,
                sample_rate: crate::audio::SAMPLE_RATE,
                timestamp: Duration::ZERO,
                duration: Duration::from_secs(decoded_samples.len() as u64 / 2)
                    / crate::audio::SAMPLE_RATE as u32,
                sample_entry: None,
            };

            return Ok(Some(decoded));
        }
        Ok(None)
    }
}

fn extract_audio_config(sample_entry: &SampleEntry) -> orfail::Result<(u32, NonZeroU8)> {
    match sample_entry {
        SampleEntry::Mp4a(mp4a) => {
            let sample_rate = mp4a.audio.samplerate.integer as u32;
            let channels = NonZeroU8::new(mp4a.audio.channelcount as u8).or_fail()?;
            Ok((sample_rate, channels))
        }
        _ => Err(orfail::Failure::new("TODO"))?,
    }
}
