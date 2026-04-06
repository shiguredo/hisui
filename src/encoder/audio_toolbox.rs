// shiguredo_audio_toolbox の内部型 (OpaqueAudioConverter) が Send を実装していないため、
// tokio のマルチスレッドランタイム上で直接使用できない。
// そのため専用のネイティブスレッドで Audio Toolbox のエンコード処理を実行し、
// std::sync::mpsc チャネル経由で同期的に通信する。
//
// チャネルには tokio::sync ではなく std::sync::mpsc を使用している。
// エンコード処理は専用のネイティブスレッド上で同期的に実行され、
// 呼び出し側も結果を同期的に待つため、tokio の非同期チャネルを使う必要がない。

use std::{collections::VecDeque, num::NonZeroUsize, time::Duration};

use shiguredo_mp4::{
    Uint,
    boxes::{EsdsBox, Mp4aBox, SampleEntry},
    descriptors::{DecoderConfigDescriptor, DecoderSpecificInfo, EsDescriptor},
};

use crate::audio::{self, AudioFormat, AudioFrame, Channels, SampleRate};

enum EncoderCommand {
    Encode(Vec<i16>),
    Finish,
}

type EncoderInitializationResponse = Result<(), String>;
type EncoderResponse = Result<Vec<shiguredo_audio_toolbox::EncodedFrame>, String>;

#[derive(Debug)]
pub struct AudioToolboxEncoder {
    cmd_tx: std::sync::mpsc::Sender<EncoderCommand>,
    result_rx: std::sync::mpsc::Receiver<EncoderResponse>,
    buffered_frames: VecDeque<shiguredo_audio_toolbox::EncodedFrame>,
    sample_entry: Option<SampleEntry>,
    total_encoded_samples: u64,
}

impl AudioToolboxEncoder {
    pub fn new(bitrate: NonZeroUsize) -> crate::Result<Self> {
        let bitrate_u32 = u32::try_from(bitrate.get())
            .map_err(|_| crate::Error::new("audio encoder bitrate does not fit into u32"))?;

        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<EncoderCommand>();
        let (init_tx, init_rx) = std::sync::mpsc::channel::<EncoderInitializationResponse>();
        let (result_tx, result_rx) = std::sync::mpsc::channel::<EncoderResponse>();

        std::thread::Builder::new()
            .name("audio-toolbox-encoder".into())
            .spawn(move || {
                let encoder =
                    shiguredo_audio_toolbox::Encoder::new(shiguredo_audio_toolbox::EncoderConfig {
                        codec: shiguredo_audio_toolbox::EncoderCodec::AacLc,
                        sample_rate: SampleRate::HZ_48000.get(),
                        channels: Channels::STEREO.get(),
                        bitrate: Some(bitrate_u32),
                        bitrate_control_mode: None,
                        codec_quality: None,
                        vbr_quality: None,
                    });
                let mut encoder = match encoder {
                    Ok(e) => e,
                    Err(e) => {
                        let _ = init_tx.send(Err(e.to_string()));
                        return;
                    }
                };
                if init_tx.send(Ok(())).is_err() {
                    return;
                }

                fn collect_frames(
                    encoder: &mut shiguredo_audio_toolbox::Encoder,
                ) -> Vec<shiguredo_audio_toolbox::EncodedFrame> {
                    let mut frames = Vec::new();
                    while let Some(f) = encoder.next_frame() {
                        frames.push(f);
                    }
                    frames
                }

                while let Ok(cmd) = cmd_rx.recv() {
                    let result = match cmd {
                        EncoderCommand::Encode(samples) => encoder
                            .encode(&samples)
                            .map(|()| collect_frames(&mut encoder))
                            .map_err(|e| e.to_string()),
                        EncoderCommand::Finish => encoder
                            .finish()
                            .map(|()| collect_frames(&mut encoder))
                            .map_err(|e| e.to_string()),
                    };
                    if result_tx.send(result).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| {
                crate::Error::new(format!("failed to spawn audio toolbox encoder thread: {e}"))
            })?;

        init_rx
            .recv()
            .map_err(|_| crate::Error::new("audio toolbox encoder thread has terminated"))?
            .map_err(crate::Error::new)?;

        let sample_entry = Some(sample_entry(bitrate));
        Ok(Self {
            cmd_tx,
            result_rx,
            buffered_frames: VecDeque::new(),
            sample_entry,
            total_encoded_samples: 0,
        })
    }

    pub fn finish(&mut self) -> crate::Result<Option<AudioFrame>> {
        self.send_command(EncoderCommand::Finish)?;
        let frames = self.recv_response()?;
        self.buffered_frames.extend(frames);
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
        self.send_command(EncoderCommand::Encode(input))?;
        let frames = self.recv_response()?;
        self.buffered_frames.extend(frames);
        self.dequeue_encoded_frame()
    }

    fn send_command(&self, cmd: EncoderCommand) -> crate::Result<()> {
        self.cmd_tx
            .send(cmd)
            .map_err(|_| crate::Error::new("audio toolbox encoder thread has terminated"))
    }

    fn recv_response(&self) -> crate::Result<Vec<shiguredo_audio_toolbox::EncodedFrame>> {
        self.result_rx
            .recv()
            .map_err(|_| crate::Error::new("audio toolbox encoder thread has terminated"))?
            .map_err(crate::Error::new)
    }

    fn dequeue_encoded_frame(&mut self) -> crate::Result<Option<AudioFrame>> {
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
