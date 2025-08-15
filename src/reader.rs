use crate::{
    audio::AudioData,
    reader_mp4::{Mp4AudioReader, Mp4VideoReader},
    reader_webm::{WebmAudioReader, WebmVideoReader},
    stats::ProcessorStats,
    video::VideoFrame,
};

#[derive(Debug)]
pub enum AudioReader {
    Mp4(Mp4AudioReader),
    Webm(WebmAudioReader),
}

impl AudioReader {
    pub fn stats(&self) -> ProcessorStats {
        match self {
            AudioReader::Mp4(r) => r.stats(),
            AudioReader::Webm(r) => r.stats(),
        }
    }
}

impl Iterator for AudioReader {
    type Item = orfail::Result<AudioData>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            AudioReader::Mp4(r) => r.next(),
            AudioReader::Webm(r) => r.next(),
        }
    }
}

#[derive(Debug)]
#[expect(clippy::large_enum_variant)]
pub enum VideoReader {
    Mp4(Mp4VideoReader),
    Webm(WebmVideoReader),
}

impl VideoReader {
    pub fn stats(&self) -> ProcessorStats {
        match self {
            VideoReader::Mp4(r) => r.stats(),
            VideoReader::Webm(r) => r.stats(),
        }
    }
}

impl Iterator for VideoReader {
    type Item = orfail::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            VideoReader::Mp4(r) => r.next(),
            VideoReader::Webm(r) => r.next(),
        }
    }
}
