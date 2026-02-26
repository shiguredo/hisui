use std::time::Duration;

use shiguredo_mp4::boxes::SampleEntry;

use crate::audio::{AudioFormat, AudioFrame, Channels};

/// FDK AAC デコーダー
#[derive(Debug)]
pub struct FdkAacDecoder {
    inner: Option<shiguredo_fdk_aac::Decoder>,
    sample_rate: u32,
    channels: Option<Channels>,
    total_output_samples: u64,
}

impl FdkAacDecoder {
    /// デコーダーインスタンスを生成する
    pub fn new() -> crate::Result<Self> {
        // サンプルレートなどの情報が実際にデータが届くまで不明なので遅延初期化している
        Ok(Self {
            inner: None,
            sample_rate: 0, // ダミー値。後でちゃんとした値に更新される
            channels: None,
            total_output_samples: 0,
        })
    }

    /// AAC データをデコードする
    pub fn decode(&mut self, frame: &AudioFrame) -> crate::Result<AudioFrame> {
        if frame.format != AudioFormat::Aac {
            return Err(crate::Error::new(format!(
                "expected AAC audio format, got {:?}",
                frame.format
            )));
        }

        if self.inner.is_none() {
            let sample_entry = frame.sample_entry.as_ref().ok_or_else(|| {
                crate::Error::new("AAC sample entry is required to initialize FDK AAC decoder")
            })?;
            let audio_specific_config = extract_audio_specific_config(sample_entry)?;
            tracing::debug!(
                "FDK AAC decoder initialized with config length: {}",
                audio_specific_config.len()
            );
            self.inner = Some(
                shiguredo_fdk_aac::Decoder::new(&audio_specific_config).map_err(|e| {
                    crate::Error::from(e).with_context("Failed to create FDK AAC decoder")
                })?,
            );
        }

        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| crate::Error::new("FDK AAC decoder is not initialized"))?;
        let decoded_frame = inner
            .decode(&frame.data)
            .map_err(|e| crate::Error::from(e).with_context("Failed to decode AAC"))?;

        if let Some(decoded) = decoded_frame {
            self.sample_rate = decoded.sample_rate;
            self.channels = Some(Channels::from_u8(decoded.channels)?);
            self.build_audio_frame(&decoded.data)
        } else {
            // デコード可能なフレームがない場合は空のデータを返す
            //
            // TODO: そもそも将来的には decoder.rs のインタフェースを見直して、このようなワークアラウンドを不要にする
            let timestamp = if self.sample_rate == 0 {
                Duration::ZERO
            } else {
                Duration::from_secs_f64(self.total_output_samples as f64 / self.sample_rate as f64)
            };
            Ok(AudioFrame {
                data: Vec::new(),
                format: AudioFormat::I16Be,
                channels: self.channels.unwrap_or(Channels::STEREO),
                sample_rate: if self.sample_rate == 0 {
                    frame.sample_rate
                } else {
                    u16::try_from(self.sample_rate).map_err(|_| {
                        crate::Error::new(format!(
                            "unsupported AAC sample rate: {}",
                            self.sample_rate
                        ))
                    })?
                },
                timestamp,
                duration: Duration::from_secs(0),
                sample_entry: None,
            })
        }
    }

    /// デコード済みデータを AudioFrame に変換する共通処理
    fn build_audio_frame(&mut self, decoded_samples: &[i16]) -> crate::Result<AudioFrame> {
        if self.sample_rate == 0 {
            return Err(crate::Error::new("audio sample rate is not initialized"));
        }
        let channels = self
            .channels
            .ok_or_else(|| crate::Error::new("audio channel count is not initialized"))?;
        if !decoded_samples
            .len()
            .is_multiple_of(usize::from(channels.get()))
        {
            return Err(crate::Error::new("invalid decoded audio sample count"));
        }
        let sample_rate = u16::try_from(self.sample_rate).map_err(|_| {
            crate::Error::new(format!("unsupported AAC sample rate: {}", self.sample_rate))
        })?;
        let samples_per_channel = decoded_samples.len() / usize::from(channels.get());
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
            channels,
            sample_rate,
            timestamp,
            duration,
            sample_entry: None,
        })
    }
}

/// SampleEntry から Audio Specific Config を抽出する
fn extract_audio_specific_config(sample_entry: &SampleEntry) -> crate::Result<Vec<u8>> {
    match sample_entry {
        SampleEntry::Mp4a(mp4a) => {
            // esds (Elementary Stream Descriptor) ボックスから Audio Specific Config を取得
            let esds = &mp4a.esds_box;
            // ESDS ボックスの構造は複雑だが、ここでは Audio Specific Config を直接取得
            // 通常、AudioSpecificConfig はデコーダースペシフィック情報として保存される
            Ok(esds
                .es
                .dec_config_descr
                .dec_specific_info
                .as_ref()
                .ok_or_else(|| {
                    crate::Error::new("AudioSpecificConfig is missing in MP4a sample entry")
                })?
                .payload
                .clone())
        }
        _ => Err(crate::Error::new(
            "Only MP4a audio sample entries are currently supported",
        )),
    }
}
