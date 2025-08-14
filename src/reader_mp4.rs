use std::{
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    num::NonZeroU32,
    path::Path,
    time::{Duration, Instant},
};

use orfail::OrFail;
use shiguredo_mp4::{
    Decode, Either,
    aux::SampleTableAccessor,
    boxes::{FtypBox, HdlrBox, IgnoredBox, MoovBox, SampleEntry, StblBox, TrakBox},
};

use crate::{
    audio::{AudioData, AudioFormat},
    metadata::SourceId,
    stats::{Mp4AudioReaderStats, Mp4VideoReaderStats, Seconds},
    types::{CodecName, EvenUsize},
    video::{VideoFormat, VideoFrame},
};

#[derive(Debug)]
pub struct Mp4VideoReader {
    // ビデオトラックが存在しない場合は None になる
    inner: Option<Mp4VideoReaderInner>,

    // ビデオトラックが存在しない時の統計値
    default_stats: Mp4VideoReaderStats,
}

impl Mp4VideoReader {
    pub fn new<P: AsRef<Path>>(source_id: SourceId, path: P) -> orfail::Result<Self> {
        let default_stats = Mp4VideoReaderStats {
            input_file: path.as_ref().canonicalize().or_fail_with(|e| {
                format!(
                    "failed to canonicalize path {}: {e}",
                    path.as_ref().display()
                )
            })?,
            ..Default::default()
        };

        let inner = Mp4VideoReaderInner::new(source_id, path).or_fail()?;

        Ok(Self {
            inner,
            default_stats,
        })
    }

    pub fn stats(&self) -> &Mp4VideoReaderStats {
        self.inner
            .as_ref()
            .map_or(&self.default_stats, |x| &x.stats)
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
    fn new<P: AsRef<Path>>(source_id: SourceId, path: P) -> orfail::Result<Option<Self>> {
        let start_time = Instant::now();
        let file = File::open(&path)
            .or_fail_with(|e| format!("Cannot open file {}: {e}", path.as_ref().display()))?;
        let mut file = BufReader::new(file);
        let Some(trak) = Self::find_trak_box(&mut file).or_fail()? else {
            return Ok(None);
        };
        let table = SampleTableAccessor::new(trak.mdia_box.minf_box.stbl_box.clone()).or_fail()?;

        file.seek(SeekFrom::Start(0)).or_fail()?;

        let stats = Mp4VideoReaderStats {
            input_file: path.as_ref().canonicalize().or_fail_with(|e| {
                format!(
                    "failed to canonicalize path {}: {e}",
                    path.as_ref().display()
                )
            })?,
            total_processing_seconds: Seconds::new(start_time.elapsed()),
            ..Default::default()
        };
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

        let sample_entry = sample.chunk().sample_entry();
        let (metadata, format) = match sample_entry {
            SampleEntry::Avc1(b) => (&b.visual, VideoFormat::H264),
            SampleEntry::Hev1(b) => (&b.visual, VideoFormat::H265),
            SampleEntry::Vp08(b) => (&b.visual, VideoFormat::Vp8),
            SampleEntry::Vp09(b) => (&b.visual, VideoFormat::Vp9),
            SampleEntry::Av01(b) => (&b.visual, VideoFormat::Av1),
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
        let resolution = (metadata.width, metadata.height);

        self.stats.total_sample_count += 1;
        self.stats.total_track_seconds = Seconds::new(timestamp + duration);
        if self.stats.codec.is_none() {
            self.stats.codec = format.codec_name();
        }
        if self
            .stats
            .resolutions
            .last()
            .is_none_or(|&r| r != resolution)
        {
            self.stats.resolutions.push(resolution);
        }

        let (Some(width), Some(height)) = (
            EvenUsize::new(metadata.width as usize),
            EvenUsize::new(metadata.height as usize),
        ) else {
            // [NOTE] 奇数入力はもしかしたら対応する必要があるかもしれない（ただ結構大変）
            //        もし対応するならデコード後に端の画像を切り捨ててしまうのが簡単そう
            return Some(Err(orfail::Failure::new(format!(
                "odd video resolution is unsupported: {}x{}",
                metadata.width, metadata.height,
            ))));
        };

        Some(Ok(VideoFrame {
            source_id: Some(self.source_id.clone()),
            sample_entry: if self
                .prev_sample_entry
                .as_ref()
                .is_none_or(|entry| entry != sample_entry)
            {
                self.prev_sample_entry = Some(sample_entry.clone());
                Some(sample_entry.clone())
            } else {
                None
            },
            data,
            format,
            keyframe: sample.is_sync_sample(),
            width,
            height,
            timestamp,
            duration,
        }))
    }
}

impl Iterator for Mp4VideoReaderInner {
    type Item = orfail::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        let (result, elapsed) = Seconds::elapsed(|| self.next_video_frame());
        self.stats.total_processing_seconds += elapsed;
        if matches!(result, Some(Err(_))) {
            self.stats.error = true;
        }
        result
    }
}

#[derive(Debug)]
pub struct Mp4AudioReader {
    // 音声トラックが存在しない場合は None になる
    inner: Option<Mp4AudioReaderInner>,

    // 音声トラックが存在しない時の統計値
    default_stats: Mp4AudioReaderStats,
}

impl Mp4AudioReader {
    pub fn new<P: AsRef<Path>>(source_id: SourceId, path: P) -> orfail::Result<Self> {
        let default_stats = Mp4AudioReaderStats {
            input_file: path.as_ref().to_path_buf(),
            ..Default::default()
        };

        let inner = Mp4AudioReaderInner::new(source_id, path).or_fail()?;

        Ok(Self {
            inner,
            default_stats,
        })
    }

    pub fn stats(&self) -> &Mp4AudioReaderStats {
        self.inner
            .as_ref()
            .map_or(&self.default_stats, |x| &x.stats)
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
    fn new<P: AsRef<Path>>(source_id: SourceId, path: P) -> orfail::Result<Option<Self>> {
        let start_time = Instant::now();
        let file = File::open(&path)
            .or_fail_with(|e| format!("Cannot open file {}: {e}", path.as_ref().display()))?;
        let mut file = BufReader::new(file);
        let Some(trak) = Self::find_trak_box(&mut file).or_fail()? else {
            return Ok(None);
        };
        let table = SampleTableAccessor::new(trak.mdia_box.minf_box.stbl_box.clone()).or_fail()?;

        file.seek(SeekFrom::Start(0)).or_fail()?;

        let stats = Mp4AudioReaderStats {
            input_file: path.as_ref().canonicalize().or_fail()?,
            codec: Some(CodecName::Opus),
            total_processing_seconds: Seconds::new(start_time.elapsed()),
            ..Default::default()
        };
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

        self.stats.total_sample_count.increment();
        self.stats.total_track_seconds = Seconds::new(timestamp + duration);

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
        let (result, elapsed) = Seconds::elapsed(|| self.next_audio_data());
        self.stats.total_processing_seconds += elapsed;
        if matches!(result, Some(Err(_))) {
            self.stats.error.set(true);
        }
        result
    }
}
