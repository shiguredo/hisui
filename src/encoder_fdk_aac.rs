use std::{num::NonZeroUsize, time::Duration};

use orfail::OrFail;
use shiguredo_mp4::{
    Uint,
    boxes::{EsdsBox, Mp4aBox, SampleEntry},
    descriptors::{DecoderConfigDescriptor, DecoderSpecificInfo, EsDescriptor},
};

use crate::audio::{self, AudioData, AudioFormat, SAMPLE_RATE};

#[derive(Debug)]
pub struct FdkAacEncoder {
    inner: shiguredo_fdk_aac::Encoder,
    sample_entry: Option<SampleEntry>,
    total_encoded_samples: u64,
}

impl FdkAacEncoder {
    pub fn new(bitrate: NonZeroUsize) -> orfail::Result<Self> {
        let config = shiguredo_fdk_aac::EncoderConfig {
            target_bitrate: bitrate.get(),
        };
        let inner = shiguredo_fdk_aac::Encoder::new(config).or_fail()?;
        let sample_entry = Some(sample_entry(&inner, bitrate));
        Ok(Self {
            inner,
            sample_entry,
            total_encoded_samples: 0,
        })
    }

    pub fn finish(&mut self) -> orfail::Result<Option<AudioData>> {
        let Some(encoded) = self.inner.finish().or_fail()? else {
            return Ok(None);
        };
        Ok(Some(self.handle_encoded_frame(encoded)))
    }

    pub fn encode(&mut self, data: &AudioData) -> orfail::Result<Option<AudioData>> {
        (data.format == AudioFormat::I16Be).or_fail()?;
        data.stereo.or_fail()?;

        let input = data
            .interleaved_stereo_samples()
            .or_fail()?
            .collect::<Vec<_>>();
        let Some(encoded) = self.inner.encode(&input).or_fail()? else {
            return Ok(None);
        };
        Ok(Some(self.handle_encoded_frame(encoded)))
    }

    fn handle_encoded_frame(&mut self, encoded: shiguredo_fdk_aac::EncodedFrame) -> AudioData {
        let duration = Duration::from_secs(encoded.samples as u64) / SAMPLE_RATE as u32;
        let timestamp = Duration::from_secs(self.total_encoded_samples) / SAMPLE_RATE as u32;
        self.total_encoded_samples += encoded.samples as u64;

        AudioData {
            // 固定値
            format: AudioFormat::Aac,
            stereo: true,
            sample_rate: SAMPLE_RATE,
            source_id: None,

            // サンプルエントリーは途中で変わらないので、最初に一回だけ載せる
            sample_entry: self.sample_entry.take(),

            // エンコード結果を反映する
            data: encoded.data,
            timestamp,
            duration,
        }
    }
}

fn sample_entry(encoder: &shiguredo_fdk_aac::Encoder, bitrate: NonZeroUsize) -> SampleEntry {
    SampleEntry::Mp4a(Mp4aBox {
        audio: audio::sample_entry_audio_fields(),
        esds_box: EsdsBox {
            es: EsDescriptor {
                es_id: EsDescriptor::MIN_ES_ID,
                stream_priority: EsDescriptor::LOWEST_STREAM_PRIORITY,
                depends_on_es_id: None,
                url_string: None,
                ocr_es_id: None,
                dec_config_descr: DecoderConfigDescriptor {
                    object_type_indication:
                        DecoderConfigDescriptor::OBJECT_TYPE_INDICATION_AUDIO_ISO_IEC_14496_3,
                    stream_type: DecoderConfigDescriptor::STREAM_TYPE_AUDIO,
                    up_stream: DecoderConfigDescriptor::UP_STREAM_FALSE,
                    dec_specific_info: DecoderSpecificInfo {
                        payload: encoder.audio_specific_config().to_vec(),
                    },

                    // 以下は適当にそれっぽい値を指定している
                    buffer_size_db: Uint::new(bitrate.get() as u32 / 8), // 1 秒分のバッファサイズ
                    max_bitrate: bitrate.get() as u32 * 2,               // 平均の 2 倍にしておく
                    avg_bitrate: bitrate.get() as u32,
                },
                sl_config_descr: shiguredo_mp4::descriptors::SlConfigDescriptor,
            },
        },
        unknown_boxes: Vec::new(),
    })
}
