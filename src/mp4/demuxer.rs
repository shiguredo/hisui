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
    pub track_kind: TrackKind,
    pub track_id: u32,
    pub timescale: u32,
    pub timestamp: u64,
    pub duration: u64,
    pub data_offset: u64,
    pub data_size: usize,
    pub keyframe: bool,
    pub composition_time_offset: Option<i64>,
    pub sample_entry: Option<SampleEntry>,
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

/// 前方読みデマルチプレクサの共通操作 (内部用)
///
/// `Mp4FileDemuxer` と `Fmp4FileDemuxer` を同一インターフェースで扱うための trait。
trait ForwardDemuxer {
    fn handle_input(&mut self, input: Input);
    fn tracks(&mut self) -> std::result::Result<&[TrackInfo], DemuxError>;
    fn next_sample(&mut self) -> std::result::Result<Option<Sample<'_>>, DemuxError>;
}

impl ForwardDemuxer for Mp4FileDemuxer {
    fn handle_input(&mut self, input: Input) {
        Mp4FileDemuxer::handle_input(self, input);
    }
    fn tracks(&mut self) -> std::result::Result<&[TrackInfo], DemuxError> {
        Mp4FileDemuxer::tracks(self)
    }
    fn next_sample(&mut self) -> std::result::Result<Option<Sample<'_>>, DemuxError> {
        Mp4FileDemuxer::next_sample(self)
    }
}

impl ForwardDemuxer for Fmp4FileDemuxer {
    fn handle_input(&mut self, input: Input) {
        Fmp4FileDemuxer::handle_input(self, input);
    }
    fn tracks(&mut self) -> std::result::Result<&[TrackInfo], DemuxError> {
        Fmp4FileDemuxer::tracks(self)
    }
    fn next_sample(&mut self) -> std::result::Result<Option<Sample<'_>>, DemuxError> {
        Fmp4FileDemuxer::next_sample(self)
    }
}

enum DemuxerKind {
    Mp4(Mp4FileDemuxer),
    Fmp4(Fmp4FileDemuxer),
}

impl DemuxerKind {
    fn as_dyn(&mut self) -> &mut dyn ForwardDemuxer {
        match self {
            DemuxerKind::Mp4(d) => d,
            DemuxerKind::Fmp4(d) => d,
        }
    }
}

/// 前方読みステップの結果 (借用を含まない所有形)
enum Step {
    Sample(Box<SampleContext>),
    Eof,
    NeedInput(RequiredInput),
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
            let outcome: std::result::Result<(), DemuxError> = match self.inner.as_dyn().tracks() {
                Ok(_) => Ok(()),
                Err(e) => Err(e),
            };
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
            match self.step()? {
                Step::Sample(ctx) => return Ok(Some(*ctx)),
                Step::Eof => return Ok(None),
                Step::NeedInput(required) => self.supply(required)?,
            }
        }
    }

    /// デマルチプレクサを 1 ステップ進める。借用は関数内で完結させる。
    fn step(&mut self) -> Result<Step> {
        let outcome: std::result::Result<Step, DemuxError> = match self.inner.as_dyn().next_sample()
        {
            Ok(Some(sample)) => Ok(Step::Sample(Box::new(SampleContext::from_sample(&sample)))),
            Ok(None) => Ok(Step::Eof),
            Err(DemuxError::InputRequired(required)) => Ok(Step::NeedInput(required)),
            Err(e) => Err(e),
        };
        outcome.map_err(|e| Error::new(format!("Demux error {}: {e}", self.path.display())))
    }

    /// `RequiredInput` が示す範囲をファイルから読み込んでデマルチプレクサに供給する
    fn supply(&mut self, required: RequiredInput) -> Result<()> {
        let position = required.position;
        let buf = read_required_range(&mut self.file, self.file_size, &self.path, required)?;
        self.inner.as_dyn().handle_input(Input {
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
