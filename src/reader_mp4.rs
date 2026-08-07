use std::{
    borrow::Cow,
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    num::NonZeroU32,
    path::Path,
    time::Duration,
};

use orfail::OrFail;
use shiguredo_mp4::{
    BoxType, Decode, Either,
    aux::SampleTableAccessor,
    boxes::{FtypBox, HdlrBox, IgnoredBox, MoovBox, SampleEntry, StblBox, TrakBox},
};

use crate::{
    audio::{AudioData, AudioFormat},
    metadata::SourceId,
    stats::{Mp4AudioReaderStats, Mp4VideoReaderStats, VideoResolution},
    video::{VideoFormat, VideoFrame},
    video_h265::hev1_box_from_hvc1_unknown,
};

#[derive(Debug)]
pub struct Mp4VideoReader {
    // ビデオトラックが存在しない場合は None になる
    inner: Option<Mp4VideoReaderInner>,
    stats: Mp4VideoReaderStats,
}

impl Mp4VideoReader {
    pub fn new<P: AsRef<Path>>(
        source_id: SourceId,
        path: P,
        stats: Mp4VideoReaderStats,
    ) -> orfail::Result<Self> {
        let inner = Mp4VideoReaderInner::new(source_id, path, stats.clone()).or_fail()?;
        Ok(Self { inner, stats })
    }

    pub fn stats(&self) -> &Mp4VideoReaderStats {
        &self.stats
    }
}

impl Iterator for Mp4VideoReader {
    type Item = orfail::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.as_mut()?.next()
    }
}

#[derive(Debug)]
pub struct Mp4VideoReaderInner {
    file: BufReader<File>,
    source_id: SourceId,
    table: SampleTableAccessor<StblBox>,
    timescale: NonZeroU32,
    next_sample_index: NonZeroU32,
    prev_sample_entry: Option<SampleEntry>,
    stats: Mp4VideoReaderStats,
}

impl Mp4VideoReaderInner {
    fn new<P: AsRef<Path>>(
        source_id: SourceId,
        path: P,
        stats: Mp4VideoReaderStats,
    ) -> orfail::Result<Option<Self>> {
        let file = File::open(&path)
            .or_fail_with(|e| format!("Cannot open file {}: {e}", path.as_ref().display()))?;
        let mut file = BufReader::new(file);
        let Some(trak) = Self::find_trak_box(&mut file).or_fail()? else {
            return Ok(None);
        };
        let table = SampleTableAccessor::new(trak.mdia_box.minf_box.stbl_box.clone()).or_fail()?;

        file.seek(SeekFrom::Start(0)).or_fail()?;

        Ok(Some(Self {
            file,
            source_id,
            table,
            timescale: trak.mdia_box.mdhd_box.timescale,
            next_sample_index: NonZeroU32::MIN,
            prev_sample_entry: None,
            stats,
        }))
    }

    fn find_trak_box<R: Read>(mut reader: R) -> orfail::Result<Option<TrakBox>> {
        let _ = FtypBox::decode(&mut reader).or_fail()?;
        let moov: MoovBox = loop {
            if let Either::A(moov) =
                IgnoredBox::decode_or_ignore(&mut reader, |t| t == MoovBox::TYPE).or_fail()?
            {
                break moov;
            }
        };
        Ok(moov
            .trak_boxes
            .into_iter()
            .find(|t| t.mdia_box.hdlr_box.handler_type == HdlrBox::HANDLER_TYPE_VIDE))
    }

    fn next_video_frame(&mut self) -> Option<orfail::Result<VideoFrame>> {
        let sample = self.table.get_sample(self.next_sample_index)?;
        self.next_sample_index = self.next_sample_index.checked_add(1)?;

        let raw_sample_entry = sample.chunk().sample_entry();
        // hisui が使う shiguredo_mp4 (=2025.2.0) は hvc1 に未対応で Unknown として parse される
        // ここで hev1 相当に変換して後続の H.265 処理を共通化する
        let sample_entry: Cow<SampleEntry> = match raw_sample_entry {
            SampleEntry::Unknown(b) if b.box_type == BoxType::Normal(*b"hvc1") => {
                match hev1_box_from_hvc1_unknown(b) {
                    Ok(hev1) => Cow::Owned(SampleEntry::Hev1(hev1)),
                    Err(e) => return Some(Err(e)),
                }
            }
            _ => Cow::Borrowed(raw_sample_entry),
        };

        let (width, height, format) = match sample_entry.as_ref() {
            SampleEntry::Avc1(b) => (b.visual.width, b.visual.height, VideoFormat::H264),
            SampleEntry::Hev1(b) => (b.visual.width, b.visual.height, VideoFormat::H265),
            SampleEntry::Vp08(b) => (b.visual.width, b.visual.height, VideoFormat::Vp8),
            SampleEntry::Vp09(b) => (b.visual.width, b.visual.height, VideoFormat::Vp9),
            SampleEntry::Av01(b) => (b.visual.width, b.visual.height, VideoFormat::Av1),
            entry => {
                return Some(Err(orfail::Failure::new(format!(
                    "unsupported sample entry: {entry:?}"
                ))));
            }
        };

        if let Err(e) = self
            .file
            .seek(SeekFrom::Start(sample.data_offset()))
            .or_fail()
        {
            return Some(Err(e));
        }

        let mut data = vec![0; sample.data_size() as usize];
        if let Err(e) = self.file.read_exact(&mut data).or_fail() {
            return Some(Err(e));
        }

        let timestamp = Duration::from_secs(sample.timestamp()) / self.timescale.get();
        let duration = Duration::from_secs(sample.duration() as u64) / self.timescale.get();

        self.stats.total_sample_count.add(1);
        self.stats.total_track_duration.set(timestamp + duration);
        if self.stats.codec.get().is_none()
            && let Some(name) = format.codec_name()
        {
            self.stats.codec.set(name);
        }
        self.stats.resolutions.insert(VideoResolution {
            width: width as usize,
            height: height as usize,
        });

        let sample_entry_out = if self
            .prev_sample_entry
            .as_ref()
            .is_none_or(|entry| entry != sample_entry.as_ref())
        {
            let owned = sample_entry.into_owned();
            self.prev_sample_entry = Some(owned.clone());
            Some(owned)
        } else {
            None
        };

        Some(Ok(VideoFrame {
            source_id: Some(self.source_id.clone()),
            sample_entry: sample_entry_out,
            data,
            format,
            keyframe: sample.is_sync_sample(),
            width: width as usize,
            height: height as usize,
            timestamp,
            duration,
        }))
    }
}

impl Iterator for Mp4VideoReaderInner {
    type Item = orfail::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_video_frame()
    }
}

#[derive(Debug)]
pub struct Mp4AudioReader {
    // 音声トラックが存在しない場合は None になる
    inner: Option<Mp4AudioReaderInner>,
    stats: Mp4AudioReaderStats,
}

impl Mp4AudioReader {
    pub fn new<P: AsRef<Path>>(
        source_id: SourceId,
        path: P,
        stats: Mp4AudioReaderStats,
    ) -> orfail::Result<Self> {
        let inner = Mp4AudioReaderInner::new(source_id, path, stats.clone()).or_fail()?;
        Ok(Self { inner, stats })
    }

    pub fn stats(&self) -> &Mp4AudioReaderStats {
        &self.stats
    }
}

impl Iterator for Mp4AudioReader {
    type Item = orfail::Result<AudioData>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.as_mut()?.next()
    }
}

#[derive(Debug)]
pub struct Mp4AudioReaderInner {
    file: BufReader<File>,
    source_id: SourceId,
    table: SampleTableAccessor<StblBox>,
    timescale: NonZeroU32,
    next_sample_index: NonZeroU32,
    stats: Mp4AudioReaderStats,
}

impl Mp4AudioReaderInner {
    fn new<P: AsRef<Path>>(
        source_id: SourceId,
        path: P,
        stats: Mp4AudioReaderStats,
    ) -> orfail::Result<Option<Self>> {
        let file = File::open(&path)
            .or_fail_with(|e| format!("Cannot open file {}: {e}", path.as_ref().display()))?;
        let mut file = BufReader::new(file);
        let Some(trak) = Self::find_trak_box(&mut file).or_fail()? else {
            return Ok(None);
        };
        let table = SampleTableAccessor::new(trak.mdia_box.minf_box.stbl_box.clone()).or_fail()?;

        file.seek(SeekFrom::Start(0)).or_fail()?;

        Ok(Some(Self {
            source_id,
            file,
            table,
            timescale: trak.mdia_box.mdhd_box.timescale,
            next_sample_index: NonZeroU32::MIN,
            stats,
        }))
    }

    fn find_trak_box<R: Read>(mut reader: R) -> orfail::Result<Option<TrakBox>> {
        let _ = FtypBox::decode(&mut reader).or_fail()?;
        let moov: MoovBox = loop {
            if let Either::A(moov) =
                IgnoredBox::decode_or_ignore(&mut reader, |t| t == MoovBox::TYPE).or_fail()?
            {
                break moov;
            }
        };
        Ok(moov
            .trak_boxes
            .into_iter()
            .find(|t| t.mdia_box.hdlr_box.handler_type == HdlrBox::HANDLER_TYPE_SOUN))
    }

    fn next_audio_data(&mut self) -> Option<orfail::Result<AudioData>> {
        let sample = self.table.get_sample(self.next_sample_index)?;
        self.next_sample_index = self.next_sample_index.checked_add(1)?;

        let sample_entry = sample.chunk().sample_entry();
        let (metadata, format) = match &sample_entry {
            SampleEntry::Opus(b) => (&b.audio, AudioFormat::Opus),
            entry => {
                return Some(Err(orfail::Failure::new(format!(
                    "unsupported sample entry: {entry:?}"
                ))));
            }
        };

        if let Err(e) = self
            .file
            .seek(SeekFrom::Start(sample.data_offset()))
            .or_fail()
        {
            return Some(Err(e));
        }

        let mut data = vec![0; sample.data_size() as usize];
        if let Err(e) = self.file.read_exact(&mut data).or_fail() {
            return Some(Err(e));
        }

        let timestamp = Duration::from_secs(sample.timestamp()) / self.timescale.get();
        let duration = Duration::from_secs(sample.duration() as u64) / self.timescale.get();

        self.stats.total_sample_count.add(1);
        self.stats.total_track_duration.set(timestamp + duration);

        Some(Ok(AudioData {
            source_id: Some(self.source_id.clone()),
            data,
            format,
            sample_entry: Some(sample_entry.clone()),

            // [NOTE]
            // 一応、コンテナで指定された値を設定しているけど、
            // ここの値はあまり信用できないので、`AudioData` 処理側は、
            // 実際のペイロードの値を参照する想定
            stereo: metadata.channelcount != 1,

            sample_rate: metadata.samplerate.integer,
            timestamp,
            duration,
        }))
    }
}

impl Iterator for Mp4AudioReaderInner {
    type Item = orfail::Result<AudioData>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_audio_data()
    }
}
