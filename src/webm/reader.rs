use std::{
    io::{BufReader, Read},
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{
    audio::{
        AudioFormat, AudioFrame, Channels, SampleRate,
        opus::{opus_sample_entry, parse_opus_head_pre_skip},
    },
    sample_entry::SharedSampleEntry,
    types::{CodecName, EvenUsize},
    video::{
        VideoFormat, VideoFrame,
        av1::{av1_sample_entry, parse_av1_codec_private},
        h264::{h264_sample_entry_from_sps_pps_lists, parse_avcc_sps_pps_lists},
        vpx::{vp8_sample_entry, vp9_sample_entry},
    },
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
const ID_CODEC_PRIVATE: u32 = 0x63A2;
const ID_VIDEO: u32 = 0xE0;
const ID_PIXEL_WIDTH: u32 = 0xB0;
const ID_PIXEL_HEIGHT: u32 = 0xBA;

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

    // 上限サイズを呼び出し側で指定する版の read_bytes。
    // AV1CodecConfigurationRecord / avcC のように 1024 バイト上限を超える可能性のある
    // 要素を読むときに使う。
    fn read_bytes_with_limit(&mut self, expected_id: u32, max_size: u64) -> crate::Result<Vec<u8>> {
        self.expect_id(expected_id)?;

        let size = self.read_element_data_size()?;
        if size >= max_size {
            return Err(crate::Error::new(format!(
                "WebM element data too large: id=0x{expected_id:X}, size={size}, max={max_size}"
            )));
        }

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
struct AudioTrackHeader {
    pre_skip: u16,
}

impl AudioTrackHeader {
    // 音声 TRACK_ENTRY (A_OPUS) を走査して OpusHead pre_skip を取得する。
    // TRACKS 内の他の TRACK_ENTRY (映像など) は skip_all で読み捨てる。
    // 音声トラック不在の WebM (映像のみ録画など、Sora 録画でも普通にある) では Ok(None) を返し、
    // WebmAudioReader::new 側で sample_entry: None として旧版互換の挙動を維持する。
    fn read<R: Read>(reader: &mut ElementReader<R>) -> crate::Result<Option<Self>> {
        reader.skip_until(ID_TRACKS)?;
        let mut tracks_reader = reader.read_master(ID_TRACKS)?;
        let mut found: Option<u16> = None;
        while !tracks_reader.is_eos() {
            let mut entry = tracks_reader.read_master(ID_TRACK_ENTRY)?;
            let track_number = entry.read_u64(ID_TRACK_NUMBER)?;
            if track_number != TRACK_NUMBER_AUDIO || found.is_some() {
                // 対象外トラック、または既に音声 TRACK_ENTRY を処理済み。
                entry.skip_all()?;
                continue;
            }
            // TRACK_ENTRY 内の残り子要素を peek_id ループで走査する。
            let mut codec_id_ok = false;
            let mut pre_skip: Option<u16> = None;
            while !entry.is_eos() {
                let id = entry.peek_id()?;
                match id {
                    ID_CODEC_ID => {
                        let bytes = entry.read_bytes(ID_CODEC_ID)?;
                        if bytes.as_slice() != b"A_OPUS" {
                            return Err(crate::Error::new(format!(
                                "unsupported audio codec ID: {bytes:?} (expected A_OPUS)"
                            )));
                        }
                        codec_id_ok = true;
                    }
                    ID_CODEC_PRIVATE => {
                        let bytes = entry.read_bytes(ID_CODEC_PRIVATE)?;
                        pre_skip = Some(parse_opus_head_pre_skip(&bytes)?);
                    }
                    _ => {
                        // SamplingFrequency / Channels / OutputGain などは本 reader では使わない。
                        entry.read_id()?;
                        entry.skip_element_data()?;
                    }
                }
            }
            if !codec_id_ok {
                return Err(crate::Error::new(
                    "audio TRACK_ENTRY missing CodecID element",
                ));
            }
            let Some(pre_skip) = pre_skip else {
                return Err(crate::Error::new(
                    "audio TRACK_ENTRY missing OpusHead (CodecPrivate)",
                ));
            };
            found = Some(pre_skip);
        }
        Ok(found.map(|pre_skip| Self { pre_skip }))
    }
}

#[derive(Debug)]
struct VideoTrackHeader {
    codec: VideoFormat,
}

impl VideoTrackHeader {
    // TRACKS を走査して映像 TRACK_ENTRY を 1 件取り出す。
    // `AudioTrackHeader::read` と対称に、TRACK_ENTRY 直下を単一 peek_id ループで走査して
    // CodecID / CodecPrivate / Video master をまとめて拾う。Matroska 仕様で TRACK_ENTRY 内
    // 子要素順序は規定されていないため、CodecPrivate が CodecID より先に出る WebM でも
    // 取りこぼさない構造になる。
    //
    // width / height / codec_private は `WebmVideoReader::new` のローカル変数で消費するため
    // 戻り値タプルで返し、struct には保持しない。
    //
    // 映像トラック不在 (TRACKS 内に TRACK_NUMBER_VIDEO の TRACK_ENTRY が無い) の場合は
    // `(Self { codec: VideoFormat::I420 }, 0, 0, Vec::new())` のセンチネル値を返す
    // (生 YUV を指す I420 を映像トラック不在の placeholder に流用する。
    // Sora 録画は映像トラック必須前提で実運用では発生しない経路)。
    fn read<R: Read>(
        reader: &mut ElementReader<R>,
    ) -> crate::Result<(Self, usize, usize, Vec<u8>)> {
        let mut tracks_reader = reader.read_master(ID_TRACKS)?;
        let mut found: Option<(VideoFormat, usize, usize, Vec<u8>)> = None;
        while !tracks_reader.is_eos() {
            let mut entry = tracks_reader.read_master(ID_TRACK_ENTRY)?;
            let track_number = entry.read_u64(ID_TRACK_NUMBER)?;
            if track_number != TRACK_NUMBER_VIDEO || found.is_some() {
                // 対象外トラック、または既に映像 TRACK_ENTRY を処理済み。
                // `AudioTrackHeader::read` と対称に、found 後も break せず残り TRACK_ENTRY を消費する。
                entry.skip_all()?;
                continue;
            }
            // TRACK_ENTRY 内残り子要素を peek_id ループで走査する。
            let mut codec: Option<VideoFormat> = None;
            let mut codec_private: Vec<u8> = Vec::new();
            let mut width: usize = 0;
            let mut height: usize = 0;
            let mut video_seen = false;
            while !entry.is_eos() {
                let id = entry.peek_id()?;
                match id {
                    ID_CODEC_ID => {
                        let bytes = entry.read_bytes(ID_CODEC_ID)?;
                        codec = Some(match bytes.as_slice() {
                            b"V_VP8" => VideoFormat::Vp8,
                            b"V_VP9" => VideoFormat::Vp9,
                            b"V_AV1" => VideoFormat::Av1,
                            b"V_MPEG4/ISO/AVC" => VideoFormat::H264AnnexB,
                            _ => {
                                return Err(crate::Error::new(format!(
                                    "unknown video codec ID: {bytes:?}"
                                )));
                            }
                        });
                    }
                    ID_CODEC_PRIVATE => {
                        // AV1CodecConfigurationRecord / avcC は 1024 バイトを超える可能性があるため
                        // CodecPrivate 専用の `read_bytes_with_limit` を使い上限を 64 KB に拡大する。
                        codec_private = entry.read_bytes_with_limit(ID_CODEC_PRIVATE, 64 * 1024)?;
                    }
                    ID_VIDEO => {
                        video_seen = true;
                        let mut video_reader = entry.read_master(ID_VIDEO)?;
                        let (w, h) = read_pixel_dimensions(&mut video_reader)?;
                        width = w;
                        height = h;
                    }
                    _ => {
                        // FlagLacing / Language など本 reader では使わない子要素は読み捨て。
                        entry.read_id()?;
                        entry.skip_element_data()?;
                    }
                }
            }
            let Some(codec) = codec else {
                return Err(crate::Error::new(
                    "video TRACK_ENTRY missing CodecID element",
                ));
            };
            if !video_seen {
                // VIDEO master が TRACK_ENTRY 直下に不在のフォールバック。
                // 警告ログを出して width / height = 0 で続行する。
                tracing::warn!(
                    "WebM video TRACK_ENTRY has no Video master element; falling back to width=0 height=0"
                );
            }
            found = Some((codec, width, height, codec_private));
        }
        if let Some((codec, width, height, codec_private)) = found {
            Ok((Self { codec }, width, height, codec_private))
        } else {
            // 映像トラックが存在しないパターン。Sora 録画は映像トラックを必ず含む前提のため、
            // 実運用では発生しない経路。生 YUV を指す I420 を映像トラック不在の placeholder に
            // 流用するが、Sora 録画前提では `WebmVideoReader::read_simple_block` で
            // この値が参照されることはない (track_number 不一致でフレーム生成が起きないため)。
            tracing::warn!(
                "WebM TRACKS has no video TRACK_ENTRY; codec set to I420 as placeholder"
            );
            Ok((
                Self {
                    codec: VideoFormat::I420,
                },
                0,
                0,
                Vec::new(),
            ))
        }
    }
}

// VIDEO master 内の PixelWidth / PixelHeight を取得する。
// PixelWidth / PixelHeight が VIDEO master 内に無い場合は 0 でフォールバックして警告ログを出す
// (VIDEO master 自体が TRACK_ENTRY 直下に不在の場合の警告は `VideoTrackHeader::read` 側で出す)。
//
// width=0 / height=0 のフォールバック値はそのまま VP8 / VP9 の sample_entry に載って下流に
// 流れる。Sora 録画前提では発生しない異常系のため Err にはしないが、もし sample_entry 経由で
// MP4 STSD に直接書き出される経路が将来発生した場合は、ISO/IEC 14496-12 上の不正解像度として
// 扱われる可能性がある。compose 経路 (src/sora/recording_reader.rs) では再エンコードで
// sample_entry が差し替わるため実害は無い。
fn read_pixel_dimensions<R: Read>(
    video_reader: &mut ElementReader<std::io::Take<R>>,
) -> crate::Result<(usize, usize)> {
    let mut width: usize = 0;
    let mut height: usize = 0;
    while !video_reader.is_eos() {
        let inner_id = video_reader.peek_id()?;
        match inner_id {
            ID_PIXEL_WIDTH => {
                width = video_reader.read_u64(ID_PIXEL_WIDTH)? as usize;
            }
            ID_PIXEL_HEIGHT => {
                height = video_reader.read_u64(ID_PIXEL_HEIGHT)? as usize;
            }
            _ => {
                // Video master 内の他の子要素 (FrameRate / DisplayWidth など) は本 reader では使わない。
                video_reader.read_id()?;
                video_reader.skip_element_data()?;
            }
        }
    }
    if width == 0 || height == 0 {
        tracing::warn!(
            width,
            height,
            "WebM video TRACK_ENTRY missing PixelWidth or PixelHeight; falling back to 0"
        );
    }
    Ok((width, height))
}

#[derive(Debug)]
pub struct WebmAudioReader {
    reader: ElementReader<std::io::Take<BufReader<std::fs::File>>>,
    cluster_timestamp: Duration,
    sample_entry: Option<SharedSampleEntry>,

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
        // 音声 TRACK_ENTRY (A_OPUS) から OpusHead pre_skip を取得して sample_entry を構築する。
        // ファイル切り替え時はリーダー再生成で sample_entry も自動的に再構築される (inherit_stats_from の対象外)。
        // 音声トラック不在 (映像のみ録画など) では sample_entry: None になるが、read_simple_block で
        // 圧縮 AudioFrame が生成される経路は track_number 不一致で発生しないため不変条件は保たれる。
        let sample_entry = AudioTrackHeader::read(&mut reader)?
            .map(|h| SharedSampleEntry::new(opus_sample_entry(h.pre_skip)));
        reader.skip_until(ID_CLUSTER)?;

        Ok(Self {
            reader,
            cluster_timestamp: Duration::ZERO,
            sample_entry,
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
        // WebM は payload を解釈しないため、サンプル末尾の正確な duration はここでは求めない。
        // そのため total_track_duration は「最終 SimpleBlock の開始時刻」を表す。
        // (MP4 の total_track_duration とは意味が異なる)
        self.total_track_duration = self.total_track_duration.max(timestamp);

        Ok(Some(AudioFrame {
            data,
            format: AudioFormat::Opus,
            timestamp,

            // sample_entry は WebmAudioReader::new で OpusHead pre_skip から構築済み。
            // 不変条件 (圧縮 AudioFrame は常に Some) は src/audio.rs::AudioFrame の docstring を参照。
            sample_entry: self.sample_entry.clone(),
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
    sample_entry: Option<SharedSampleEntry>,
    pub current_input_file: Option<PathBuf>,
    pub codec: Option<CodecName>,
    pub total_cluster_count: u64,
    pub total_simple_block_count: u64,
    pub total_track_duration: Duration,
    pub track_duration_offset: Duration,
}

impl WebmVideoReader {
    pub fn new<P: AsRef<Path>>(path: P) -> crate::Result<Self> {
        let file = std::fs::File::open(&path).map_err(|e| {
            crate::Error::new(format!("failed to open {}: {e}", path.as_ref().display()))
        })?;
        let mut reader = ElementReader::new(BufReader::new(file));
        check_ebml_header_element(&mut reader)?;

        let mut reader = reader.read_master_owned(ID_SEGMENT)?;
        reader.skip_until(ID_INFO)?;
        check_info_element(&mut reader)?;

        let (header, width, height, codec_private) = VideoTrackHeader::read(&mut reader)?;
        // VP8 / VP9 / AV1 / H264AnnexB は CodecPrivate (AV1CodecConfigurationRecord / avcC) を
        // 解析して sample_entry を構築する。I420 (映像トラック不在のフォールバック) は生フォーマットで
        // 不変条件の対象外のため None を保持する。
        let sample_entry = match header.codec {
            VideoFormat::Vp8 => Some(SharedSampleEntry::new(vp8_sample_entry(width, height))),
            VideoFormat::Vp9 => Some(SharedSampleEntry::new(vp9_sample_entry(width, height))),
            VideoFormat::Av1 => {
                let config_obus = parse_av1_codec_private(&codec_private)?;
                Some(SharedSampleEntry::new(av1_sample_entry(
                    EvenUsize::truncating_new(width),
                    EvenUsize::truncating_new(height),
                    config_obus,
                )))
            }
            VideoFormat::H264AnnexB => {
                let (sps_list, pps_list) = parse_avcc_sps_pps_lists(&codec_private)?;
                let (entry, _frame_size) =
                    h264_sample_entry_from_sps_pps_lists(sps_list, pps_list)?;
                Some(SharedSampleEntry::new(entry))
            }
            VideoFormat::I420 => None,
            // VideoTrackHeader::read が返す codec は Vp8 / Vp9 / Av1 / H264AnnexB / I420 の 5 値のみで、
            // 以下は構造的に到達不能な防御。VideoFormat の他バリアント (H264 / H265 / I420A) が将来
            // VideoTrackHeader::read のマッピングに追加されたとき silently ランタイムエラー化する
            // 余地を防ぐためのガードとして残す。
            other => {
                return Err(crate::Error::new(format!(
                    "WebmVideoReader received unexpected video format from TRACKS: {other:?}"
                )));
            }
        };
        reader.skip_until(ID_CLUSTER)?;

        Ok(Self {
            header,
            reader,
            cluster_timestamp: Duration::ZERO,
            sample_entry,
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
        // WebM は payload を解釈しないため、サンプル末尾の正確な duration はここでは求めない。
        // そのため total_track_duration は「最終 SimpleBlock の開始時刻」を表す。
        // (MP4 の total_track_duration とは意味が異なる)
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

            // WebM では payload を解析しないためサイズ情報は保持しない。
            // 利用側で必要な場合は後段で補完する。
            size: None,
            // sample_entry は WebmVideoReader::new で TRACKS の PixelWidth / PixelHeight から構築済み。
            // 不変条件 (圧縮 VideoFrame は常に Some) は src/video.rs::VideoFrame の docstring を参照。
            sample_entry: self.sample_entry.clone(),
        }))
    }
}

impl Iterator for WebmVideoReader {
    type Item = crate::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        self.read_video_frame().transpose()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn webm_audio_reader_releases_new_arc_per_construction() {
        // ファイル切り替えはリーダー再生成で新 Arc を確保する。同じ testdata を 2 回開いて
        // それぞれの sample_entry が「実体としては等しいが Arc としては別物」であることを検証する。
        let mut reader_a = WebmAudioReader::new("testdata/archive-black-silent.webm")
            .expect("testdata を 1 回目で開ける");
        let mut reader_b = WebmAudioReader::new("testdata/archive-black-silent.webm")
            .expect("testdata を 2 回目で開ける");
        let frame_a = reader_a
            .next()
            .expect("1 回目: フレームが存在")
            .expect("1 回目: フレーム取得が成功");
        let frame_b = reader_b
            .next()
            .expect("2 回目: フレームが存在")
            .expect("2 回目: フレーム取得が成功");
        let entry_a = frame_a
            .sample_entry
            .expect("1 回目: 圧縮フレームには sample_entry が載る");
        let entry_b = frame_b
            .sample_entry
            .expect("2 回目: 圧縮フレームには sample_entry が載る");
        // 別々にコンストラクタを呼んだので Arc は別物。
        assert!(
            !entry_a.ptr_eq(&entry_b),
            "ファイルごとに新規 Arc を確保すること"
        );
        // 実体 (SampleEntry の中身) は等しいため、writer 側の changed_since が Arc::ptr_eq 短絡を外れても
        // PartialEq で同値判定され、サンプルエントリー差し替えは発生しない。
        assert_eq!(
            entry_a.get(),
            entry_b.get(),
            "実体は同値なので writer 側で dedup されること"
        );
    }

    #[test]
    fn webm_video_reader_releases_new_arc_per_construction() {
        // 映像側でも同じ「ファイルごとに新規 Arc を確保 + 実体は同値」契約を検証する。
        // 音声側と対称な保証で、sample_entry が intern キャッシュで共有される退行を検出する。
        let mut reader_a = WebmVideoReader::new("testdata/archive-black-silent.webm")
            .expect("testdata を 1 回目で開ける");
        let mut reader_b = WebmVideoReader::new("testdata/archive-black-silent.webm")
            .expect("testdata を 2 回目で開ける");
        let frame_a = reader_a
            .next()
            .expect("1 回目: フレームが存在")
            .expect("1 回目: フレーム取得が成功");
        let frame_b = reader_b
            .next()
            .expect("2 回目: フレームが存在")
            .expect("2 回目: フレーム取得が成功");
        let entry_a = frame_a
            .sample_entry
            .expect("1 回目: 圧縮フレームには sample_entry が載る");
        let entry_b = frame_b
            .sample_entry
            .expect("2 回目: 圧縮フレームには sample_entry が載る");
        assert!(
            !entry_a.ptr_eq(&entry_b),
            "ファイルごとに新規 Arc を確保すること"
        );
        assert_eq!(
            entry_a.get(),
            entry_b.get(),
            "実体は同値なので writer 側で dedup されること"
        );
    }

    #[test]
    fn read_bytes_with_limit_succeeds_with_size_below_limit() {
        // ID_CODEC_PRIVATE (0x63A2、2 バイト vint) + size = 4 (1 バイト vint 0x84) + 4 バイト data。
        // max_size = 16 で size = 4 はリミット内のため、正常にデータを取り出せる。
        let bytes: Vec<u8> = vec![
            0x63, 0xa2, // ID_CODEC_PRIVATE
            0x84, // vint size = 4
            0xaa, 0xbb, 0xcc, 0xdd, // data
        ];
        let mut reader = ElementReader::new(Cursor::new(bytes));
        let result = reader
            .read_bytes_with_limit(ID_CODEC_PRIVATE, 16)
            .expect("リミット内の正常入力でパース成功");
        assert_eq!(result, vec![0xaa, 0xbb, 0xcc, 0xdd]);
    }

    #[test]
    fn read_bytes_with_limit_returns_err_when_size_reaches_limit() {
        // ID_CODEC_PRIVATE + size = 16 (1 バイト vint 0x90)。max_size = 16 のため
        // 境界条件 `size >= max_size` で Err を返すこと (size ちょうども拒否される)。
        let bytes: Vec<u8> = vec![
            0x63, 0xa2, // ID_CODEC_PRIVATE
            0x90, // vint size = 16 (data 本体は Err 前に読まれないため省略)
        ];
        let mut reader = ElementReader::new(Cursor::new(bytes));
        let result = reader.read_bytes_with_limit(ID_CODEC_PRIVATE, 16);
        assert!(
            result.is_err(),
            "size == max_size は境界判定 `>=` で Err を返すこと: {result:?}"
        );
    }

    #[test]
    fn read_bytes_with_limit_returns_err_on_id_mismatch() {
        // ID_CODEC_ID (0x86、1 バイト vint) + size = 4 + 4 バイト data を書き込み、
        // `expected_id = ID_CODEC_PRIVATE` で読み込もうとすると ID 不一致で Err を返すこと。
        let bytes: Vec<u8> = vec![
            0x86, // ID_CODEC_ID (期待値の ID_CODEC_PRIVATE と異なる)
            0x84, // vint size = 4
            0xaa, 0xbb, 0xcc, 0xdd, // data
        ];
        let mut reader = ElementReader::new(Cursor::new(bytes));
        let result = reader.read_bytes_with_limit(ID_CODEC_PRIVATE, 16);
        assert!(result.is_err(), "ID 不一致時に Err が返ること: {result:?}");
    }
}
