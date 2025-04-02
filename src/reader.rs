use crate::{
    audio::AudioData,
    reader_mp4::{Mp4AudioReader, Mp4VideoReader},
    reader_webm::{WebmAudioReader, WebmVideoReader},
    stats::ReaderStats,
    video::VideoFrame,
};

#[derive(Debug)]
pub enum AudioReader {
    Mp4(Mp4AudioReader),
    Webm(WebmAudioReader),
}

impl AudioReader {
    pub fn stats(&self) -> ReaderStats {
        match self {
            AudioReader::Mp4(r) => ReaderStats::Mp4Audio(r.stats().clone()),
            AudioReader::Webm(r) => ReaderStats::WebmAudio(r.stats().clone()),
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
pub enum VideoReader {
    Mp4(Mp4VideoReader),
    Webm(WebmVideoReader),
}

impl VideoReader {
    pub fn stats(&self) -> ReaderStats {
        match self {
            VideoReader::Mp4(r) => ReaderStats::Mp4Video(r.stats().clone()),
            VideoReader::Webm(r) => ReaderStats::WebmVideo(r.stats().clone()),
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
