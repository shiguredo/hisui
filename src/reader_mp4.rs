use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use orfail::OrFail;
use shiguredo_mp4::{
    TrackKind,
    demux::{Input, Mp4FileDemuxer},
};

use crate::{
    audio::{AudioData, AudioFormat},
    metadata::SourceId,
    video::{VideoFormat, VideoFrame},
};

#[derive(Debug)]
pub struct Mp4VideoReader {
    file_data: Vec<u8>,
    demuxer: Mp4FileDemuxer,
    source_id: SourceId,
}

impl Mp4VideoReader {
    pub fn new<P: AsRef<Path>>(source_id: SourceId, path: P) -> orfail::Result<Self> {
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
        })
    }
}

impl Iterator for Mp4VideoReader {
    type Item = orfail::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let sample = match self.demuxer.next_sample() {
                Ok(Some(sample)) => sample,
                Ok(None) => return None,
                Err(e) => return Some(Err(orfail::Failure::new(e.to_string()).into())),
            };

            // ビデオトラックのみを処理
            if sample.track.kind != TrackKind::Video {
                continue;
            }

            let format = match sample.sample_entry {
                shiguredo_mp4::boxes::SampleEntry::Avc1(_) => VideoFormat::H264,
                shiguredo_mp4::boxes::SampleEntry::Hev1(_) => VideoFormat::H265,
                shiguredo_mp4::boxes::SampleEntry::Vp08(_) => VideoFormat::Vp8,
                shiguredo_mp4::boxes::SampleEntry::Vp09(_) => VideoFormat::Vp9,
                shiguredo_mp4::boxes::SampleEntry::Av01(_) => VideoFormat::Av1,
                entry => {
                    return Some(Err(orfail::Failure::new(format!(
                        "unsupported sample entry: {entry:?}"
                    ))));
                }
            };

            let metadata = match sample.sample_entry {
                shiguredo_mp4::boxes::SampleEntry::Avc1(b) => &b.visual,
                shiguredo_mp4::boxes::SampleEntry::Hev1(b) => &b.visual,
                shiguredo_mp4::boxes::SampleEntry::Vp08(b) => &b.visual,
                shiguredo_mp4::boxes::SampleEntry::Vp09(b) => &b.visual,
                shiguredo_mp4::boxes::SampleEntry::Av01(b) => &b.visual,
                _ => unreachable!(),
            };

            let data_offset = sample.data_offset as usize;
            let data_size = sample.data_size;
            let data = self.file_data[data_offset..data_offset + data_size].to_vec();

            return Some(Ok(VideoFrame {
                source_id: Some(self.source_id.clone()),
                sample_entry: Some(sample.sample_entry.clone()),
                data,
                format,
                keyframe: sample.keyframe,
                width: metadata.width as usize,
                height: metadata.height as usize,
                timestamp: sample.timestamp(),
                duration: sample.duration(),
            }));
        }
    }
}

#[derive(Debug)]
pub struct Mp4AudioReader {
    file_data: Vec<u8>,
    demuxer: Mp4FileDemuxer,
    source_id: SourceId,
}

impl Mp4AudioReader {
    pub fn new<P: AsRef<Path>>(source_id: SourceId, path: P) -> orfail::Result<Self> {
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
        })
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
