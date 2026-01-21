use std::{
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    num::NonZeroU32,
    path::Path,
    time::Duration,
};

use orfail::OrFail;
use shiguredo_mp4::{
    BoxHeader, Decode, TrackKind,
    aux::SampleTableAccessor,
    boxes::{HdlrBox, MoovBox, SampleEntry, StblBox, TrakBox},
    demux::Mp4FileDemuxer,
};

use crate::{
    audio::{AudioData, AudioFormat},
    metadata::SourceId,
    stats::{Mp4AudioReaderStats, Mp4VideoReaderStats, VideoResolution},
    video::{VideoFormat, VideoFrame},
};

#[derive(Debug)]
pub struct Mp4VideoReader2 {
    file: File,
    demuxer: Mp4FileDemuxer,
    source_id: SourceId,
    format: VideoFormat,
    width: usize,
    height: usize,
    stats: Mp4VideoReaderStats,
}

impl Mp4VideoReader2 {
    pub fn new<P: AsRef<Path>>(
        source_id: SourceId,
        path: P,
        stats: Mp4VideoReaderStats,
    ) -> orfail::Result<Self> {
        let mut file = File::open(&path)
            .or_fail_with(|e| format!("Cannot open file {}: {e}", path.as_ref().display()))?;
        let mut demuxer = Mp4FileDemuxer::new();

        // トラック情報を読み込む
        //
        // NOTE: fMP4 には未対応なので、これ以降で demuxer がファイル読み込みを要求することはない
        while let Some(required) = demuxer.required_input() {
            let size = required.size.or_fail_with(|()| {
                format!(
                    "MP4 file contains unexpected variable size box {}",
                    path.as_ref().display()
                )
            })?;
            let mut buf = vec![0; size];
            file.seek(SeekFrom::Start(required.position))
                .or_fail_with(|e| format!("Seek error {}: {e}", path.as_ref().display()))?;
            file.read_exact(&mut buf)
                .or_fail_with(|e| format!("Read error {}: {e}", path.as_ref().display()))?;
            let input = required.to_input(&buf);
            demuxer.handle_input(input);
        }

        Ok(Self {
            file,
            demuxer,
            source_id,
            stats,

            // 後で更新されるので適当な初期値を設定しておく
            format: VideoFormat::Vp8,
            width: 0,
            height: 0,
        })
    }

    pub fn stats(&self) -> &Mp4VideoReaderStats {
        &self.stats
    }

    fn next_frame(&mut self) -> orfail::Result<Option<VideoFrame>> {
        let sample = 'next_sample: loop {
            match self
                .demuxer
                .next_sample()
                .or_fail_with(|e| format!("Read sample error: {e}"))?
            {
                None => return Ok(None),
                Some(sample) if sample.track.kind != TrackKind::Video => {}
                Some(sample) => break 'next_sample sample,
            }
        };

        let sample_entry = sample.sample_entry.cloned();
        if let Some(sample_entry) = &sample_entry {
            // 新しいサンプルエントリーが来たのでハンドリングする
            let (metadata, format) = match sample_entry {
                SampleEntry::Avc1(b) => (&b.visual, VideoFormat::H264),
                SampleEntry::Hev1(b) => (&b.visual, VideoFormat::H265),
                SampleEntry::Hvc1(b) => (&b.visual, VideoFormat::H265),
                SampleEntry::Vp08(b) => (&b.visual, VideoFormat::Vp8),
                SampleEntry::Vp09(b) => (&b.visual, VideoFormat::Vp9),
                SampleEntry::Av01(b) => (&b.visual, VideoFormat::Av1),
                entry => {
                    return Err(orfail::Failure::new(format!(
                        "unsupported sample entry: {entry:?}"
                    )));
                }
            };

            self.format = format;
            self.width = metadata.width as usize;
            self.height = metadata.height as usize;
        }

        // サンプルデータを読み込む
        let mut data = vec![0; sample.data_size];
        self.file
            .seek(SeekFrom::Start(sample.data_offset))
            .or_fail_with(|e| format!("Seek error: {e}"))?;
        self.file
            .read_exact(&mut data)
            .or_fail_with(|e| format!("Read error: {e}"))?;

        // タイムスタンプを計算する
        let timescale = sample.track.timescale.get();
        let timestamp = Duration::from_secs(sample.timestamp) / timescale;
        let duration = Duration::from_secs(sample.duration as u64) / timescale;

        // 統計値を更新する
        self.stats.total_sample_count.add(1);
        self.stats.total_track_duration.set(timestamp + duration);
        if self.stats.codec.get().is_none()
            && let Some(name) = self.format.codec_name()
        {
            self.stats.codec.set(name);
        }
        self.stats.resolutions.insert(VideoResolution {
            width: self.width,
            height: self.height,
        });

        Ok(Some(VideoFrame {
            source_id: Some(self.source_id.clone()),
            sample_entry,
            data,
            format: self.format,
            keyframe: sample.keyframe,
            width: self.width,
            height: self.height,
            timestamp,
            duration,
        }))
    }
}

impl Iterator for Mp4VideoReader2 {
    type Item = orfail::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_frame().or_fail().transpose()
    }
}

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

    fn find_trak_box<R: Read + Seek>(mut reader: R) -> orfail::Result<Option<TrakBox>> {
        let moov = find_moov_box(&mut reader).or_fail()?;
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
            SampleEntry::Hvc1(b) => (&b.visual, VideoFormat::H265),
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

        self.stats.total_sample_count.add(1);
        self.stats.total_track_duration.set(timestamp + duration);
        if self.stats.codec.get().is_none()
            && let Some(name) = format.codec_name()
        {
            self.stats.codec.set(name);
        }
        self.stats.resolutions.insert(VideoResolution {
            width: resolution.0 as usize,
            height: resolution.1 as usize,
        });

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
            width: metadata.width as usize,
            height: metadata.height as usize,
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

    fn find_trak_box<R: Read + Seek>(mut reader: R) -> orfail::Result<Option<TrakBox>> {
        let moov = find_moov_box(&mut reader).or_fail()?;
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

fn read_next_box_header<R: Read + Seek>(reader: &mut R) -> orfail::Result<(BoxHeader, Vec<u8>)> {
    let mut buf = vec![0; BoxHeader::MIN_SIZE];
    reader.read_exact(&mut buf).or_fail()?;

    loop {
        match BoxHeader::decode(&buf) {
            Ok((header, _)) => return Ok((header, buf)),
            Err(e) if e.kind == shiguredo_mp4::ErrorKind::InsufficientBuffer => {
                (buf.len() <= BoxHeader::MAX_SIZE)
                    .or_fail_with(|()| "unexpected EOF".to_owned())?;

                let mut byte = [0];
                reader.read_exact(&mut byte).or_fail()?;
                buf.push(byte[0]);
            }
            Err(e) => return Err(e).or_fail(),
        }
    }
}

fn find_moov_box<R: Read + Seek>(reader: &mut R) -> orfail::Result<MoovBox> {
    loop {
        let (header, mut buf) = read_next_box_header(reader).or_fail()?;
        if header.box_type != MoovBox::TYPE {
            let payload_size = header.box_size.get() as i64 - buf.len() as i64;
            reader.seek(SeekFrom::Current(payload_size)).or_fail()?;
            continue;
        }

        let header_len = buf.len();
        buf.resize(header.box_size.get() as usize, 0);

        reader.read_exact(&mut buf[header_len..]).or_fail()?;

        let (moov, _size) = MoovBox::decode(&buf).or_fail()?;
        return Ok(moov);
    }
}
