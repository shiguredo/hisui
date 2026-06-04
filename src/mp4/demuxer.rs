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

use super::file_kind::{MAX_BUF_SIZE, detect_mp4_file_kind, read_required_range};

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
                    return Err(Error::new(format!(
                        "Demux error {}: {e}",
                        self.path.display()
                    )));
                }
            };
            self.supply(required)?;
        }
    }

    /// `RequiredInput` が示す範囲をファイルから読み込んでデマルチプレクサに供給する
    fn supply(&mut self, required: RequiredInput) -> Result<()> {
        let position = required.position;
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

/// サンプルデータをファイルの指定位置から読み込む
pub(crate) fn read_sample_data_at(
    file: &mut File,
    path: &Path,
    data_offset: u64,
    data_size: usize,
) -> Result<Vec<u8>> {
    // 破損ファイル対策: data_size は入力由来なので、巨大なバッファを確保する前に
    // 絶対上限 (MAX_BUF_SIZE) とファイルサイズの両方で検証する。
    if data_size > MAX_BUF_SIZE {
        return Err(Error::new(format!(
            "MP4 sample larger than maximum allowed size ({data_size} > {MAX_BUF_SIZE}): {}",
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
        // ファイルサイズより大きい (ただし MAX_BUF_SIZE 未満の) data_size は拒否する
        let result = read_sample_data_at(&mut file, Path::new(TEST_MP4), 0, 1_000_000);
        assert!(
            result.is_err(),
            "ファイル範囲を超える読み込みは拒否されること"
        );
    }

    #[test]
    fn read_sample_data_rejects_size_over_max_buf_size() {
        let mut file = File::open(TEST_MP4).expect("テスト用 MP4 を開けること");
        let result = read_sample_data_at(&mut file, Path::new(TEST_MP4), 0, MAX_BUF_SIZE + 1);
        assert!(result.is_err(), "絶対上限を超える読み込みは拒否されること");
    }
}
