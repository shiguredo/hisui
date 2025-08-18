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
pub struct AudioReader {
    output_stream_id: MediaStreamId,
    inner: AudioReaderInner,
}

impl AudioReader {
    pub fn new_mp4(output_stream_id: MediaStreamId, reader: Mp4AudioReader) -> Self {
        Self {
            output_stream_id,
            inner: AudioReaderInner::Mp4(reader),
        }
    }

    pub fn new_webm(output_stream_id: MediaStreamId, reader: WebmAudioReader) -> Self {
        Self {
            output_stream_id,
            inner: AudioReaderInner::Webm(reader),
        }
    }
}

impl Iterator for AudioReader {
    type Item = orfail::Result<AudioData>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            AudioReaderInner::Mp4(r) => r.next(),
            AudioReaderInner::Webm(r) => r.next(),
        }
    }
}

impl MediaProcessor for AudioReader {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: Vec::new(),
            output_stream_ids: vec![self.output_stream_id],
            stats: self.inner.stats(),
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
                self.output_stream_id,
                data,
            )),
        }
    }
}

#[derive(Debug)]
enum AudioReaderInner {
    Mp4(Mp4AudioReader),
    Webm(WebmAudioReader),
}

impl AudioReaderInner {
    fn stats(&self) -> ProcessorStats {
        match self {
            Self::Mp4(r) => r.stats(),
            Self::Webm(r) => r.stats(),
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

    pub fn output_stream_id(&self) -> MediaStreamId {
        match self {
            VideoReader::Mp4(r) => r.output_stream_id(),
            VideoReader::Webm(r) => r.output_stream_id(),
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

impl MediaProcessor for VideoReader {
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
            Some(Ok(frame)) => Ok(MediaProcessorOutput::video_frame(
                self.output_stream_id(),
                frame,
            )),
        }
    }
}
