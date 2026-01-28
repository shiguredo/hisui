use std::num::NonZeroU8;
use std::time::Duration;

use orfail::OrFail;
use shiguredo_mp4::boxes::SampleEntry;

use crate::audio::{AudioData, AudioFormat, CHANNELS, SAMPLE_RATE};
use crate::metadata::SourceId;

#[derive(Debug)]
pub struct AudioToolboxDecoder {
    inner: Option<shiguredo_audio_toolbox::Decoder>,
    sample_rate: u32,
    timestamp: Duration,
    source_id: Option<SourceId>,
}

impl AudioToolboxDecoder {
    pub fn new() -> orfail::Result<Self> {
        // サンプルレートなどの情報が実際にデータが届くまで不明なので遅延初期化している
        Ok(Self {
            inner: None,
            sample_rate: 0, // ダミー値。後でちゃんとした値に更新される
            timestamp: Duration::ZERO,
            source_id: None,
        })
    }

    pub fn decode(&mut self, data: &AudioData) -> orfail::Result<AudioData> {
        (data.format == AudioFormat::Aac).or_fail()?;

        if self.inner.is_none() {
            let sample_entry = data.sample_entry.as_ref().or_fail()?;
            let (sample_rate, channels) = extract_audio_config(sample_entry)?;
            log::debug!(
                "Audio Toolbox AAC decoder configuration: sample_rate={sample_rate}Hz, channels={channels}"
            );
            self.inner =
                Some(shiguredo_audio_toolbox::Decoder::new(sample_rate, channels).or_fail()?);
            self.sample_rate = sample_rate;
            self.source_id = data.source_id.clone();
        }

        let inner = self.inner.as_mut().or_fail()?;
        inner.decode(&data.data).or_fail()?;

        self.build_audio_data()
    }

    pub fn finish(&mut self) -> orfail::Result<Option<AudioData>> {
        let Some(inner) = &mut self.inner else {
            return Ok(None);
        };

        inner.finish().or_fail()?;

        let audio_data = self.build_audio_data()?;
        if audio_data.data.is_empty() {
            return Ok(None);
        }

        Ok(Some(audio_data))
    }

    /// デコード済みデータをAudioDataに変換する共通処理
    fn build_audio_data(&mut self) -> orfail::Result<AudioData> {
        let mut decoded_samples = Vec::new();
        while let Some(samples) = self
            .inner
            .as_mut()
            .or_fail()?
            .next_decoded_data()
            .or_fail()?
        {
            decoded_samples.extend(samples);
        }

        let duration = Duration::from_secs(decoded_samples.len() as u64 / CHANNELS as u64)
            / SAMPLE_RATE as u32;
        let timestamp = self.timestamp;
        self.timestamp += duration;

        Ok(AudioData {
            source_id: self.source_id.clone(),
            data: decoded_samples
                .iter()
                .flat_map(|v| v.to_be_bytes().into_iter())
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

fn extract_audio_config(sample_entry: &SampleEntry) -> orfail::Result<(u32, NonZeroU8)> {
    match sample_entry {
        SampleEntry::Mp4a(mp4a) => {
            let sample_rate = mp4a.audio.samplerate.integer as u32;
            let channels = NonZeroU8::new(mp4a.audio.channelcount as u8).or_fail()?;
            Ok((sample_rate, channels))
        }
        _ => Err(orfail::Failure::new(
            "Only MP4a audio sample entries are currently supported",
        ))?,
    }
}
