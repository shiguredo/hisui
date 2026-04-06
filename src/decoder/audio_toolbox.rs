// shiguredo_audio_toolbox の内部型 (OpaqueAudioConverter) が Send を実装していないため、
// tokio のマルチスレッドランタイム上で直接使用できない。
// そのため専用のネイティブスレッドで Audio Toolbox のデコード処理を実行し、
// std::sync::mpsc チャネル経由で同期的に通信する。
//
// チャネルには tokio::sync ではなく std::sync::mpsc を使用している。
// デコード処理は専用のネイティブスレッド上で同期的に実行され、
// 呼び出し側も結果を同期的に待つため、tokio の非同期チャネルを使う必要がない。

use std::num::NonZeroU8;

use shiguredo_mp4::boxes::SampleEntry;

use crate::audio::{AudioFormat, AudioFrame, Channels, SampleRate};
use crate::timestamp::sample_aligner::{DEFAULT_REBASE_THRESHOLD, SampleBasedTimestampAligner};

enum DecoderCommand {
    Decode(Vec<u8>),
    Finish,
}

type DecoderInitializationResponse = Result<(), String>;
type DecoderResponse = Result<Vec<i16>, String>;

/// Audio Toolbox デコーダースレッドのハンドル。
///
/// デコーダーの初期化にはサンプルレート等の情報が必要で、
/// それは最初のフレーム到着時にしか分からないため、スレッドは初回 decode 時に起動する。
#[derive(Debug)]
pub struct AudioToolboxDecoder {
    state: DecoderState,
    sample_rate: Option<SampleRate>,
    channels: Option<Channels>,
    total_output_samples: u64,
    timestamp_aligner: Option<SampleBasedTimestampAligner>,
}

#[derive(Debug)]
enum DecoderState {
    /// デコーダー未初期化（最初のフレーム待ち）
    Uninitialized,
    /// デコーダースレッド稼働中
    Running {
        cmd_tx: std::sync::mpsc::Sender<DecoderCommand>,
        result_rx: std::sync::mpsc::Receiver<DecoderResponse>,
    },
}

impl AudioToolboxDecoder {
    pub fn new() -> crate::Result<Self> {
        // サンプルレートなどの情報が実際にデータが届くまで不明なので遅延初期化している
        Ok(Self {
            state: DecoderState::Uninitialized,
            sample_rate: None,
            channels: None,
            total_output_samples: 0,
            timestamp_aligner: None,
        })
    }

    pub fn decode(&mut self, frame: &AudioFrame) -> crate::Result<AudioFrame> {
        if frame.format != AudioFormat::Aac {
            return Err(crate::Error::new(format!(
                "expected AAC format, got {}",
                frame.format
            )));
        }

        if matches!(self.state, DecoderState::Uninitialized) {
            let sample_entry = frame
                .sample_entry
                .as_ref()
                .ok_or_else(|| crate::Error::new("missing sample entry for AAC decoder"))?;
            self.initialize(sample_entry)?;
        }

        let sample_rate_for_tracking = self.sample_rate.unwrap_or(frame.sample_rate);
        let aligner = self.timestamp_aligner.get_or_insert_with(|| {
            SampleBasedTimestampAligner::new(sample_rate_for_tracking, DEFAULT_REBASE_THRESHOLD)
        });
        // AudioToolbox AAC は初期化時点で sample rate が確定しているため、ここで 1 回だけ設定すればよい。
        aligner.set_sample_rate(sample_rate_for_tracking);
        // AAC は入力と出力が 1 対 1 に対応しないことがあるので、
        // 入力 timestamp は基準オフセットとして扱い、乖離が大きい場合のみ再基準化する。
        aligner.align_input_timestamp(frame.timestamp, self.total_output_samples);

        self.send_command(DecoderCommand::Decode(frame.data.clone()))?;
        let decoded_samples = self.recv_response()?;
        self.build_audio_frame(decoded_samples)
    }

    pub fn finish(&mut self) -> crate::Result<Option<AudioFrame>> {
        if matches!(self.state, DecoderState::Uninitialized) {
            return Ok(None);
        }

        self.send_command(DecoderCommand::Finish)?;
        let decoded_samples = self.recv_response()?;
        if decoded_samples.is_empty() {
            return Ok(None);
        }

        self.build_audio_frame(decoded_samples).map(Some)
    }

    fn initialize(&mut self, sample_entry: &SampleEntry) -> crate::Result<()> {
        let (raw_sample_rate, channels) = extract_audio_config(sample_entry)?;
        tracing::debug!(
            "Audio Toolbox AAC decoder configuration: sample_rate={raw_sample_rate}Hz, channels={channels}"
        );
        let input_channel_layout = Channels::from_u8(channels.get())?;
        let sample_rate = SampleRate::from_u32(raw_sample_rate)?;

        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<DecoderCommand>();
        let (init_tx, init_rx) = std::sync::mpsc::channel::<DecoderInitializationResponse>();
        let (result_tx, result_rx) = std::sync::mpsc::channel::<DecoderResponse>();

        std::thread::Builder::new()
            .name("audio-toolbox-decoder".into())
            .spawn(move || {
                let decoder =
                    shiguredo_audio_toolbox::Decoder::new(shiguredo_audio_toolbox::DecoderConfig {
                        codec: shiguredo_audio_toolbox::DecoderCodec::AacLc,
                        input_sample_rate: raw_sample_rate,
                        input_channels: input_channel_layout.get(),
                    });
                let mut decoder = match decoder {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = init_tx.send(Err(e.to_string()));
                        return;
                    }
                };
                if init_tx.send(Ok(())).is_err() {
                    return;
                }

                fn collect_samples(
                    decoder: &mut shiguredo_audio_toolbox::Decoder,
                ) -> Result<Vec<i16>, String> {
                    let mut all_samples = Vec::new();
                    while let Some(samples) = decoder.next_frame().map_err(|e| e.to_string())? {
                        all_samples.extend(samples);
                    }
                    Ok(all_samples)
                }

                while let Ok(cmd) = cmd_rx.recv() {
                    let result = match cmd {
                        DecoderCommand::Decode(data) => decoder
                            .decode(&data)
                            .map_err(|e| e.to_string())
                            .and_then(|()| collect_samples(&mut decoder)),
                        DecoderCommand::Finish => decoder
                            .finish()
                            .map_err(|e| e.to_string())
                            .and_then(|()| collect_samples(&mut decoder)),
                    };
                    if result_tx.send(result).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| {
                crate::Error::new(format!("failed to spawn audio toolbox decoder thread: {e}"))
            })?;

        init_rx
            .recv()
            .map_err(|_| crate::Error::new("audio toolbox decoder thread has terminated"))?
            .map_err(crate::Error::new)?;

        self.sample_rate = Some(sample_rate);
        // Audio Toolbox デコーダーの出力は常にステレオになる。
        self.channels = Some(Channels::STEREO);
        self.state = DecoderState::Running { cmd_tx, result_rx };
        Ok(())
    }

    fn send_command(&self, cmd: DecoderCommand) -> crate::Result<()> {
        let DecoderState::Running { ref cmd_tx, .. } = self.state else {
            return Err(crate::Error::new(
                "audio toolbox decoder is not initialized",
            ));
        };
        cmd_tx
            .send(cmd)
            .map_err(|_| crate::Error::new("audio toolbox decoder thread has terminated"))
    }

    fn recv_response(&self) -> crate::Result<Vec<i16>> {
        let DecoderState::Running { ref result_rx, .. } = self.state else {
            return Err(crate::Error::new(
                "audio toolbox decoder is not initialized",
            ));
        };
        result_rx
            .recv()
            .map_err(|_| crate::Error::new("audio toolbox decoder thread has terminated"))?
            .map_err(crate::Error::new)
    }

    /// デコード済みデータを AudioFrame に変換する共通処理
    fn build_audio_frame(&mut self, decoded_samples: Vec<i16>) -> crate::Result<AudioFrame> {
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
        // AAC は内部バッファリングで出力タイミングが揺れるので、timestamp は sample 数基準で生成する。
        let timestamp = self
            .timestamp_aligner
            .as_ref()
            .expect("timestamp aligner must be initialized before decoding")
            .estimate_timestamp_from_output_samples(self.total_output_samples);
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
