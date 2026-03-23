use crate::audio::{AudioFormat, AudioFrame, Channels, SampleRate};

// 以下の理由で Opus デコーダーは常にステレオ扱いにする:
// - 実際の入力に関わらず常にステレオを指定しても問題ない
// - mp4 / webm コンテナに格納されるチャネル数の情報は信用できない
// - 無音補完が入るとモノラル・ステレオのパケットが混在することがある
const DECODED_CHANNELS: u8 = 2;

#[derive(Debug)]
pub struct OpusDecoder {
    inner: shiguredo_opus::Decoder,
}

impl OpusDecoder {
    pub fn new() -> crate::Result<Self> {
        Ok(Self {
            inner: shiguredo_opus::Decoder::new(shiguredo_opus::DecoderConfig::new(
                u32::from(SampleRate::HZ_48000.as_u16()?),
                DECODED_CHANNELS,
            ))?,
        })
    }

    pub fn decode(&mut self, frame: &AudioFrame) -> crate::Result<AudioFrame> {
        if frame.format != AudioFormat::Opus {
            return Err(crate::Error::new(format!(
                "expected Opus format, got {}",
                frame.format
            )));
        }

        let decoded_samples = self.inner.decode(&frame.data)?;
        let decoded = AudioFrame {
            data: decoded_samples
                .iter()
                .flat_map(|v| v.to_be_bytes().into_iter())
                .collect(),
            format: AudioFormat::I16Be,
            channels: Channels::STEREO,
            sample_rate: SampleRate::HZ_48000,

            // Opus は通常 1 packet -> 1 frame で出力されるため、入力 timestamp をそのまま使う。
            // AAC は内部バッファリングで入出力が 1 対 1 にならない場合があるため、別方式で補正している。
            timestamp: frame.timestamp,

            // 生データにはサンプルエントリーは存在しない
            sample_entry: None,
        };

        Ok(decoded)
    }
}
