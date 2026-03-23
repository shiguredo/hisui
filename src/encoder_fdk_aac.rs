use std::{num::NonZeroUsize, time::Duration};

use shiguredo_mp4::{
    Uint,
    boxes::{EsdsBox, Mp4aBox, SampleEntry},
    descriptors::{DecoderConfigDescriptor, DecoderSpecificInfo, EsDescriptor},
};

use crate::audio::{self, AudioFormat, AudioFrame, Channels, SampleRate};

#[derive(Debug)]
pub struct FdkAacEncoder {
    inner: shiguredo_fdk_aac::Encoder,
    sample_entry: Option<SampleEntry>,
    total_encoded_samples: u64,
}

impl FdkAacEncoder {
    pub fn new(
        lib: shiguredo_fdk_aac::FdkAacLibrary,
        bitrate: NonZeroUsize,
    ) -> crate::Result<Self> {
        let config = shiguredo_fdk_aac::EncoderConfig {
            sample_rate: 48000,
            channels: 2,
            bitrate: Some(bitrate.get() as u32),
        };
        let inner = shiguredo_fdk_aac::Encoder::new(lib, config)?;
        let sample_entry = Some(sample_entry(&inner, bitrate));
        Ok(Self {
            inner,
            sample_entry,
            total_encoded_samples: 0,
        })
    }

    pub fn finish(&mut self) -> crate::Result<Vec<AudioFrame>> {
        self.inner.finish()?;
        Ok(self.drain_encoded_frames())
    }

    pub fn encode(&mut self, frame: &AudioFrame) -> crate::Result<Vec<AudioFrame>> {
        if frame.format != AudioFormat::I16Be {
            return Err(crate::Error::new(format!(
                "expected I16Be audio format, got {:?}",
                frame.format
            )));
        }
        if !frame.is_stereo() {
            return Err(crate::Error::new(
                "FDK AAC encoder expects stereo audio input",
            ));
        }

        let input = frame.interleaved_stereo_samples()?.collect::<Vec<_>>();
        self.inner.encode(&input)?;
        Ok(self.drain_encoded_frames())
    }

    /// 内部キューに溜まったエンコード済みフレームをすべて回収する
    fn drain_encoded_frames(&mut self) -> Vec<AudioFrame> {
        let mut frames = Vec::new();
        while let Some(encoded) = self.inner.next_frame() {
            frames.push(self.handle_encoded_frame(encoded));
        }
        frames
    }

    fn handle_encoded_frame(&mut self, encoded: shiguredo_fdk_aac::EncodedFrame) -> AudioFrame {
        let timestamp =
            Duration::from_secs(self.total_encoded_samples) / SampleRate::HZ_48000.get();
        self.total_encoded_samples += encoded.samples as u64;

        AudioFrame {
            // 固定値
            format: AudioFormat::Aac,
            channels: Channels::STEREO,
            sample_rate: SampleRate::HZ_48000,

            // サンプルエントリーは途中で変わらないので、最初に一回だけ載せる
            sample_entry: self.sample_entry.take(),

            // エンコード結果を反映する
            data: encoded.data,
            timestamp,
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
                    dec_specific_info: Some(DecoderSpecificInfo {
                        payload: encoder.audio_specific_config().to_vec(),
                    }),

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
