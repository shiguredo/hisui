use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use orfail::OrFail;
use shiguredo_mp4::{
    TrackKind,
    demux::{DemuxError, Input, Mp4FileDemuxer, Sample},
};

use crate::{
    audio::{AudioData, AudioFormat},
    metadata::SourceId,
    stats::{Mp4AudioReaderStats, Mp4VideoReaderStats},
    video::{VideoFormat, VideoFrame},
};

#[derive(Debug)]
pub struct Mp4VideoReader {
    file: File,
    demuxer: Mp4FileDemuxer,
    source_id: SourceId,
    stats: Mp4VideoReaderStats,
}

impl Mp4VideoReader {
    pub fn new<P: AsRef<Path>>(
        source_id: SourceId,
        path: P,
        stats: Mp4VideoReaderStats,
    ) -> orfail::Result<Self> {
        let file = File::open(&path)
            .or_fail_with(|e| format!("Cannot open file {}: {e}", path.as_ref().display()))?;
        Ok(Self {
            file,
            demuxer: Mp4FileDemuxer::new(),
            source_id,
            stats,
        })
    }

    pub fn stats(&self) -> &Mp4VideoReaderStats {
        &self.stats
    }

    fn with_io<F, T>(&mut self, f: F) -> orfail::Result<T>
    where
        F: Fn(&mut Self) -> Result<T, DemuxError>,
    {
        loop {
            match f(self) {
                Err(DemuxError::NeedInput {
                    position,
                    size: Some(size),
                }) => {
                    let mut data = vec![0; size];
                    self.file.seek(SeekFrom::Start(position)).or_fail()?;
                    self.file.read_exact(&mut data).or_fail()?;
                    self.demuxer.handle_input(Input {
                        position,
                        data: &data,
                    });
                }
                other => return Ok(other.or_fail()?),
            }
        }
    }

    fn next_sample<'a>(
        demuxer: &'a mut Mp4FileDemuxer,
        file: &mut File,
    ) -> orfail::Result<Option<Sample<'a>>> {
        let mut data = Vec::new();
        let mut input = Input {
            position: 0,
            data: &data,
        };
        while let Err(DemuxError::NeedInput {
            position,
            size: Some(size),
        }) = demuxer.handle_input(input)
        {
            data.resize(size, 0);
            file.seek(SeekFrom::Start(position)).or_fail()?;
            file.read_exact(&mut data).or_fail()?;
            input = Input {
                position,
                data: &data,
            };
        }

        // ここは常に成功するはず
        demuxer.next_sample().or_fail()
    }

    fn next_frame(&mut self) -> orfail::Result<Option<VideoFrame>> {
        while let Some(sample) = Self::next_sample(&mut self.demuxer, &mut self.file).or_fail()? {
            // ビデオトラックのみを処理
            if sample.track.kind != TrackKind::Video {
                continue;
            }

            let (format, metadata) = match sample.sample_entry {
                shiguredo_mp4::boxes::SampleEntry::Avc1(b) => (VideoFormat::H264, &b.visual),
                shiguredo_mp4::boxes::SampleEntry::Hev1(b) => (VideoFormat::H265, &b.visual),
                shiguredo_mp4::boxes::SampleEntry::Vp08(b) => (VideoFormat::Vp8, &b.visual),
                shiguredo_mp4::boxes::SampleEntry::Vp09(b) => (VideoFormat::Vp9, &b.visual),
                shiguredo_mp4::boxes::SampleEntry::Av01(b) => (VideoFormat::Av1, &b.visual),
                entry => {
                    return Err(orfail::Failure::new(format!(
                        "unsupported sample entry: {entry:?}"
                    )));
                }
            };
            let keyframe = sample.keyframe;
            let sample_entry = keyframe.then(|| sample.sample_entry.clone());
            let width = metadata.width as usize;
            let height = metadata.height as usize;
            let timestamp = sample.timestamp();
            let duration = sample.duration();
            let data_offset = sample.data_offset;
            let data_size = sample.data_size;

            let mut data = vec![0; data_size];
            self.file.seek(SeekFrom::Start(data_offset)).or_fail()?;
            self.file.read_exact(&mut data).or_fail()?;

            return Ok(Some(VideoFrame {
                source_id: Some(self.source_id.clone()),
                sample_entry,
                data,
                format,
                keyframe,
                width,
                height,
                timestamp,
                duration,
            }));
        }
        Ok(None)
    }
}

impl Iterator for Mp4VideoReader {
    type Item = orfail::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_frame().transpose()
    }
}

#[derive(Debug)]
pub struct Mp4AudioReader {
    file_data: Vec<u8>,
    demuxer: Mp4FileDemuxer,
    source_id: SourceId,
    stats: Mp4AudioReaderStats,
}

impl Mp4AudioReader {
    pub fn new<P: AsRef<Path>>(
        source_id: SourceId,
        path: P,
        stats: Mp4AudioReaderStats,
    ) -> orfail::Result<Self> {
        let file_data = std::fs::read(&path)
            .or_fail_with(|e| format!("Cannot open file {}: {e}", path.as_ref().display()))?;

        let mut demuxer = Mp4FileDemuxer::new();
        let input = Input {
            position: 0,
            data: &file_data,
        };
        demuxer.handle_input(input).or_fail()?;

        Ok(Self {
            file_data,
            demuxer,
            source_id,
            stats,
        })
    }

    pub fn stats(&self) -> &Mp4AudioReaderStats {
        &self.stats
    }
}

impl Iterator for Mp4AudioReader {
    type Item = orfail::Result<AudioData>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let sample = match self.demuxer.next_sample() {
                Ok(Some(sample)) => sample,
                Ok(None) => return None,
                Err(e) => return Some(Err(orfail::Failure::new(e.to_string()).into())),
            };

            // 音声トラックのみを処理
            if sample.track.kind != TrackKind::Audio {
                continue;
            }

            let format = match sample.sample_entry {
                shiguredo_mp4::boxes::SampleEntry::Opus(_) => AudioFormat::Opus,
                entry => {
                    return Some(Err(orfail::Failure::new(format!(
                        "unsupported sample entry: {entry:?}"
                    ))));
                }
            };

            let metadata = match sample.sample_entry {
                shiguredo_mp4::boxes::SampleEntry::Opus(b) => &b.audio,
                _ => unreachable!(),
            };

            let data_offset = sample.data_offset as usize;
            let data_size = sample.data_size;
            let data = self.file_data[data_offset..data_offset + data_size].to_vec();

            return Some(Ok(AudioData {
                source_id: Some(self.source_id.clone()),
                data,
                format,
                sample_entry: Some(sample.sample_entry.clone()),
                stereo: metadata.channelcount != 1,
                sample_rate: metadata.samplerate.integer,
                timestamp: sample.timestamp(),
                duration: sample.duration(),
            }));
        }
    }
}
