use std::num::NonZeroU8;
use std::time::Duration;

use shiguredo_mp4::boxes::SampleEntry;

use crate::audio::{AudioFormat, AudioFrame};

#[derive(Debug)]
pub struct AudioToolboxDecoder {
    inner: Option<shiguredo_audio_toolbox::Decoder>,
    sample_rate: u32,
    channels: u16,
    total_output_samples: u64,
}

impl AudioToolboxDecoder {
    pub fn new() -> crate::Result<Self> {
        // サンプルレートなどの情報が実際にデータが届くまで不明なので遅延初期化している
        Ok(Self {
            inner: None,
            sample_rate: 0, // ダミー値。後でちゃんとした値に更新される
            channels: 0,
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
            if channels.get() > 2 {
                return Err(crate::Error::new(format!(
                    "unsupported AAC channel count: {}",
                    channels.get()
                )));
            }
            self.inner = Some(shiguredo_audio_toolbox::Decoder::new(
                sample_rate,
                channels,
            )?);
            self.sample_rate = sample_rate;
            self.channels = u16::from(channels.get());
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

        if self.sample_rate == 0 {
            return Err(crate::Error::new("audio sample rate is not initialized"));
        }
        if self.channels == 0 {
            return Err(crate::Error::new("audio channel count is not initialized"));
        }
        let sample_rate = u16::try_from(self.sample_rate).map_err(|_| {
            crate::Error::new(format!("unsupported AAC sample rate: {}", self.sample_rate))
        })?;
        if !decoded_samples
            .len()
            .is_multiple_of(usize::from(self.channels))
        {
            return Err(crate::Error::new("invalid decoded audio sample count"));
        }
        let samples_per_channel = decoded_samples.len() / usize::from(self.channels);
        let timestamp =
            Duration::from_secs_f64(self.total_output_samples as f64 / self.sample_rate as f64);
        let duration =
            Duration::from_secs_f64(samples_per_channel as f64 / self.sample_rate as f64);
        self.total_output_samples += samples_per_channel as u64;

        Ok(AudioFrame {
            data: decoded_samples
                .iter()
                .flat_map(|v| v.to_be_bytes())
                .collect(),
            format: AudioFormat::I16Be,
            stereo: self.channels == 2,
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
            let channels = NonZeroU8::new(mp4a.audio.channelcount as u8)
                .ok_or_else(|| crate::Error::new("invalid AAC channel count: 0"))?;
            Ok((sample_rate, channels))
        }
        _ => Err(crate::Error::new(
            "Only MP4a audio sample entries are currently supported",
        )),
    }
}
