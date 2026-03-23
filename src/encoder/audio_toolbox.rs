use std::{collections::VecDeque, num::NonZeroUsize, time::Duration};

use shiguredo_mp4::{
    Uint,
    boxes::{EsdsBox, Mp4aBox, SampleEntry},
    descriptors::{DecoderConfigDescriptor, DecoderSpecificInfo, EsDescriptor},
};

use crate::audio::{self, AudioFormat, AudioFrame, Channels, SampleRate};

#[derive(Debug)]
pub struct AudioToolboxEncoder {
    inner: shiguredo_audio_toolbox::Encoder,
    buffered_frames: VecDeque<shiguredo_audio_toolbox::EncodedFrame>,
    sample_entry: Option<SampleEntry>,
    total_encoded_samples: u64,
}

impl AudioToolboxEncoder {
    pub fn new(bitrate: NonZeroUsize) -> crate::Result<Self> {
        let bitrate_u32 = u32::try_from(bitrate.get())
            .map_err(|_| crate::Error::new("audio encoder bitrate does not fit into u32"))?;
        let inner =
            shiguredo_audio_toolbox::Encoder::new(shiguredo_audio_toolbox::EncoderConfig {
                codec: shiguredo_audio_toolbox::EncoderCodec::AacLc,
                sample_rate: SampleRate::HZ_48000.get(),
                channels: Channels::STEREO.get(),
                bitrate: Some(bitrate_u32),
                bitrate_control_mode: None,
                codec_quality: None,
                vbr_quality: None,
            })?;
        let sample_entry = Some(sample_entry(bitrate));
        Ok(Self {
            inner,
            buffered_frames: VecDeque::new(),
            sample_entry,
            total_encoded_samples: 0,
        })
    }

    pub fn finish(&mut self) -> crate::Result<Option<AudioFrame>> {
        self.inner.finish()?;
        self.dequeue_encoded_frame()
    }

    pub fn encode(&mut self, frame: &AudioFrame) -> crate::Result<Option<AudioFrame>> {
        if frame.format != AudioFormat::I16Be {
            return Err(crate::Error::new(format!(
                "expected I16Be format, got {}",
                frame.format
            )));
        }
        if !frame.is_stereo() {
            return Err(crate::Error::new("expected stereo audio data"));
        }

        let input = frame.interleaved_stereo_samples()?.collect::<Vec<_>>();
        self.inner.encode(&input)?;
        self.dequeue_encoded_frame()
    }

    fn dequeue_encoded_frame(&mut self) -> crate::Result<Option<AudioFrame>> {
        if self.buffered_frames.is_empty() {
            while let Some(encoded) = self.inner.next_frame() {
                self.buffered_frames.push_back(encoded);
            }
        }

        let Some(encoded) = self.buffered_frames.pop_front() else {
            return Ok(None);
        };
        Ok(Some(self.handle_encoded_frame(encoded)))
    }

    fn handle_encoded_frame(
        &mut self,
        encoded: shiguredo_audio_toolbox::EncodedFrame,
    ) -> AudioFrame {
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

fn sample_entry(bitrate: NonZeroUsize) -> SampleEntry {
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
                        // AAC LC, 48kHz, stereo 用の配列 (ISO_IEC_14496-3)
                        // - 最初の 5 bit: 0b00010 (AAC LC)
                        // - 次の 4 bit: 0b0011 (48kHz を意味する値)
                        // - 次の 4 bit: 0b0010 (ステレオを意味する値)
                        // - 最後の 3 bit: 未使用
                        payload: vec![0x11, 0x90],
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
