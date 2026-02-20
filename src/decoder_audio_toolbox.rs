use std::num::NonZeroU8;
use std::time::Duration;

use shiguredo_mp4::boxes::SampleEntry;

use crate::audio::{AudioData, AudioFormat, CHANNELS, SAMPLE_RATE};
use crate::metadata::SourceId;

#[derive(Debug)]
pub struct AudioToolboxDecoder {
    inner: Option<shiguredo_audio_toolbox::Decoder>,
    sample_rate: u32,
    source_id: Option<SourceId>,
    original_samples: u64,
    resampled_samples: u64,
    prev_decoded_original_samples: Vec<i16>,
}

impl AudioToolboxDecoder {
    pub fn new() -> crate::Result<Self> {
        // サンプルレートなどの情報が実際にデータが届くまで不明なので遅延初期化している
        Ok(Self {
            inner: None,
            sample_rate: 0, // ダミー値。後でちゃんとした値に更新される
            source_id: None,
            original_samples: 0,
            resampled_samples: 0,
            prev_decoded_original_samples: Vec::new(),
        })
    }

    pub fn decode(&mut self, data: &AudioData) -> crate::Result<AudioData> {
        if data.format != AudioFormat::Aac {
            return Err(crate::Error::new("condition is false"));
        }

        if self.inner.is_none() {
            let sample_entry = data
                .sample_entry
                .as_ref()
                .ok_or_else(|| crate::Error::new("value is missing"))?;
            let (sample_rate, channels) = extract_audio_config(sample_entry)?;
            tracing::debug!(
                "Audio Toolbox AAC decoder configuration: sample_rate={sample_rate}Hz, channels={channels}"
            );
            self.inner = Some(shiguredo_audio_toolbox::Decoder::new(
                sample_rate,
                channels,
            )?);
            self.sample_rate = sample_rate;
            self.source_id = data.source_id.clone();
        }

        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| crate::Error::new("value is missing"))?;
        inner.decode(&data.data)?;

        self.build_audio_data()
    }

    pub fn finish(&mut self) -> crate::Result<Option<AudioData>> {
        let Some(inner) = &mut self.inner else {
            return Ok(None);
        };

        inner.finish()?;

        let audio_data = self.build_audio_data()?;
        if audio_data.data.is_empty() {
            return Ok(None);
        }

        Ok(Some(audio_data))
    }

    /// デコード済みデータをAudioDataに変換する共通処理
    fn build_audio_data(&mut self) -> crate::Result<AudioData> {
        let mut decoded_samples = Vec::new();
        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| crate::Error::new("value is missing"))?;
        while let Some(samples) = inner.next_decoded_data()? {
            decoded_samples.extend(samples);
        }

        let decoded_samples_len = decoded_samples.len() as u64;
        if let Some(resampled) = crate::audio::resample(
            &decoded_samples,
            &self.prev_decoded_original_samples,
            self.sample_rate,
            self.original_samples,
            self.resampled_samples,
        ) {
            self.prev_decoded_original_samples = decoded_samples;
            decoded_samples = resampled;
        } else {
            self.prev_decoded_original_samples = decoded_samples.clone();
        }

        self.original_samples += decoded_samples_len;
        self.resampled_samples += decoded_samples.len() as u64;

        let duration = Duration::from_secs(decoded_samples.len() as u64 / CHANNELS as u64)
            / SAMPLE_RATE as u32;
        let timestamp =
            Duration::from_secs(self.resampled_samples / CHANNELS as u64) / SAMPLE_RATE as u32;

        Ok(AudioData {
            source_id: self.source_id.clone(),
            data: decoded_samples
                .iter()
                .flat_map(|v| v.to_be_bytes())
                .collect(),
            format: AudioFormat::I16Be,
            stereo: true,
            sample_rate: SAMPLE_RATE,
            timestamp,
            duration,
            sample_entry: None,
        })
    }
}

fn extract_audio_config(sample_entry: &SampleEntry) -> crate::Result<(u32, NonZeroU8)> {
    match sample_entry {
        SampleEntry::Mp4a(mp4a) => {
            let sample_rate = mp4a.audio.samplerate.integer as u32;
            let channels = NonZeroU8::new(mp4a.audio.channelcount as u8)
                .ok_or_else(|| crate::Error::new("value is missing"))?;
            Ok((sample_rate, channels))
        }
        _ => Err(crate::Error::new(
            "Only MP4a audio sample entries are currently supported",
        )),
    }
}
