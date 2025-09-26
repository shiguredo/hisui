use orfail::OrFail;

use crate::audio::{AudioData, AudioFormat, SAMPLE_RATE};

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
    pub fn new() -> orfail::Result<Self> {
        Ok(Self {
            inner: shiguredo_opus::Decoder::new(SAMPLE_RATE, DECODED_CHANNELS).or_fail()?,
        })
    }

    pub fn decode(&mut self, data: &AudioData) -> orfail::Result<AudioData> {
        (data.format == AudioFormat::Opus).or_fail()?;

        let decoded_samples = self.inner.decode(&data.data).or_fail()?;
        let decoded = AudioData {
            source_id: data.source_id.clone(),
            data: decoded_samples
                .iter()
                .flat_map(|v| v.to_be_bytes().into_iter())
                .collect(),
            format: AudioFormat::I16Be,
            stereo: true,
            sample_rate: SAMPLE_RATE,
            timestamp: data.timestamp,
            duration: data.duration,

            // 生データにはサンプルエントリーは存在しない
            sample_entry: None,
        };

        Ok(decoded)
    }
}
