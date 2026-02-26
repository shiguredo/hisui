use std::num::NonZeroU8;

use shiguredo_mp4::boxes::SampleEntry;

use crate::audio::{AudioFormat, AudioFrame, Channels, SampleRate};

#[derive(Debug)]
pub struct AudioToolboxDecoder {
    inner: Option<shiguredo_audio_toolbox::Decoder>,
    sample_rate: Option<SampleRate>,
    channels: Option<Channels>,
    total_output_samples: u64,
}

impl AudioToolboxDecoder {
    pub fn new() -> crate::Result<Self> {
        // サンプルレートなどの情報が実際にデータが届くまで不明なので遅延初期化している
        Ok(Self {
            inner: None,
            sample_rate: None,
            channels: None,
            total_output_samples: 0,
        })
    }

    pub fn decode(&mut self, frame: &AudioFrame) -> crate::Result<AudioFrame> {
        if frame.format != AudioFormat::Aac {
            return Err(crate::Error::new(format!(
                "expected AAC format, got {}",
                frame.format
            )));
        }

        if self.inner.is_none() {
            let sample_entry = frame
                .sample_entry
                .as_ref()
                .ok_or_else(|| crate::Error::new("missing sample entry for AAC decoder"))?;
            let (sample_rate, channels) = extract_audio_config(sample_entry)?;
            tracing::debug!(
                "Audio Toolbox AAC decoder configuration: sample_rate={sample_rate}Hz, channels={channels}"
            );
            let channel_layout = Channels::from_u8(channels.get())?;
            self.inner = Some(shiguredo_audio_toolbox::Decoder::new(
                sample_rate,
                channels,
            )?);
            self.sample_rate = Some(SampleRate::from_u32(sample_rate)?);
            self.channels = Some(channel_layout);
        }

        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| crate::Error::new("audio toolbox decoder is not initialized"))?;
        inner.decode(&frame.data)?;

        self.build_audio_frame()
    }

    pub fn finish(&mut self) -> crate::Result<Option<AudioFrame>> {
        let Some(inner) = &mut self.inner else {
            return Ok(None);
        };

        inner.finish()?;

        let frame = self.build_audio_frame()?;
        if frame.data.is_empty() {
            return Ok(None);
        }

        Ok(Some(frame))
    }

    /// デコード済みデータを AudioFrame に変換する共通処理
    fn build_audio_frame(&mut self) -> crate::Result<AudioFrame> {
        let mut decoded_samples = Vec::new();
        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| crate::Error::new("audio toolbox decoder is not initialized"))?;
        while let Some(samples) = inner.next_decoded_data()? {
            decoded_samples.extend(samples);
        }

        let sample_rate = self
            .sample_rate
            .ok_or_else(|| crate::Error::new("audio sample rate is not initialized"))?;
        let channels = self
            .channels
            .ok_or_else(|| crate::Error::new("audio channel count is not initialized"))?;
        if !decoded_samples
            .len()
            .is_multiple_of(usize::from(channels.get()))
        {
            return Err(crate::Error::new("invalid decoded audio sample count"));
        }
        let samples_per_channel = decoded_samples.len() / usize::from(channels.get());
        let timestamp = sample_rate.duration_from_samples(self.total_output_samples);
        let duration = sample_rate.duration_from_samples(samples_per_channel as u64);
        self.total_output_samples += samples_per_channel as u64;

        Ok(AudioFrame {
            data: decoded_samples
                .iter()
                .flat_map(|v| v.to_be_bytes())
                .collect(),
            format: AudioFormat::I16Be,
            channels,
            sample_rate,
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
            let channel_count = u8::try_from(mp4a.audio.channelcount).map_err(|_| {
                crate::Error::new(format!(
                    "unsupported AAC channel count: {}",
                    mp4a.audio.channelcount
                ))
            })?;
            let channels = NonZeroU8::new(channel_count)
                .ok_or_else(|| crate::Error::new("invalid AAC channel count: 0"))?;
            Ok((sample_rate, channels))
        }
        _ => Err(crate::Error::new(
            "Only MP4a audio sample entries are currently supported",
        )),
    }
}
