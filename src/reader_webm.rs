use std::{
    io::{BufReader, Read},
    path::Path,
    time::{Duration, Instant},
};

use orfail::OrFail;

use crate::{
    audio::{AudioData, AudioFormat, SAMPLE_RATE},
    metadata::SourceId,
    stats::{Seconds, SharedAtomicSeconds, WebmAudioReaderStats, WebmVideoReaderStats},
    types::{CodecName, EvenUsize},
    video::{VideoFormat, VideoFrame},
};

// Hisui で参照する要素 ID
const ID_EBML: u32 = 0x1A45_DFA3;
const ID_SEGMENT: u32 = 0x1853_8067;
const ID_INFO: u32 = 0x1549_A966;
const ID_TIMESTAMP_SCALE: u32 = 0x2AD7B1;
const ID_MUXING_APP: u32 = 0x4D80;
const ID_WRITING_APP: u32 = 0x5741;
const ID_TRACKS: u32 = 0x1654_AE6B;
const ID_TRACK_ENTRY: u32 = 0xAE;
const ID_CLUSTER: u32 = 0x1F43_B675;
const ID_CUES: u32 = 0x1C53BB6B;
const ID_TIMESTAMP: u32 = 0xE7;
const ID_SIMPLE_BLOCK: u32 = 0xA3;
const ID_EBML_VERSION: u32 = 0x4286;
const ID_EBML_READ_VERSION: u32 = 0x42F7;
const ID_EBML_MAX_ID_LENGTH: u32 = 0x42F2;
const ID_EBML_MAX_SIZE_LENGTH: u32 = 0x42F3;
const ID_DOC_TYPE: u32 = 0x4282;
const ID_DOC_TYPE_VERSION: u32 = 0x4287;
const ID_DOC_TYPE_READ_VERSION: u32 = 0x4285;
const ID_TRACK_NUMBER: u32 = 0xD7;
const ID_CODEC_ID: u32 = 0x86;

// 各種バージョンや設定値 (Sora 前提なので固定で大丈夫なもの)
const EBML_VERSION: u64 = 1;
const WEBM_VERSION: u64 = 4;
const WEBM_READ_VERSION: u64 = 2;
const MAX_ID_LENGTH: u64 = 4;
const MAX_SIZE_LENGTH: u64 = 8;
const TIMESTAMP_SCALE: u64 = 1_000_000; // ナノ秒が基点なので、これでミリ秒となる
const TRACK_NUMBER_VIDEO: u64 = 1;
const TRACK_NUMBER_AUDIO: u64 = 2;

#[derive(Debug)]
struct ElementReader<R> {
    inner: R,
    next_id: Option<u32>,
}

impl<R: Read> ElementReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            next_id: None,
        }
    }

    fn skip_all(&mut self) -> orfail::Result<()> {
        let mut buf = [0; 1024];
        while 0 != self.inner.read(&mut buf).or_fail()? {}
        Ok(())
    }

    fn skip_until(&mut self, id: u32) -> orfail::Result<()> {
        while id != self.peek_id().or_fail()? {
            self.read_id().or_fail()?;
            self.skip_element_data().or_fail()?;
        }
        Ok(())
    }

    fn skip_element_data(&mut self) -> orfail::Result<()> {
        let size = self.read_element_data_size().or_fail()?;
        let mut reader = self.inner.by_ref().take(size);
        let mut buf = [0; 1024];
        while 0 != reader.read(&mut buf).or_fail()? {}
        Ok(())
    }

    fn read_element_data_size(&mut self) -> orfail::Result<u64> {
        let b0 = self.read_raw_u8().or_fail()?;
        let mut size = 0;
        for i in 0..8 {
            if (b0 >> (7 - i)) == 1 {
                let mask = (1 << (7 - i)) - 1;
                size += ((b0 & mask) as u64) << (i * 8);
                if size == (1 << (i * 8 + (7 - i))) - 1 {
                    // Sora は unknown-length なデータは使ってないはずなので対応不要
                    return Err(orfail::Failure::new("unsupported: unknown length data"));
                }
                return Ok(size);
            }

            let b = self.read_raw_u8().or_fail()? as u64;
            size = (size << 8) + b;
        }
        Err(orfail::Failure::new("invalid data"))
    }

    fn read_master(
        &mut self,
        expected_id: u32,
    ) -> orfail::Result<ElementReader<std::io::Take<&mut R>>> {
        self.expect_id(expected_id).or_fail()?;
        let size = self.read_element_data_size().or_fail()?;
        Ok(ElementReader::new(self.inner.by_ref().take(size)))
    }

    fn read_master_owned(
        mut self,
        expected_id: u32,
    ) -> orfail::Result<ElementReader<std::io::Take<R>>> {
        self.expect_id(expected_id).or_fail()?;
        let size = self.read_element_data_size().or_fail()?;
        Ok(ElementReader::new(self.inner.take(size)))
    }

    fn expect_id(&mut self, expected_id: u32) -> orfail::Result<()> {
        let id = self.read_id().or_fail()?;
        (id == expected_id).or_fail_with(|()| {
            format!("expected WebM element ID 0x{expected_id:X}, but got 0x{id:X}")
        })?;
        Ok(())
    }

    fn expect_u64(&mut self, expected_id: u32, expected_value: u64) -> orfail::Result<()> {
        let actual_value = self.read_u64(expected_id).or_fail()?;
        (actual_value == expected_value).or_fail_with(|()| {
            format!(
                "expected WebM element (ID=0x{expected_id:X}) value {expected_value}, but got {actual_value}"
            )
        })?;
        Ok(())
    }

    fn read_raw_u8(&mut self) -> orfail::Result<u8> {
        let mut buf = [0];
        self.inner.read_exact(&mut buf).or_fail()?;
        Ok(buf[0])
    }

    fn read_raw_i16(&mut self) -> orfail::Result<i16> {
        let mut buf = [0; 2];
        self.inner.read_exact(&mut buf).or_fail()?;
        Ok(i16::from_be_bytes(buf))
    }

    fn read_raw_data(&mut self) -> orfail::Result<Vec<u8>> {
        let mut buf = Vec::new();
        self.inner.read_to_end(&mut buf).or_fail()?;
        Ok(buf)
    }

    fn read_u64(&mut self, expected_id: u32) -> orfail::Result<u64> {
        let data = self.read_bytes(expected_id).or_fail()?;
        (data.len() <= 8).or_fail()?;

        let mut bytes = [0; 8];
        for (i, b) in data.into_iter().rev().enumerate() {
            bytes[7 - i] = b;
        }

        Ok(u64::from_be_bytes(bytes))
    }

    fn read_bytes(&mut self, expected_id: u32) -> orfail::Result<Vec<u8>> {
        self.expect_id(expected_id).or_fail()?;

        let size = self.read_element_data_size().or_fail()?;
        (size < 1024).or_fail()?; // 念のために大きすぎる値はエラーにしておく

        let mut buf = vec![0; size as usize];
        self.inner.read_exact(&mut buf).or_fail()?;
        Ok(buf)
    }

    fn expect_str(&mut self, expected_id: u32, expected_value: &str) -> orfail::Result<()> {
        let actual_value = self.read_bytes(expected_id).or_fail()?;
        (actual_value == expected_value.as_bytes()).or_fail_with(|()| {
            format!(
                "expected WebM element (ID=0x{:X}) value {:?}, but got {:?}",
                expected_id,
                expected_value,
                String::from_utf8_lossy(&actual_value)
            )
        })?;
        Ok(())
    }

    fn peek_id(&mut self) -> orfail::Result<u32> {
        if let Some(id) = self.next_id {
            Ok(id)
        } else {
            let id = self.read_id().or_fail()?;
            self.next_id = Some(id);
            Ok(id)
        }
    }

    fn read_id(&mut self) -> orfail::Result<u32> {
        if let Some(id) = self.next_id.take() {
            return Ok(id);
        }

        let b0 = self.read_raw_u8().or_fail()?;
        if (b0 >> 7) == 1 {
            Ok(b0 as u32)
        } else if (b0 >> 6) == 1 {
            let b1 = self.read_raw_u8().or_fail()?;
            Ok(u32::from_be_bytes([0, 0, b0, b1]))
        } else if (b0 >> 5) == 1 {
            let b1 = self.read_raw_u8().or_fail()?;
            let b2 = self.read_raw_u8().or_fail()?;
            Ok(u32::from_be_bytes([0, b0, b1, b2]))
        } else {
            ((b0 >> 4) == 1).or_fail()?;
            let b1 = self.read_raw_u8().or_fail()?;
            let b2 = self.read_raw_u8().or_fail()?;
            let b3 = self.read_raw_u8().or_fail()?;
            Ok(u32::from_be_bytes([b0, b1, b2, b3]))
        }
    }
}

impl<R: Read> ElementReader<std::io::Take<R>> {
    fn is_eos(&self) -> bool {
        self.inner.limit() == 0
    }
}

fn check_ebml_header_element<R: Read>(reader: &mut ElementReader<R>) -> orfail::Result<()> {
    let mut reader = reader.read_master(ID_EBML).or_fail()?;

    reader // rustfmt の結果を揃えるためのコメント
        .expect_u64(ID_EBML_VERSION, EBML_VERSION)
        .or_fail()?;
    reader
        .expect_u64(ID_EBML_READ_VERSION, EBML_VERSION)
        .or_fail()?;
    reader
        .expect_u64(ID_EBML_MAX_ID_LENGTH, MAX_ID_LENGTH)
        .or_fail()?;
    reader
        .expect_u64(ID_EBML_MAX_SIZE_LENGTH, MAX_SIZE_LENGTH)
        .or_fail()?;
    reader // rustfmt の結果を揃えるためのコメント
        .expect_str(ID_DOC_TYPE, "webm")
        .or_fail()?;
    reader
        .expect_u64(ID_DOC_TYPE_VERSION, WEBM_VERSION)
        .or_fail()?;
    reader
        .expect_u64(ID_DOC_TYPE_READ_VERSION, WEBM_READ_VERSION)
        .or_fail()?;
    Ok(())
}

fn check_info_element<R: Read>(reader: &mut ElementReader<R>) -> orfail::Result<()> {
    let mut reader = reader.read_master(ID_INFO).or_fail()?;
    reader
        .expect_u64(ID_TIMESTAMP_SCALE, TIMESTAMP_SCALE)
        .or_fail()?;
    reader
        .expect_str(ID_MUXING_APP, "WebRTC SFU Sora")
        .or_fail()?;
    reader
        .expect_str(ID_WRITING_APP, "WebRTC SFU Sora")
        .or_fail()?;

    // 残りの要素は気にしない
    reader.skip_all().or_fail()?;

    Ok(())
}

#[derive(Debug)]
struct VideoTrackHeader {
    codec: VideoFormat,
}

impl VideoTrackHeader {
    fn read<R: Read>(reader: &mut ElementReader<R>) -> orfail::Result<Self> {
        let mut reader = reader.read_master(ID_TRACKS).or_fail()?;
        loop {
            if reader.is_eos() {
                // 映像トラックが存在しないパターン
                // コーデックの値は、実際に参照されることはないので、あり得ない値を適当に設定しておく
                log::warn!("no video track");
                return Ok(Self {
                    codec: VideoFormat::I420,
                });
            }

            let mut reader = reader.read_master(ID_TRACK_ENTRY).or_fail()?;
            let track_number = reader.read_u64(ID_TRACK_NUMBER).or_fail()?;
            if track_number != TRACK_NUMBER_VIDEO {
                reader.skip_all().or_fail()?;
                continue;
            }

            reader.skip_until(ID_CODEC_ID).or_fail()?;
            let bytes = reader.read_bytes(ID_CODEC_ID).or_fail()?;
            let codec = match bytes.as_slice() {
                b"V_VP8" => VideoFormat::Vp8,
                b"V_VP9" => VideoFormat::Vp9,
                b"V_AV1" => VideoFormat::Av1,
                b"V_MPEG4/ISO/AVC" => VideoFormat::H264AnnexB,
                _ => {
                    return Err(orfail::Failure::new(format!(
                        "unknown video codec ID: {bytes:?}"
                    )));
                }
            };
            reader.skip_all().or_fail()?;
            return Ok(Self { codec });
        }
    }
}

#[derive(Debug)]
pub struct WebmAudioReader {
    source_id: SourceId,
    reader: ElementReader<std::io::Take<BufReader<std::fs::File>>>,
    cluster_timestamp: Duration,
    last_duration: Duration,
    prev_audio_data: Option<AudioData>,
    stats: WebmAudioReaderStats,
}

impl WebmAudioReader {
    pub fn new<P: AsRef<Path>>(source_id: SourceId, path: P) -> orfail::Result<Self> {
        let start_time = Instant::now();
        let file = std::fs::File::open(&path)
            .or_fail_with(|e| format!("failed to open {}: {e}", path.as_ref().display()))?;
        let mut reader = ElementReader::new(BufReader::new(file));
        check_ebml_header_element(&mut reader).or_fail()?;

        let mut reader = reader.read_master_owned(ID_SEGMENT).or_fail()?;
        reader.skip_until(ID_INFO).or_fail()?;
        check_info_element(&mut reader).or_fail()?;
        reader.skip_until(ID_CLUSTER).or_fail()?;

        let stats = WebmAudioReaderStats {
            input_file: path.as_ref().canonicalize().or_fail_with(|e| {
                format!(
                    "failed to canonicalize path {}: {e}",
                    path.as_ref().display()
                )
            })?,
            codec: Some(CodecName::Opus),
            total_processing_seconds: SharedAtomicSeconds::new(Seconds::new(start_time.elapsed())),
            ..Default::default()
        };
        Ok(Self {
            source_id,
            reader,
            cluster_timestamp: Duration::ZERO,
            last_duration: Duration::ZERO,
            prev_audio_data: None,
            stats,
        })
    }

    pub fn stats(&self) -> &WebmAudioReaderStats {
        &self.stats
    }

    fn read_simple_block(&mut self) -> orfail::Result<Option<AudioData>> {
        let mut reader = self.reader.read_master(ID_SIMPLE_BLOCK).or_fail()?;

        let track_number = reader.read_raw_u8().or_fail()?;
        if track_number != 0b1000_0000 + TRACK_NUMBER_AUDIO as u8 {
            // 映像の場合は無視する
            reader.skip_all().or_fail()?;
            return Ok(None);
        }

        let timestamp_delta = reader.read_raw_i16().or_fail()?;
        let timestamp = if timestamp_delta < 0 {
            self.cluster_timestamp
                .saturating_sub(Duration::from_millis(timestamp_delta.unsigned_abs() as u64))
        } else {
            self.cluster_timestamp
                .saturating_add(Duration::from_millis(timestamp_delta as u64))
        };
        let _flags = reader.read_raw_u8().or_fail()?;
        let data = reader.read_raw_data().or_fail()?;

        self.stats.total_simple_block_count.add(1);
        self.stats
            .total_track_seconds
            .set(Seconds::new(timestamp + self.last_duration));

        Ok(Some(AudioData {
            source_id: Some(self.source_id.clone()),
            data,
            format: AudioFormat::Opus,
            timestamp,

            // WebM には明示的な duration の情報は格納されていないので、
            // 前後の音声データのタイムスタンプの差を設定する
            duration: self.last_duration,

            // 以降のフィールドはデコーダーには参照されないのでダミー値を設定しておく
            sample_entry: None,
            stereo: true,             // Hisui では常に固定値
            sample_rate: SAMPLE_RATE, // Hisui では常に固定値
        }))
    }

    fn read_audio_data(&mut self) -> orfail::Result<Option<AudioData>> {
        loop {
            match self.reader.peek_id().or_fail()? {
                ID_CLUSTER => {
                    // 本来ならサイズをちゃんとハンドリングすべきだけど、
                    // Hisui では Sora の録画ファイルだけが扱えればいいので無視する
                    let _ = self.reader.read_id().or_fail()?;
                    let _ = self.reader.read_element_data_size().or_fail()?;

                    let value = self.reader.read_u64(ID_TIMESTAMP).or_fail()?;
                    self.cluster_timestamp = Duration::from_millis(value);
                    self.stats.total_cluster_count.add(1);
                }
                ID_SIMPLE_BLOCK => {
                    if let Some(current) = self.read_simple_block().or_fail()? {
                        let timestamp = current.timestamp;
                        if let Some(mut prev) = self.prev_audio_data.replace(current) {
                            // 尺を確定する
                            prev.duration = timestamp.saturating_sub(prev.timestamp);
                            self.last_duration = prev.duration;
                            return Ok(Some(prev));
                        }
                    }
                }
                ID_CUES => {
                    // メディアデータ格納部分を抜けたのでここで終了
                    // 最後の AudioData が残っている場合には、まずそれを返す
                    return Ok(self.prev_audio_data.take());
                }
                id => {
                    return Err(orfail::Failure::new(format!(
                        "unexpected element ID: 0x{id:X}"
                    )));
                }
            }
        }
    }
}

impl Iterator for WebmAudioReader {
    type Item = orfail::Result<AudioData>;

    fn next(&mut self) -> Option<Self::Item> {
        // TODO: プロセッサ実行スレッドの導入タイミングで、時間計測はそっちに移動する
        let (result, elapsed) = Seconds::elapsed(|| self.read_audio_data().or_fail());
        self.stats.total_processing_seconds.add(elapsed);
        if result.is_err() {
            self.stats.error.set(true);
        }
        result.transpose()
    }
}

#[derive(Debug)]
pub struct WebmVideoReader {
    source_id: SourceId,
    header: VideoTrackHeader,
    reader: ElementReader<std::io::Take<BufReader<std::fs::File>>>,
    cluster_timestamp: Duration,
    last_duration: Duration,
    prev_video_frame: Option<VideoFrame>,
    stats: WebmVideoReaderStats,
}

impl WebmVideoReader {
    pub fn new<P: AsRef<Path>>(source_id: SourceId, path: P) -> orfail::Result<Self> {
        let start_time = Instant::now();
        let file = std::fs::File::open(&path).or_fail()?;
        let mut reader = ElementReader::new(BufReader::new(file));
        check_ebml_header_element(&mut reader).or_fail()?;

        let mut reader = reader.read_master_owned(ID_SEGMENT).or_fail()?;
        reader.skip_until(ID_INFO).or_fail()?;
        check_info_element(&mut reader).or_fail()?;

        let header = VideoTrackHeader::read(&mut reader).or_fail()?;
        reader.skip_until(ID_CLUSTER).or_fail()?;

        let stats = WebmVideoReaderStats {
            input_file: path.as_ref().canonicalize().or_fail_with(|e| {
                format!(
                    "failed to canonicalize path {}: {e}",
                    path.as_ref().display()
                )
            })?,
            total_processing_seconds: SharedAtomicSeconds::new(Seconds::new(start_time.elapsed())),
            ..Default::default()
        };
        Ok(Self {
            source_id,
            header,
            reader,
            cluster_timestamp: Duration::ZERO,
            last_duration: Duration::ZERO,
            prev_video_frame: None,
            stats,
        })
    }

    pub fn stats(&self) -> &WebmVideoReaderStats {
        &self.stats
    }

    fn read_video_frame(&mut self) -> orfail::Result<Option<VideoFrame>> {
        loop {
            match self.reader.peek_id().or_fail()? {
                ID_CLUSTER => {
                    // 本来ならサイズをちゃんとハンドリングすべきだけど、
                    // Hisui では Sora の録画ファイルだけが扱えればいいので無視する
                    let _ = self.reader.read_id().or_fail()?;
                    let _ = self.reader.read_element_data_size().or_fail()?;

                    let value = self.reader.read_u64(ID_TIMESTAMP).or_fail()?;
                    self.cluster_timestamp = Duration::from_millis(value);
                    self.stats.total_cluster_count.add(1);
                }
                ID_SIMPLE_BLOCK => {
                    if let Some(current) = self.read_simple_block().or_fail()? {
                        let timestamp = current.timestamp;
                        if let Some(mut prev) = self.prev_video_frame.replace(current) {
                            // 尺を確定する
                            prev.duration = timestamp.saturating_sub(prev.timestamp);
                            self.last_duration = prev.duration;
                            return Ok(Some(prev));
                        }
                    }
                }
                ID_CUES => {
                    // メディアデータ格納部分を抜けたのでここで終了
                    // 最後の VideoFrame が残っている場合には、まずそれを返す
                    return Ok(self.prev_video_frame.take());
                }
                id => {
                    return Err(orfail::Failure::new(format!(
                        "unexpected element ID: 0x{id:X}"
                    )));
                }
            }
        }
    }

    fn read_simple_block(&mut self) -> orfail::Result<Option<VideoFrame>> {
        let mut reader = self.reader.read_master(ID_SIMPLE_BLOCK).or_fail()?;

        let track_number = reader.read_raw_u8().or_fail()?;
        if track_number != 0b1000_0000 + TRACK_NUMBER_VIDEO as u8 {
            // 音声の場合は無視する
            reader.skip_all().or_fail()?;
            return Ok(None);
        }

        let timestamp_delta = reader.read_raw_i16().or_fail()?;
        let timestamp = if timestamp_delta < 0 {
            self.cluster_timestamp
                .saturating_sub(Duration::from_millis(timestamp_delta.unsigned_abs() as u64))
        } else {
            self.cluster_timestamp
                .saturating_add(Duration::from_millis(timestamp_delta as u64))
        };
        let flags = reader.read_raw_u8().or_fail()?;
        let keyframe = (flags >> 7) == 1;
        let data = reader.read_raw_data().or_fail()?;

        self.stats.total_simple_block_count.add(1);
        self.stats
            .total_track_seconds
            .set(Seconds::new(timestamp + self.last_duration));
        if self.stats.codec.get().is_none()
            && let Some(name) = self.header.codec.codec_name()
        {
            self.stats.codec.set(name);
        }

        Ok(Some(VideoFrame {
            source_id: Some(self.source_id.clone()),
            data,
            format: self.header.codec,
            keyframe,
            timestamp,

            // WebM には明示的な duration の情報は格納されていないので、
            // 前後のフレームのタイムスタンプの差を設定する
            duration: self.last_duration,

            // 以降のフィールドはデコーダーには参照されないのでダミー値を設定しておく
            width: EvenUsize::default(),
            height: EvenUsize::default(),
            sample_entry: None,
        }))
    }
}

impl Iterator for WebmVideoReader {
    type Item = orfail::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        // TODO: プロセッサ実行スレッドの導入タイミングで、時間計測はそっちに移動する
        let (result, elapsed) = Seconds::elapsed(|| self.read_video_frame().or_fail());
        self.stats.total_processing_seconds.add(elapsed);
        if result.is_err() {
            self.stats.error.set(true);
        }
        result.transpose()
    }
}
