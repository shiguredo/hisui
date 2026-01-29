use std::time::Duration;

use orfail::OrFail;
use shiguredo_mp4::boxes::SampleEntry;

use crate::audio::{AudioData, AudioFormat, CHANNELS, SAMPLE_RATE};
use crate::metadata::SourceId;

/// FDK AAC デコーダー
#[derive(Debug)]
pub struct FdkAacDecoder {
    inner: Option<shiguredo_fdk_aac::Decoder>,
    sample_rate: u32,
    source_id: Option<SourceId>,
    original_samples: u64,
    resampled_samples: u64,
    prev_decoded_original_samples: Vec<i16>,
}

impl FdkAacDecoder {
    /// デコーダーインスタンスを生成する
    pub fn new() -> orfail::Result<Self> {
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

    /// AAC データをデコードする
    pub fn decode(&mut self, data: &AudioData) -> orfail::Result<AudioData> {
        (data.format == AudioFormat::Aac).or_fail()?;

        if self.inner.is_none() {
            let sample_entry = data.sample_entry.as_ref().or_fail()?;
            let audio_specific_config = extract_audio_specific_config(sample_entry)?;
            log::debug!(
                "FDK AAC decoder initialized with config length: {}",
                audio_specific_config.len()
            );
            self.inner = Some(
                shiguredo_fdk_aac::Decoder::new(&audio_specific_config).map_err(|e| {
                    orfail::Failure::new(format!("Failed to create FDK AAC decoder: {}", e))
                })?,
            );
            self.source_id = data.source_id.clone();
        }

        let inner = self.inner.as_mut().or_fail()?;
        let decoded_frame = inner
            .decode(&data.data)
            .map_err(|e| orfail::Failure::new(format!("Failed to decode AAC: {}", e)))?;

        if let Some(frame) = decoded_frame {
            // いったんステレオチャネル以外は非対応にする
            (frame.channels == 2).or_fail_with(|()| format!("TODO"))?;

            self.sample_rate = frame.sample_rate;
            self.build_audio_data(&frame.data)
        } else {
            // デコード可能なフレームがない場合は空のデータを返す
            Ok(AudioData {
                source_id: self.source_id.clone(),
                data: Vec::new(),
                format: AudioFormat::I16Be,
                stereo: true,
                sample_rate: SAMPLE_RATE,
                timestamp: Duration::from_secs(0),
                duration: Duration::from_secs(0),
                sample_entry: None,
            })
        }
    }

    /// デコード済みデータを AudioData に変換する共通処理
    fn build_audio_data(&mut self, decoded_samples: &[i16]) -> orfail::Result<AudioData> {
        let decoded_samples_len = decoded_samples.len() as u64;

        let resampled = if let Some(resampled) = crate::audio::resample(
            decoded_samples,
            &self.prev_decoded_original_samples,
            self.sample_rate,
            self.original_samples,
            self.resampled_samples,
        ) {
            self.prev_decoded_original_samples = decoded_samples.to_vec();
            resampled
        } else {
            self.prev_decoded_original_samples = decoded_samples.to_vec();
            decoded_samples.to_vec()
        };

        self.original_samples += decoded_samples_len;
        self.resampled_samples += resampled.len() as u64;

        let duration =
            Duration::from_secs(resampled.len() as u64 / CHANNELS as u64) / SAMPLE_RATE as u32;
        let timestamp =
            Duration::from_secs(self.resampled_samples / CHANNELS as u64) / SAMPLE_RATE as u32;

        Ok(AudioData {
            source_id: self.source_id.clone(),
            data: resampled.iter().flat_map(|v| v.to_be_bytes()).collect(),
            format: AudioFormat::I16Be,
            stereo: true,
            sample_rate: SAMPLE_RATE,
            timestamp,
            duration,
            sample_entry: None,
        })
    }
}

/// SampleEntry から Audio Specific Config を抽出する
fn extract_audio_specific_config(sample_entry: &SampleEntry) -> orfail::Result<Vec<u8>> {
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
                .or_fail()?
                .payload
                .clone())
        }
        _ => Err(orfail::Failure::new(
            "Only MP4a audio sample entries are currently supported",
        ))?,
    }
}
