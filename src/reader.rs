use crate::{
    audio::AudioData,
    media::MediaStreamId,
    processor::{MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec},
    reader_mp4::{Mp4AudioReader, Mp4VideoReader},
    reader_webm::{WebmAudioReader, WebmVideoReader},
    stats::ProcessorStats,
    video::VideoFrame,
};

// TODO: 最終的にはこの enum はなくして直接 Processor に追加する
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

    pub fn output_stream_id(&self) -> MediaStreamId {
        match self {
            AudioReader::Mp4(r) => r.output_stream_id(),
            AudioReader::Webm(r) => r.output_stream_id(),
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

impl MediaProcessor for AudioReader {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: Vec::new(),
            output_stream_ids: vec![self.output_stream_id()],
            stats: self.stats(),
        }
    }

    fn process(&mut self, _input: MediaProcessorInput) -> orfail::Result<()> {
        Err(orfail::Failure::new(
            "BUG: reader does not require any input streams",
        ))
    }

    fn poll_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        match self.next() {
            None => Ok(MediaProcessorOutput::Finished),
            Some(Err(e)) => Err(e),
            Some(Ok(data)) => Ok(MediaProcessorOutput::audio_data(
                self.output_stream_id(),
                data,
            )),
        }
    }
}

// TODO: 最終的にはこの enum はなくして直接 Processor に追加する
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
