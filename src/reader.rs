use crate::{
    audio::AudioData,
    media::MediaStreamId,
    processor::{MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec},
    reader_mp4::{Mp4AudioReader, Mp4VideoReader},
    reader_webm::{WebmAudioReader, WebmVideoReader},
    stats::ProcessorStats,
    video::VideoFrame,
};

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

    fn process_input(&mut self, _input: MediaProcessorInput) -> orfail::Result<()> {
        Err(orfail::Failure::new(
            "BUG: reader does not require any input streams",
        ))
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
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
            Self::Mp4(r) => ProcessorStats::Mp4AudioReader(r.stats().clone()),
            Self::Webm(r) => ProcessorStats::WebmAudioReader(r.stats().clone()),
        }
    }
}

#[derive(Debug)]
pub struct VideoReader {
    output_stream_id: MediaStreamId,
    inner: VideoReaderInner,
}

impl VideoReader {
    pub fn new_mp4(output_stream_id: MediaStreamId, reader: Mp4VideoReader) -> Self {
        Self {
            output_stream_id,
            inner: VideoReaderInner::Mp4(reader),
        }
    }

    pub fn new_webm(output_stream_id: MediaStreamId, reader: WebmVideoReader) -> Self {
        Self {
            output_stream_id,
            inner: VideoReaderInner::Webm(reader),
        }
    }
}

impl Iterator for VideoReader {
    type Item = orfail::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            VideoReaderInner::Mp4(r) => r.next(),
            VideoReaderInner::Webm(r) => r.next(),
        }
    }
}

impl MediaProcessor for VideoReader {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: Vec::new(),
            output_stream_ids: vec![self.output_stream_id],
            stats: self.inner.stats(),
        }
    }

    fn process_input(&mut self, _input: MediaProcessorInput) -> orfail::Result<()> {
        Err(orfail::Failure::new(
            "BUG: reader does not require any input streams",
        ))
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        match self.next() {
            None => Ok(MediaProcessorOutput::Finished),
            Some(Err(e)) => Err(e),
            Some(Ok(frame)) => Ok(MediaProcessorOutput::video_frame(
                self.output_stream_id,
                frame,
            )),
        }
    }
}

#[derive(Debug)]
enum VideoReaderInner {
    Mp4(Mp4VideoReader),
    Webm(WebmVideoReader),
}

impl VideoReaderInner {
    fn stats(&self) -> ProcessorStats {
        match self {
            Self::Mp4(r) => ProcessorStats::Mp4VideoReader(r.stats().clone()),
            Self::Webm(r) => ProcessorStats::WebmVideoReader(r.stats().clone()),
        }
    }
}
