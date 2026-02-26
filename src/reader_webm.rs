use std::{
    io::{BufReader, Read},
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{
    audio::{AudioFormat, AudioFrame, Channels, SampleRate},
    types::CodecName,
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

    fn skip_all(&mut self) -> crate::Result<()> {
        let mut buf = [0; 1024];
        while 0 != self.inner.read(&mut buf)? {}
        Ok(())
    }

    fn skip_until(&mut self, id: u32) -> crate::Result<()> {
        while id != self.peek_id()? {
            self.read_id()?;
            self.skip_element_data()?;
        }
        Ok(())
    }

    fn skip_element_data(&mut self) -> crate::Result<()> {
        let size = self.read_element_data_size()?;
        let mut reader = self.inner.by_ref().take(size);
        let mut buf = [0; 1024];
        while 0 != reader.read(&mut buf)? {}
        Ok(())
    }

    fn read_element_data_size(&mut self) -> crate::Result<u64> {
        let b0 = self.read_raw_u8()?;
        let mut size = 0;
        for i in 0..8 {
            if (b0 >> (7 - i)) == 1 {
                let mask = (1 << (7 - i)) - 1;
                size += ((b0 & mask) as u64) << (i * 8);
                if size == (1 << (i * 8 + (7 - i))) - 1 {
                    // Sora は unknown-length なデータは使ってないはずなので対応不要
                    return Err(crate::Error::new("unsupported: unknown length data"));
                }
                return Ok(size);
            }

            let b = self.read_raw_u8()? as u64;
            size = (size << 8) + b;
        }
        Err(crate::Error::new("invalid data"))
    }

    fn read_master(
        &mut self,
        expected_id: u32,
    ) -> crate::Result<ElementReader<std::io::Take<&mut R>>> {
        self.expect_id(expected_id)?;
        let size = self.read_element_data_size()?;
        Ok(ElementReader::new(self.inner.by_ref().take(size)))
    }

    fn read_master_owned(
        mut self,
        expected_id: u32,
    ) -> crate::Result<ElementReader<std::io::Take<R>>> {
        self.expect_id(expected_id)?;
        let size = self.read_element_data_size()?;
        Ok(ElementReader::new(self.inner.take(size)))
    }

    fn expect_id(&mut self, expected_id: u32) -> crate::Result<()> {
        let id = self.read_id()?;
        if id != expected_id {
            return Err(crate::Error::new(format!(
                "expected WebM element ID 0x{expected_id:X}, but got 0x{id:X}"
            )));
        }
        Ok(())
    }

    fn expect_u64(&mut self, expected_id: u32, expected_value: u64) -> crate::Result<()> {
        let actual_value = self.read_u64(expected_id)?;
        if actual_value != expected_value {
            return Err(crate::Error::new(format!(
                "expected WebM element (ID=0x{expected_id:X}) value {expected_value}, but got {actual_value}"
            )));
        }
        Ok(())
    }

    fn read_raw_u8(&mut self) -> crate::Result<u8> {
        let mut buf = [0];
        self.inner.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    fn read_raw_i16(&mut self) -> crate::Result<i16> {
        let mut buf = [0; 2];
        self.inner.read_exact(&mut buf)?;
        Ok(i16::from_be_bytes(buf))
    }

    fn read_raw_data(&mut self) -> crate::Result<Vec<u8>> {
        let mut buf = Vec::new();
        self.inner.read_to_end(&mut buf)?;
        Ok(buf)
    }

    fn read_u64(&mut self, expected_id: u32) -> crate::Result<u64> {
        let data = self.read_bytes(expected_id)?;
        if data.len() > 8 {
            return Err(crate::Error::new("invalid data"));
        }

        let mut bytes = [0; 8];
        for (i, b) in data.into_iter().rev().enumerate() {
            bytes[7 - i] = b;
        }

        Ok(u64::from_be_bytes(bytes))
    }

    fn read_bytes(&mut self, expected_id: u32) -> crate::Result<Vec<u8>> {
        self.expect_id(expected_id)?;

        let size = self.read_element_data_size()?;
        if size >= 1024 {
            return Err(crate::Error::new("invalid data"));
        } // 念のために大きすぎる値はエラーにしておく

        let mut buf = vec![0; size as usize];
        self.inner.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn expect_str(&mut self, expected_id: u32, expected_value: &str) -> crate::Result<()> {
        let actual_value = self.read_bytes(expected_id)?;
        if actual_value != expected_value.as_bytes() {
            return Err(crate::Error::new(format!(
                "expected WebM element (ID=0x{:X}) value {:?}, but got {:?}",
                expected_id,
                expected_value,
                String::from_utf8_lossy(&actual_value)
            )));
        }
        Ok(())
    }

    fn peek_id(&mut self) -> crate::Result<u32> {
        if let Some(id) = self.next_id {
            Ok(id)
        } else {
            let id = self.read_id()?;
            self.next_id = Some(id);
            Ok(id)
        }
    }

    fn read_id(&mut self) -> crate::Result<u32> {
        if let Some(id) = self.next_id.take() {
            return Ok(id);
        }

        let b0 = self.read_raw_u8()?;
        if (b0 >> 7) == 1 {
            Ok(b0 as u32)
        } else if (b0 >> 6) == 1 {
            let b1 = self.read_raw_u8()?;
            Ok(u32::from_be_bytes([0, 0, b0, b1]))
        } else if (b0 >> 5) == 1 {
            let b1 = self.read_raw_u8()?;
            let b2 = self.read_raw_u8()?;
            Ok(u32::from_be_bytes([0, b0, b1, b2]))
        } else {
            if (b0 >> 4) != 1 {
                return Err(crate::Error::new("invalid data"));
            }
            let b1 = self.read_raw_u8()?;
            let b2 = self.read_raw_u8()?;
            let b3 = self.read_raw_u8()?;
            Ok(u32::from_be_bytes([b0, b1, b2, b3]))
        }
    }
}

impl<R: Read> ElementReader<std::io::Take<R>> {
    fn is_eos(&self) -> bool {
        self.inner.limit() == 0
    }
}

fn check_ebml_header_element<R: Read>(reader: &mut ElementReader<R>) -> crate::Result<()> {
    let mut reader = reader.read_master(ID_EBML)?;

    reader // rustfmt の結果を揃えるためのコメント
        .expect_u64(ID_EBML_VERSION, EBML_VERSION)?;
    reader.expect_u64(ID_EBML_READ_VERSION, EBML_VERSION)?;
    reader.expect_u64(ID_EBML_MAX_ID_LENGTH, MAX_ID_LENGTH)?;
    reader.expect_u64(ID_EBML_MAX_SIZE_LENGTH, MAX_SIZE_LENGTH)?;
    reader // rustfmt の結果を揃えるためのコメント
        .expect_str(ID_DOC_TYPE, "webm")?;
    reader.expect_u64(ID_DOC_TYPE_VERSION, WEBM_VERSION)?;
    reader.expect_u64(ID_DOC_TYPE_READ_VERSION, WEBM_READ_VERSION)?;
    Ok(())
}

fn check_info_element<R: Read>(reader: &mut ElementReader<R>) -> crate::Result<()> {
    let mut reader = reader.read_master(ID_INFO)?;
    reader.expect_u64(ID_TIMESTAMP_SCALE, TIMESTAMP_SCALE)?;
    reader.expect_str(ID_MUXING_APP, "WebRTC SFU Sora")?;
    reader.expect_str(ID_WRITING_APP, "WebRTC SFU Sora")?;

    // 残りの要素は気にしない
    reader.skip_all()?;

    Ok(())
}

#[derive(Debug)]
struct VideoTrackHeader {
    codec: VideoFormat,
}

impl VideoTrackHeader {
    fn read<R: Read>(reader: &mut ElementReader<R>) -> crate::Result<Self> {
        let mut reader = reader.read_master(ID_TRACKS)?;
        loop {
            if reader.is_eos() {
                // 映像トラックが存在しないパターン
                // コーデックの値は、実際に参照されることはないので、あり得ない値を適当に設定しておく
                tracing::warn!("no video track");
                return Ok(Self {
                    codec: VideoFormat::I420,
                });
            }

            let mut reader = reader.read_master(ID_TRACK_ENTRY)?;
            let track_number = reader.read_u64(ID_TRACK_NUMBER)?;
            if track_number != TRACK_NUMBER_VIDEO {
                reader.skip_all()?;
                continue;
            }

            reader.skip_until(ID_CODEC_ID)?;
            let bytes = reader.read_bytes(ID_CODEC_ID)?;
            let codec = match bytes.as_slice() {
                b"V_VP8" => VideoFormat::Vp8,
                b"V_VP9" => VideoFormat::Vp9,
                b"V_AV1" => VideoFormat::Av1,
                b"V_MPEG4/ISO/AVC" => VideoFormat::H264AnnexB,
                _ => {
                    return Err(crate::Error::new(format!(
                        "unknown video codec ID: {bytes:?}"
                    )));
                }
            };
            reader.skip_all()?;
            return Ok(Self { codec });
        }
    }
}

#[derive(Debug)]
pub struct WebmAudioReader {
    reader: ElementReader<std::io::Take<BufReader<std::fs::File>>>,
    cluster_timestamp: Duration,

    pub current_input_file: Option<PathBuf>,
    pub codec: Option<CodecName>,
    pub total_cluster_count: u64,
    pub total_simple_block_count: u64,
    pub total_track_duration: Duration,
    pub track_duration_offset: Duration,
}

impl WebmAudioReader {
    pub fn new<P: AsRef<Path>>(path: P) -> crate::Result<Self> {
        let file = std::fs::File::open(&path).map_err(|e| {
            crate::Error::new(format!("failed to open {}: {e}", path.as_ref().display()))
        })?;
        let mut reader = ElementReader::new(BufReader::new(file));
        check_ebml_header_element(&mut reader)?;

        let mut reader = reader.read_master_owned(ID_SEGMENT)?;
        reader.skip_until(ID_INFO)?;
        check_info_element(&mut reader)?;
        reader.skip_until(ID_CLUSTER)?;

        Ok(Self {
            reader,
            cluster_timestamp: Duration::ZERO,
            current_input_file: Some(path.as_ref().to_path_buf()),
            codec: None,
            total_cluster_count: 0,
            total_simple_block_count: 0,
            total_track_duration: Duration::ZERO,
            track_duration_offset: Duration::ZERO,
        })
    }

    pub fn stats(&self) -> &Self {
        self
    }

    pub fn stats_mut(&mut self) -> &mut Self {
        self
    }

    pub fn inherit_stats_from(&mut self, prev: &Self) {
        self.codec = prev.codec;
        self.total_cluster_count = prev.total_cluster_count;
        self.total_simple_block_count = prev.total_simple_block_count;
        self.total_track_duration = prev.total_track_duration;
        self.track_duration_offset = prev.track_duration_offset;
    }

    fn read_simple_block(&mut self) -> crate::Result<Option<AudioFrame>> {
        let mut reader = self.reader.read_master(ID_SIMPLE_BLOCK)?;

        let track_number = reader.read_raw_u8()?;
        if track_number != 0b1000_0000 + TRACK_NUMBER_AUDIO as u8 {
            // 映像の場合は無視する
            reader.skip_all()?;
            return Ok(None);
        }

        let timestamp_delta = reader.read_raw_i16()?;
        let timestamp = if timestamp_delta < 0 {
            self.cluster_timestamp
                .saturating_sub(Duration::from_millis(timestamp_delta.unsigned_abs() as u64))
        } else {
            self.cluster_timestamp
                .saturating_add(Duration::from_millis(timestamp_delta as u64))
        };
        let _flags = reader.read_raw_u8()?;
        let data = reader.read_raw_data()?;

        self.total_simple_block_count += 1;
        self.total_track_duration = self.total_track_duration.max(timestamp);

        Ok(Some(AudioFrame {
            data,
            format: AudioFormat::Opus,
            timestamp,

            // 以降のフィールドはデコーダーには参照されないのでダミー値を設定しておく
            sample_entry: None,
            channels: Channels::STEREO,        // Hisui では常に固定値
            sample_rate: SampleRate::HZ_48000, // Hisui では常に固定値
        }))
    }

    fn read_audio_data(&mut self) -> crate::Result<Option<AudioFrame>> {
        loop {
            match self.reader.peek_id()? {
                ID_CLUSTER => {
                    // 本来ならサイズをちゃんとハンドリングすべきだけど、
                    // Hisui では Sora の録画ファイルだけが扱えればいいので無視する
                    let _ = self.reader.read_id()?;
                    let _ = self.reader.read_element_data_size()?;

                    let value = self.reader.read_u64(ID_TIMESTAMP)?;
                    self.cluster_timestamp = Duration::from_millis(value);
                    self.total_cluster_count += 1;
                }
                ID_SIMPLE_BLOCK => {
                    if let Some(current) = self.read_simple_block()? {
                        return Ok(Some(current));
                    }
                }
                ID_CUES => {
                    // メディアデータ格納部分を抜けたのでここで終了
                    return Ok(None);
                }
                id => {
                    return Err(crate::Error::new(format!(
                        "unexpected element ID: 0x{id:X}"
                    )));
                }
            }
        }
    }
}

impl Iterator for WebmAudioReader {
    type Item = crate::Result<AudioFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        self.read_audio_data().transpose()
    }
}

#[derive(Debug)]
pub struct WebmVideoReader {
    header: VideoTrackHeader,
    reader: ElementReader<std::io::Take<BufReader<std::fs::File>>>,
    cluster_timestamp: Duration,
    pub current_input_file: Option<PathBuf>,
    pub codec: Option<CodecName>,
    pub total_cluster_count: u64,
    pub total_simple_block_count: u64,
    pub total_track_duration: Duration,
    pub track_duration_offset: Duration,
}

impl WebmVideoReader {
    pub fn new<P: AsRef<Path>>(path: P) -> crate::Result<Self> {
        let file = std::fs::File::open(&path)?;
        let mut reader = ElementReader::new(BufReader::new(file));
        check_ebml_header_element(&mut reader)?;

        let mut reader = reader.read_master_owned(ID_SEGMENT)?;
        reader.skip_until(ID_INFO)?;
        check_info_element(&mut reader)?;

        let header = VideoTrackHeader::read(&mut reader)?;
        reader.skip_until(ID_CLUSTER)?;

        Ok(Self {
            header,
            reader,
            cluster_timestamp: Duration::ZERO,
            current_input_file: Some(path.as_ref().to_path_buf()),
            codec: None,
            total_cluster_count: 0,
            total_simple_block_count: 0,
            total_track_duration: Duration::ZERO,
            track_duration_offset: Duration::ZERO,
        })
    }

    pub fn stats(&self) -> &Self {
        self
    }

    pub fn stats_mut(&mut self) -> &mut Self {
        self
    }

    pub fn inherit_stats_from(&mut self, prev: &Self) {
        self.codec = prev.codec;
        self.total_cluster_count = prev.total_cluster_count;
        self.total_simple_block_count = prev.total_simple_block_count;
        self.total_track_duration = prev.total_track_duration;
        self.track_duration_offset = prev.track_duration_offset;
    }

    fn read_video_frame(&mut self) -> crate::Result<Option<VideoFrame>> {
        loop {
            match self.reader.peek_id()? {
                ID_CLUSTER => {
                    // 本来ならサイズをちゃんとハンドリングすべきだけど、
                    // Hisui では Sora の録画ファイルだけが扱えればいいので無視する
                    let _ = self.reader.read_id()?;
                    let _ = self.reader.read_element_data_size()?;

                    let value = self.reader.read_u64(ID_TIMESTAMP)?;
                    self.cluster_timestamp = Duration::from_millis(value);
                    self.total_cluster_count += 1;
                }
                ID_SIMPLE_BLOCK => {
                    if let Some(current) = self.read_simple_block()? {
                        return Ok(Some(current));
                    }
                }
                ID_CUES => {
                    // メディアデータ格納部分を抜けたのでここで終了
                    return Ok(None);
                }
                id => {
                    return Err(crate::Error::new(format!(
                        "unexpected element ID: 0x{id:X}"
                    )));
                }
            }
        }
    }

    fn read_simple_block(&mut self) -> crate::Result<Option<VideoFrame>> {
        let mut reader = self.reader.read_master(ID_SIMPLE_BLOCK)?;

        let track_number = reader.read_raw_u8()?;
        if track_number != 0b1000_0000 + TRACK_NUMBER_VIDEO as u8 {
            // 音声の場合は無視する
            reader.skip_all()?;
            return Ok(None);
        }

        let timestamp_delta = reader.read_raw_i16()?;
        let timestamp = if timestamp_delta < 0 {
            self.cluster_timestamp
                .saturating_sub(Duration::from_millis(timestamp_delta.unsigned_abs() as u64))
        } else {
            self.cluster_timestamp
                .saturating_add(Duration::from_millis(timestamp_delta as u64))
        };
        let flags = reader.read_raw_u8()?;
        let keyframe = (flags >> 7) == 1;
        let data = reader.read_raw_data()?;

        self.total_simple_block_count += 1;
        self.total_track_duration = self.total_track_duration.max(timestamp);
        if self.codec.is_none()
            && let Some(name) = self.header.codec.codec_name()
        {
            self.codec = Some(name);
        }

        Ok(Some(VideoFrame {
            data,
            format: self.header.codec,
            keyframe,
            timestamp,

            // 以降のフィールドはデコーダーには参照されないのでダミー値を設定しておく
            width: 0,
            height: 0,
            sample_entry: None,
        }))
    }
}

impl Iterator for WebmVideoReader {
    type Item = crate::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        self.read_video_frame().transpose()
    }
}
