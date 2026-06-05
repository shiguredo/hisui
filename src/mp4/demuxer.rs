//! 通常 MP4 と fragmented MP4 の前方読みを統一的に扱うモジュール
//!
//! inspect のようにシーク不要でサンプルを前方に読み出すだけの用途で使う。
//! 通常 MP4 (`Mp4FileDemuxer`) と fMP4 (`Fmp4FileDemuxer`) の差異を enum で吸収する。
//! fMP4 は `next_sample()` が追加入力を要求する (`InputRequired`) ため、
//! `File` を保持してその都度ファイルから供給する。

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};

use shiguredo_mp4::{
    TrackKind,
    boxes::SampleEntry,
    demux::{
        DemuxError, Fmp4FileDemuxer, Input, Mp4FileDemuxer, Mp4FileKind, RequiredInput, Sample,
        TrackInfo,
    },
};

use crate::{
    Error, Result,
    audio::{AudioFormat, Channels, SampleRate},
    video::VideoFormat,
};

use super::file_kind::{detect_mp4_file_kind, read_required_range};

/// デマルチプレクサから取り出した 1 サンプル分の情報 (借用を含まない所有形)
#[derive(Debug, Clone)]
pub(crate) struct SampleContext {
    pub(crate) track_kind: TrackKind,
    pub(crate) track_id: u32,
    pub(crate) timescale: u32,
    pub(crate) timestamp: u64,
    pub(crate) duration: u64,
    pub(crate) data_offset: u64,
    pub(crate) data_size: usize,
    pub(crate) keyframe: bool,
    pub(crate) composition_time_offset: Option<i64>,
    pub(crate) sample_entry: Option<SampleEntry>,
}

impl SampleContext {
    pub(crate) fn from_sample(sample: &Sample<'_>) -> Self {
        Self {
            track_kind: sample.track.kind,
            track_id: sample.track.track_id,
            timescale: sample.track.timescale.get(),
            timestamp: sample.timestamp,
            duration: sample.duration as u64,
            data_offset: sample.data_offset,
            data_size: sample.data_size,
            keyframe: sample.keyframe,
            composition_time_offset: sample.composition_time_offset,
            sample_entry: sample.sample_entry.cloned(),
        }
    }
}

/// 通常 MP4 / fMP4 の前方読みデマルチプレクサを吸収する enum
///
/// 前方読みに必要な操作 (`handle_input` / `tracks` / `next_sample`) だけを
/// 各バリアントへ match で委譲する。
enum DemuxerKind {
    Mp4(Mp4FileDemuxer),
    Fmp4(Fmp4FileDemuxer),
}

impl DemuxerKind {
    fn handle_input(&mut self, input: Input) {
        match self {
            DemuxerKind::Mp4(d) => d.handle_input(input),
            DemuxerKind::Fmp4(d) => d.handle_input(input),
        }
    }

    fn tracks(&mut self) -> std::result::Result<&[TrackInfo], DemuxError> {
        match self {
            DemuxerKind::Mp4(d) => d.tracks(),
            DemuxerKind::Fmp4(d) => d.tracks(),
        }
    }

    fn next_sample(&mut self) -> std::result::Result<Option<Sample<'_>>, DemuxError> {
        match self {
            DemuxerKind::Mp4(d) => d.next_sample(),
            DemuxerKind::Fmp4(d) => d.next_sample(),
        }
    }
}

/// 通常 MP4 / fMP4 を前方読みするデマルチプレクサ
///
/// `open()` 時にファイル種別を判定し、`moov` 読了まで初期化する。
/// `next_sample()` は fMP4 の追加入力要求 (`InputRequired`) を内部で解決する。
pub(crate) struct Mp4Demuxer {
    file: File,
    file_size: u64,
    path: PathBuf,
    inner: DemuxerKind,
    /// 直近に supply() でデマルチプレクサへ供給した入力範囲の開始位置。
    /// メディアフラグメント (moof + mdat) の処理中に失敗した場合、この位置が失敗フラグメントの
    /// moof 先頭と一致するため、is_media_fragment() の判定対象として使う。
    last_supply_offset: Option<u64>,
}

impl Mp4Demuxer {
    /// ファイルを開いて種別判定し、トラック情報を取得できる状態まで初期化する
    pub(crate) fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let kind = detect_mp4_file_kind(path)?;
        let file = File::open(path)
            .map_err(|e| Error::new(format!("Cannot open file {}: {e}", path.display())))?;
        let file_size = file
            .metadata()
            .map_err(|e| Error::new(format!("Cannot stat file {}: {e}", path.display())))?
            .len();
        let inner = match kind {
            Mp4FileKind::Mp4 => DemuxerKind::Mp4(Mp4FileDemuxer::new()),
            Mp4FileKind::FragmentedMp4 => DemuxerKind::Fmp4(Fmp4FileDemuxer::new()),
        };
        let mut this = Self {
            file,
            file_size,
            path: path.to_path_buf(),
            inner,
            last_supply_offset: None,
        };
        this.initialize()?;
        Ok(this)
    }

    /// トラック情報を取得できる (= moov 読了) 状態まで入力を供給する
    fn initialize(&mut self) -> Result<()> {
        loop {
            // tracks() の借用を即座に手放してから供給処理へ移る
            let outcome = self.inner.tracks().map(|_| ());
            match outcome {
                Ok(()) => return Ok(()),
                Err(DemuxError::InputRequired(required)) => self.supply(required)?,
                Err(e) => {
                    return Err(Error::new(format!(
                        "Demux error {}: {e}",
                        self.path.display()
                    )));
                }
            }
        }
    }

    /// 次のサンプルを返す。fMP4 の追加入力要求は内部で解決する。
    pub(crate) fn next_sample(&mut self) -> Result<Option<SampleContext>> {
        loop {
            // next_sample() が返す借用は各 arm 内で完結させ、supply() の &mut self 借用と衝突させない
            let required = match self.inner.next_sample() {
                Ok(Some(sample)) => return Ok(Some(SampleContext::from_sample(&sample))),
                Ok(None) => return Ok(None),
                Err(DemuxError::InputRequired(required)) => required,
                Err(e) => {
                    // 書き込み途中でクラッシュした hybrid fMP4 では、末尾のフラグメントが中途半端に
                    // 書かれてその処理が失敗することがある。これは hybrid mp4 では想定内のため、以下の
                    // いずれかなら、その原因を問わずエラーにせず、そこで読み取りを終了 (ストリーム終端)
                    // として扱う。moov / ftyp など初期化中の破損は引き続きエラーにする。
                    //   - 構造は揃っているメディアフラグメント (moof + mdat) の処理失敗
                    //     (典型例: trun が宣言するサンプルデータが mdat に収まらない)
                    //   - ボックスが EOF で途切れている (ヘッダが読めない / 宣言サイズがファイル末尾を超える)
                    if let Some(offset) = self.last_supply_offset
                        && (self.is_media_fragment(offset)?
                            || self.is_truncated_box_at_eof(offset)?)
                    {
                        tracing::warn!(
                            "Stopping at broken trailing fragment at offset {offset} in {}: {e}",
                            self.path.display()
                        );
                        return Ok(None);
                    }
                    return Err(Error::new(format!(
                        "Demux error {}: {e}",
                        self.path.display()
                    )));
                }
            };
            self.supply(required)?;
        }
    }

    /// 失敗した位置 (`moof_offset`) が、メディアフラグメント (`moof` + `mdat`) の先頭か判定する。
    ///
    /// `moof_offset` から始まるトップレベルボックスが `moof` であり、その直後に `mdat` が
    /// 続く場合に真を返す。メディアフラグメントの処理失敗 (クラッシュによる切り詰め等) を、
    /// 初期化中 (moov / ftyp) の破損と区別して扱うために使う。
    fn is_media_fragment(&mut self, moof_offset: u64) -> Result<bool> {
        let Some((moof_type, moof_size)) = self.read_box_header(moof_offset)? else {
            return Ok(false);
        };
        if &moof_type != b"moof" || moof_size == 0 {
            return Ok(false);
        }
        let Some(mdat_offset) = moof_offset.checked_add(moof_size) else {
            return Ok(false);
        };
        let Some((mdat_type, _mdat_size)) = self.read_box_header(mdat_offset)? else {
            return Ok(false);
        };
        Ok(&mdat_type == b"mdat")
    }

    /// `offset` から始まるトップレベルボックスが、EOF で途切れているか判定する。
    ///
    /// 書き込み途中のクラッシュでフラグメント (moof / mdat) のヘッダや本体が末尾で切れた場合に真を返す。
    /// ヘッダ (8 バイト) すら読めない場合、または宣言サイズがファイル末尾を超える場合を切り詰めとみなす。
    /// `size == 0` (ファイル末尾までを表す正規のボックス) は切り詰めではないので偽を返す。
    fn is_truncated_box_at_eof(&mut self, offset: u64) -> Result<bool> {
        let Some((_box_type, size)) = self.read_box_header(offset)? else {
            // ヘッダが読み切れない = 末尾で切り詰められている
            return Ok(true);
        };
        if size == 0 {
            return Ok(false);
        }
        Ok(offset.saturating_add(size) > self.file_size)
    }

    /// 指定位置のボックスヘッダ (4 バイトの type と box サイズ) を読み取る。
    /// `size == 1` の 64 bit サイズ、`size == 0` のファイル末尾までの両方に対応する。
    /// ヘッダを読み取れない (ファイル末尾を超える等) 場合は `None` を返す。
    fn read_box_header(&mut self, offset: u64) -> Result<Option<([u8; 4], u64)>> {
        if offset.saturating_add(8) > self.file_size {
            return Ok(None);
        }
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| Error::new(format!("Seek error {}: {e}", self.path.display())))?;
        let mut header = [0u8; 8];
        self.file
            .read_exact(&mut header)
            .map_err(|e| Error::new(format!("Read error {}: {e}", self.path.display())))?;
        let size32 = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        let box_type = [header[4], header[5], header[6], header[7]];
        let size = match size32 {
            // ファイル末尾までを表す
            0 => 0,
            // 直後の 8 バイトが 64 bit サイズ
            1 => {
                if offset.saturating_add(16) > self.file_size {
                    return Ok(None);
                }
                let mut ext = [0u8; 8];
                self.file
                    .read_exact(&mut ext)
                    .map_err(|e| Error::new(format!("Read error {}: {e}", self.path.display())))?;
                u64::from_be_bytes(ext)
            }
            n => u64::from(n),
        };
        Ok(Some((box_type, size)))
    }

    /// `RequiredInput` が示す範囲をファイルから読み込んでデマルチプレクサに供給する
    fn supply(&mut self, required: RequiredInput) -> Result<()> {
        let position = required.position;
        self.last_supply_offset = Some(position);
        let buf = read_required_range(&mut self.file, self.file_size, &self.path, required)?;
        self.inner.handle_input(Input {
            position,
            data: &buf,
        });
        Ok(())
    }

    /// サンプルデータをファイルから読み込む
    pub(crate) fn read_sample_data(
        &mut self,
        data_offset: u64,
        data_size: usize,
    ) -> Result<Vec<u8>> {
        read_sample_data_at(&mut self.file, &self.path, data_offset, data_size)
    }
}

/// timescale 単位の timestamp / duration を `Duration` に変換する
pub(crate) fn calculate_timestamps(
    timescale: u32,
    timestamp: u64,
    duration: u64,
) -> (Duration, Duration) {
    let timestamp = Duration::from_secs(timestamp) / timescale;
    let duration = Duration::from_secs(duration) / timescale;
    (timestamp, duration)
}

/// 壊れたファイル対策のサンプルデータサイズ上限 (100 MB)
///
/// 破損した stsz 等で極端に大きいサンプルサイズが指定されても、巨大なバッファを
/// 確保しないための上限。
const MAX_SAMPLE_DATA_SIZE: usize = 100 * 1024 * 1024;

/// サンプルデータをファイルの指定位置から読み込む
pub(crate) fn read_sample_data_at(
    file: &mut File,
    path: &Path,
    data_offset: u64,
    data_size: usize,
) -> Result<Vec<u8>> {
    // 破損ファイル対策: data_size は入力由来なので、巨大なバッファを確保する前に
    // 絶対上限 (MAX_SAMPLE_DATA_SIZE) とファイルサイズの両方で検証する。
    if data_size > MAX_SAMPLE_DATA_SIZE {
        return Err(Error::new(format!(
            "MP4 sample larger than maximum allowed size ({data_size} > {MAX_SAMPLE_DATA_SIZE}): {}",
            path.display()
        )));
    }
    let file_size = file
        .metadata()
        .map_err(|e| Error::new(format!("Cannot stat file {}: {e}", path.display())))?
        .len();
    if data_offset.saturating_add(data_size as u64) > file_size {
        return Err(Error::new(format!(
            "MP4 sample extends beyond end of file (offset {data_offset} + size {data_size} > {file_size}): {}",
            path.display()
        )));
    }

    let mut data = vec![0; data_size];
    file.seek(SeekFrom::Start(data_offset))
        .map_err(|e| Error::new(format!("Seek error {}: {e}", path.display())))?;
    file.read_exact(&mut data)
        .map_err(|e| Error::new(format!("Read error {}: {e}", path.display())))?;
    Ok(data)
}

/// 音声サンプルエントリーから format / channels / sample_rate を取り出す
pub(crate) fn audio_format_from_entry(
    sample_entry: &SampleEntry,
) -> Result<(AudioFormat, Channels, SampleRate)> {
    let (metadata, format) = match sample_entry {
        SampleEntry::Opus(b) => (&b.audio, AudioFormat::Opus),
        SampleEntry::Mp4a(b) => (&b.audio, AudioFormat::Aac),
        entry => {
            return Err(Error::new(format!("unsupported sample entry: {entry:?}")));
        }
    };
    let channels = Channels::from_u16(metadata.channelcount)?;
    let sample_rate = SampleRate::from_u16(metadata.samplerate.integer)?;
    Ok((format, channels, sample_rate))
}

/// 映像サンプルエントリーから format / width / height を取り出す
pub(crate) fn video_format_from_entry(
    sample_entry: &SampleEntry,
) -> Result<(VideoFormat, usize, usize)> {
    let (metadata, format) = match sample_entry {
        SampleEntry::Avc1(b) => (&b.visual, VideoFormat::H264),
        SampleEntry::Hev1(b) => (&b.visual, VideoFormat::H265),
        SampleEntry::Hvc1(b) => (&b.visual, VideoFormat::H265),
        SampleEntry::Vp08(b) => (&b.visual, VideoFormat::Vp8),
        SampleEntry::Vp09(b) => (&b.visual, VideoFormat::Vp9),
        SampleEntry::Av01(b) => (&b.visual, VideoFormat::Av1),
        entry => {
            return Err(Error::new(format!("unsupported sample entry: {entry:?}")));
        }
    };
    Ok((format, metadata.width as usize, metadata.height as usize))
}

/// AAC コーデックであることを確認する
pub(crate) fn is_aac_codec(esds_box: &shiguredo_mp4::boxes::EsdsBox) -> bool {
    // DecoderConfigDescriptor の object_type_indication が AAC を示しているかチェック
    // AAC LC は 0x40 (64)
    // AAC Main Profile は 0x41 (65)
    // AAC SSR は 0x42 (66)
    // AAC LTP は 0x43 (67)
    matches!(
        esds_box.es.dec_config_descr.object_type_indication,
        0x40..=0x43
    )
}

/// hisui が対応している音声コーデックのサンプルエントリーかどうかを判定する
pub(crate) fn is_supported_audio_entry(sample_entry: &SampleEntry) -> bool {
    match sample_entry {
        SampleEntry::Opus(_) => true,
        SampleEntry::Mp4a(mp4a) => is_aac_codec(&mp4a.esds_box),
        _ => false,
    }
}

/// hisui が対応している映像コーデックのサンプルエントリーかどうかを判定する
pub(crate) fn is_supported_video_entry(sample_entry: &SampleEntry) -> bool {
    matches!(
        sample_entry,
        SampleEntry::Avc1(_)
            | SampleEntry::Hev1(_)
            | SampleEntry::Hvc1(_)
            | SampleEntry::Vp08(_)
            | SampleEntry::Vp09(_)
            | SampleEntry::Av01(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MP4: &str = "testdata/red-320x320-h264-aac.mp4";
    const TEST_FMP4: &str = "testdata/red-320x320-h264-aac-fragmented.mp4";

    /// トップレベルボックスを走査して (type, offset, size) の一覧を返す
    fn top_level_boxes(data: &[u8]) -> Vec<([u8; 4], usize, usize)> {
        let mut boxes = Vec::new();
        let mut off = 0;
        while off + 8 <= data.len() {
            let size = u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
                as usize;
            let box_type = [data[off + 4], data[off + 5], data[off + 6], data[off + 7]];
            boxes.push((box_type, off, size));
            if size < 8 {
                break;
            }
            off += size;
        }
        boxes
    }

    /// Mp4Demuxer でサンプル数を数える
    fn count_samples(path: &Path) -> Result<usize> {
        let mut demuxer = Mp4Demuxer::open(path)?;
        let mut count = 0;
        while demuxer.next_sample()?.is_some() {
            count += 1;
        }
        Ok(count)
    }

    /// 一時ファイルにデータを書き出す
    fn write_temp(data: &[u8]) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut file = tempfile::Builder::new()
            .suffix(".mp4")
            .tempfile()
            .expect("一時ファイルを作成できること");
        file.write_all(data).expect("一時ファイルに書き込めること");
        file.flush().expect("flush できること");
        file
    }

    #[test]
    fn tolerates_truncated_trailing_fragment() {
        // 正常な fMP4 の末尾に「trun が宣言する以上に mdat が短い」壊れたフラグメントを追加し、
        // クラッシュで切り詰められた hybrid fMP4 を模す。
        // demuxer は壊れたフラグメントの手前までのサンプルを返し、エラーにせず終了すること。
        let data = std::fs::read(TEST_FMP4).expect("テスト用 fMP4 を読めること");
        let boxes = top_level_boxes(&data);
        let (_, moof_off, moof_size) = boxes
            .iter()
            .find(|(t, _, _)| t == b"moof")
            .copied()
            .expect("moof があること");
        let mdat_off = moof_off + moof_size;
        let (mdat_type, _, mdat_size) = boxes
            .iter()
            .find(|(_, off, _)| *off == mdat_off)
            .copied()
            .expect("moof の直後に mdat があること");
        assert_eq!(&mdat_type, b"mdat", "moof の直後は mdat であること");

        // 最初のフラグメント (ftyp + moov + moof + mdat) のみの正常ファイル
        let valid_only = &data[..mdat_off + mdat_size];
        let valid_file = write_temp(valid_only);
        let valid_count =
            count_samples(valid_file.path()).expect("正常 fMP4 のサンプルを読めること");
        assert!(valid_count > 0, "正常フラグメントからサンプルを読めること");

        // moof をコピーし、わずか 16 バイトしか持たない mdat を続ける壊れたフラグメントを作る。
        // default_base_is_moof のため moof の位置に依存せず、trun の宣言サイズが mdat を超える。
        let mut truncated = valid_only.to_vec();
        truncated.extend_from_slice(&data[moof_off..mdat_off]); // moof のコピー
        truncated.extend_from_slice(&24u32.to_be_bytes()); // 小さい mdat (ヘッダ 8 + 本体 16)
        truncated.extend_from_slice(b"mdat");
        truncated.extend_from_slice(&[0u8; 16]);
        let truncated_file = write_temp(&truncated);

        let truncated_count = count_samples(truncated_file.path())
            .expect("壊れたフラグメントでもエラーにならないこと");
        assert_eq!(
            truncated_count, valid_count,
            "壊れた末尾フラグメントの手前までのサンプルを返すこと"
        );
    }

    /// テスト用 fMP4 から「正常な単一フラグメント (ftyp+moov+moof+mdat)」と moof のコピー、
    /// およびその正常フラグメントから読めるサンプル数を取り出す。
    fn fragmented_fixture_parts() -> (Vec<u8>, Vec<u8>, usize) {
        let data = std::fs::read(TEST_FMP4).expect("テスト用 fMP4 を読めること");
        let boxes = top_level_boxes(&data);
        let (_, moof_off, moof_size) = boxes
            .iter()
            .find(|(t, _, _)| t == b"moof")
            .copied()
            .expect("moof があること");
        let mdat_off = moof_off + moof_size;
        let (mdat_type, _, mdat_size) = boxes
            .iter()
            .find(|(_, off, _)| *off == mdat_off)
            .copied()
            .expect("moof の直後に mdat があること");
        assert_eq!(&mdat_type, b"mdat", "moof の直後は mdat であること");

        let valid_only = data[..mdat_off + mdat_size].to_vec();
        let moof_bytes = data[moof_off..mdat_off].to_vec();
        let valid_file = write_temp(&valid_only);
        let valid_count =
            count_samples(valid_file.path()).expect("正常 fMP4 のサンプルを読めること");
        assert!(valid_count > 0, "正常フラグメントからサンプルを読めること");
        (valid_only, moof_bytes, valid_count)
    }

    #[test]
    fn tolerates_mdat_header_truncated_at_eof() {
        // 完全な moof の直後で、mdat ボックスヘッダの途中までしか書かれずに EOF になったケース。
        let (valid_only, moof_bytes, valid_count) = fragmented_fixture_parts();
        let mut truncated = valid_only;
        truncated.extend_from_slice(&moof_bytes); // 完全な moof
        truncated.extend_from_slice(&[0x00, 0x00, 0x27, 0x00]); // mdat ヘッダの途中 (4 バイトのみ)
        let file = write_temp(&truncated);

        let count =
            count_samples(file.path()).expect("ヘッダ途中の切り詰めでもエラーにならないこと");
        assert_eq!(
            count, valid_count,
            "壊れたフラグメントの手前までのサンプルを返すこと"
        );
    }

    #[test]
    fn tolerates_moof_header_truncated_at_eof() {
        // moof ボックスヘッダの途中までしか書かれずに EOF になったケース。
        let (valid_only, _moof_bytes, valid_count) = fragmented_fixture_parts();
        let mut truncated = valid_only;
        truncated.extend_from_slice(&[0x00, 0x00, 0x02, 0xdc]); // moof ヘッダの途中 (4 バイトのみ)
        let file = write_temp(&truncated);

        let count =
            count_samples(file.path()).expect("moof ヘッダ途中の切り詰めでもエラーにならないこと");
        assert_eq!(
            count, valid_count,
            "壊れたフラグメントの手前までのサンプルを返すこと"
        );
    }

    #[test]
    fn tolerates_moof_body_truncated_at_eof() {
        // moof ヘッダはサイズを宣言しているが、本体が途中で EOF になったケース。
        let (valid_only, moof_bytes, valid_count) = fragmented_fixture_parts();
        let mut truncated = valid_only;
        truncated.extend_from_slice(&moof_bytes[..100]); // moof 本体の途中まで
        let file = write_temp(&truncated);

        let count =
            count_samples(file.path()).expect("moof 本体途中の切り詰めでもエラーにならないこと");
        assert_eq!(
            count, valid_count,
            "壊れたフラグメントの手前までのサンプルを返すこと"
        );
    }

    #[test]
    fn errors_on_corrupted_moov() {
        // 初期化中 (moov) の破損は、末尾フラグメントの切り詰めとは異なりエラーにする
        // (壊れたファイルを正常終了として握り潰さない = 過剰許容にしない)。
        let mut data = std::fs::read(TEST_FMP4).expect("テスト用 fMP4 を読めること");
        let boxes = top_level_boxes(&data);
        let (_, moov_off, moov_size) = boxes
            .iter()
            .find(|(t, _, _)| t == b"moov")
            .copied()
            .expect("moov があること");
        // moov ペイロード (ヘッダ直後) を壊して decode を失敗させる
        let corrupt_end = (moov_off + 8 + 64).min(moov_off + moov_size);
        for byte in &mut data[moov_off + 8..corrupt_end] {
            *byte = 0xff;
        }
        let file = write_temp(&data);

        assert!(
            count_samples(file.path()).is_err(),
            "moov が壊れている場合はエラーになること (Ok(None) で握り潰さないこと)"
        );
    }

    #[test]
    fn is_media_fragment_distinguishes_moof_from_init() {
        let data = std::fs::read(TEST_FMP4).expect("テスト用 fMP4 を読めること");
        let boxes = top_level_boxes(&data);
        let (_, moof_off, _) = boxes
            .iter()
            .find(|(t, _, _)| t == b"moof")
            .copied()
            .expect("moof があること");

        let mut demuxer = Mp4Demuxer::open(TEST_FMP4).expect("fMP4 を開けること");
        assert!(
            demuxer
                .is_media_fragment(moof_off as u64)
                .expect("判定に成功すること"),
            "moof の位置はメディアフラグメントと判定されること"
        );
        assert!(
            !demuxer.is_media_fragment(0).expect("判定に成功すること"),
            "先頭 (ftyp) はメディアフラグメントと判定されないこと"
        );
    }

    #[test]
    fn read_sample_data_reads_requested_range() {
        let mut file = File::open(TEST_MP4).expect("テスト用 MP4 を開けること");
        let data = read_sample_data_at(&mut file, Path::new(TEST_MP4), 0, 8)
            .expect("ファイル範囲内の読み込みに成功すること");
        assert_eq!(data.len(), 8, "要求したサイズ分だけ読み込めること");
    }

    #[test]
    fn read_sample_data_rejects_size_beyond_file() {
        let mut file = File::open(TEST_MP4).expect("テスト用 MP4 を開けること");
        // ファイルサイズより大きい (ただし MAX_SAMPLE_DATA_SIZE 未満の) data_size は拒否する
        let result = read_sample_data_at(&mut file, Path::new(TEST_MP4), 0, 1_000_000);
        assert!(
            result.is_err(),
            "ファイル範囲を超える読み込みは拒否されること"
        );
    }

    #[test]
    fn read_sample_data_rejects_size_over_limit() {
        let mut file = File::open(TEST_MP4).expect("テスト用 MP4 を開けること");
        let result =
            read_sample_data_at(&mut file, Path::new(TEST_MP4), 0, MAX_SAMPLE_DATA_SIZE + 1);
        assert!(result.is_err(), "絶対上限を超える読み込みは拒否されること");
    }
}
