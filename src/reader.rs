use std::path::PathBuf;
use std::time::Duration;

use orfail::OrFail;

use crate::{
    audio::AudioData,
    layout::AggregatedSourceInfo,
    media::MediaStreamId,
    metadata::{ContainerFormat, SourceId},
    processor::{
        MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
        MediaProcessorWorkloadHint,
    },
    reader_mp4::{Mp4AudioReader, Mp4VideoReader},
    reader_webm::{WebmAudioReader, WebmVideoReader},
    stats::{
        Mp4AudioReaderStats, Mp4VideoReaderStats, ProcessorStats, SharedOption,
        WebmAudioReaderStats, WebmVideoReaderStats,
    },
    types::CodecName,
    video::VideoFrame,
};

#[derive(Debug)]
pub struct AudioReader {
    output_stream_id: MediaStreamId,
    source_id: SourceId,
    timestamp_offset: Duration,
    next_timestamp_offset: Duration,
    remaining_input_files: Vec<PathBuf>,
    inner: AudioReaderInner,
}

impl AudioReader {
    pub fn from_source_info(
        output_stream_id: MediaStreamId,
        source_info: &AggregatedSourceInfo,
    ) -> orfail::Result<Self> {
        Self::new(
            output_stream_id,
            source_info.id.clone(),
            source_info.format,
            source_info.start_timestamp,
            source_info.timestamp_sorted_media_paths(),
        )
    }

    pub fn new(
        output_stream_id: MediaStreamId,
        source_id: SourceId,
        format: ContainerFormat,
        timestamp_offset: Duration,
        input_files: Vec<PathBuf>,
    ) -> orfail::Result<Self> {
        let mut remaining_input_files = input_files.clone();
        remaining_input_files.reverse();
        let first_input_file = remaining_input_files.pop().or_fail()?;
        let inner = match format {
            ContainerFormat::Mp4 => {
                let stats = Mp4AudioReaderStats {
                    input_files,
                    codec: Some(CodecName::Opus),
                    start_time: timestamp_offset,
                    current_input_file: SharedOption::new(Some(first_input_file.clone())),
                    ..Default::default()
                };
                AudioReaderInner::Mp4(
                    Mp4AudioReader::new(source_id.clone(), first_input_file, stats).or_fail()?,
                )
            }
            ContainerFormat::Webm => {
                let stats = WebmAudioReaderStats {
                    input_files,
                    codec: Some(CodecName::Opus),
                    start_time: timestamp_offset,
                    current_input_file: SharedOption::new(Some(first_input_file.clone())),
                    ..Default::default()
                };
                AudioReaderInner::Webm(
                    WebmAudioReader::new(source_id.clone(), first_input_file, stats).or_fail()?,
                )
            }
        };
        Ok(Self {
            output_stream_id,
            source_id,
            timestamp_offset,
            next_timestamp_offset: timestamp_offset,
            remaining_input_files,
            inner,
        })
    }

    fn start_next_input_file(&mut self) -> orfail::Result<bool> {
        match &mut self.inner {
            AudioReaderInner::Mp4(inner) => start_next_input_file(
                &mut self.remaining_input_files,
                self.source_id.clone(),
                inner.stats().current_input_file.clone(),
                inner.stats().clone(),
                Mp4AudioReader::new,
            )
            .map(|reader| reader.map(|reader| *inner = reader).is_some())
            .or_fail(),
            AudioReaderInner::Webm(inner) => start_next_input_file(
                &mut self.remaining_input_files,
                self.source_id.clone(),
                inner.stats().current_input_file.clone(),
                inner.stats().clone(),
                WebmAudioReader::new,
            )
            .map(|reader| reader.map(|reader| *inner = reader).is_some())
            .or_fail(),
        }
    }
}

impl MediaProcessor for AudioReader {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: Vec::new(),
            output_stream_ids: vec![self.output_stream_id],
            stats: self.inner.stats(),
            workload_hint: MediaProcessorWorkloadHint::READER,
        }
    }

    fn process_input(&mut self, _input: MediaProcessorInput) -> orfail::Result<()> {
        Err(orfail::Failure::new(
            "BUG: reader does not require any input streams",
        ))
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        loop {
            match self.inner.next() {
                None => {
                    if !self.start_next_input_file().or_fail()? {
                        return Ok(MediaProcessorOutput::Finished);
                    }
                    self.timestamp_offset = self.next_timestamp_offset;
                    self.inner.set_timestamp_offset(self.timestamp_offset);
                }
                Some(Err(e)) => return Err(e),
                Some(Ok(mut data)) => {
                    data.timestamp += self.timestamp_offset;
                    self.next_timestamp_offset = data.timestamp + data.duration;
                    return Ok(MediaProcessorOutput::audio_data(
                        self.output_stream_id,
                        data,
                    ));
                }
            }
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

    fn set_timestamp_offset(&self, offset: Duration) {
        match self {
            Self::Mp4(r) => r.stats().track_duration_offset.set(offset),
            Self::Webm(r) => r.stats().track_duration_offset.set(offset),
        }
    }
}

impl Iterator for AudioReaderInner {
    type Item = orfail::Result<AudioData>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Mp4(r) => r.next(),
            Self::Webm(r) => r.next(),
        }
    }
}

#[derive(Debug)]
pub struct VideoReader {
    output_stream_id: MediaStreamId,
    source_id: SourceId,
    timestamp_offset: Duration,
    next_timestamp_offset: Duration,
    remaining_input_files: Vec<PathBuf>,
    inner: VideoReaderInner,
}

impl VideoReader {
    pub fn from_source_info(
        output_stream_id: MediaStreamId,
        source_info: &AggregatedSourceInfo,
    ) -> orfail::Result<Self> {
        Self::new(
            output_stream_id,
            source_info.id.clone(),
            source_info.format,
            source_info.start_timestamp,
            source_info.timestamp_sorted_media_paths(),
        )
    }

    pub fn new(
        output_stream_id: MediaStreamId,
        source_id: SourceId,
        format: ContainerFormat,
        timestamp_offset: Duration,
        input_files: Vec<PathBuf>,
    ) -> orfail::Result<Self> {
        let mut remaining_input_files = input_files.clone();
        remaining_input_files.reverse();
        let first_input_file = remaining_input_files.pop().or_fail()?;
        let inner = match format {
            ContainerFormat::Mp4 => {
                let stats = Mp4VideoReaderStats {
                    input_files,
                    current_input_file: SharedOption::new(Some(first_input_file.clone())),
                    start_time: timestamp_offset,
                    ..Default::default()
                };
                VideoReaderInner::Mp4(Box::new(
                    Mp4VideoReader::new(source_id.clone(), first_input_file, stats).or_fail()?,
                ))
            }
            ContainerFormat::Webm => {
                let stats = WebmVideoReaderStats {
                    input_files,
                    current_input_file: SharedOption::new(Some(first_input_file.clone())),
                    start_time: timestamp_offset,
                    ..Default::default()
                };
                VideoReaderInner::Webm(Box::new(
                    WebmVideoReader::new(source_id.clone(), first_input_file, stats).or_fail()?,
                ))
            }
        };
        Ok(Self {
            output_stream_id,
            source_id,
            timestamp_offset,
            next_timestamp_offset: timestamp_offset,
            remaining_input_files,
            inner,
        })
    }

    fn start_next_input_file(&mut self) -> orfail::Result<bool> {
        match &mut self.inner {
            VideoReaderInner::Mp4(inner) => start_next_input_file(
                &mut self.remaining_input_files,
                self.source_id.clone(),
                inner.stats().current_input_file.clone(),
                inner.stats().clone(),
                Mp4VideoReader::new,
            )
            .map(|reader| reader.map(|reader| *inner = Box::new(reader)).is_some())
            .or_fail(),
            VideoReaderInner::Webm(inner) => start_next_input_file(
                &mut self.remaining_input_files,
                self.source_id.clone(),
                inner.stats().current_input_file.clone(),
                inner.stats().clone(),
                WebmVideoReader::new,
            )
            .map(|reader| reader.map(|reader| *inner = Box::new(reader)).is_some())
            .or_fail(),
        }
    }
}

impl MediaProcessor for VideoReader {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: Vec::new(),
            output_stream_ids: vec![self.output_stream_id],
            stats: self.inner.stats(),
            workload_hint: MediaProcessorWorkloadHint::READER,
        }
    }

    fn process_input(&mut self, _input: MediaProcessorInput) -> orfail::Result<()> {
        Err(orfail::Failure::new(
            "BUG: reader does not require any input streams",
        ))
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        loop {
            match self.inner.next() {
                None => {
                    if !self.start_next_input_file().or_fail()? {
                        return Ok(MediaProcessorOutput::Finished);
                    }
                    self.timestamp_offset = self.next_timestamp_offset;
                    self.inner.set_timestamp_offset(self.timestamp_offset);
                }
                Some(Err(e)) => return Err(e),
                Some(Ok(mut frame)) => {
                    frame.timestamp += self.timestamp_offset;
                    self.next_timestamp_offset = frame.timestamp + frame.duration;
                    return Ok(MediaProcessorOutput::video_frame(
                        self.output_stream_id,
                        frame,
                    ));
                }
            }
        }
    }
}

// Box は clippy::large_enum_variant 対策
#[derive(Debug)]
enum VideoReaderInner {
    Mp4(Box<Mp4VideoReader>),
    Webm(Box<WebmVideoReader>),
}

impl VideoReaderInner {
    fn stats(&self) -> ProcessorStats {
        match self {
            Self::Mp4(r) => ProcessorStats::Mp4VideoReader(r.stats().clone()),
            Self::Webm(r) => ProcessorStats::WebmVideoReader(r.stats().clone()),
        }
    }

    fn set_timestamp_offset(&self, offset: Duration) {
        match self {
            Self::Mp4(r) => r.stats().track_duration_offset.set(offset),
            Self::Webm(r) => r.stats().track_duration_offset.set(offset),
        }
    }
}

impl Iterator for VideoReaderInner {
    type Item = orfail::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Mp4(r) => r.next(),
            Self::Webm(r) => r.next(),
        }
    }
}

fn start_next_input_file<F, S, R>(
    remaining_input_files: &mut Vec<PathBuf>,
    source_id: SourceId,
    current_input_file: SharedOption<PathBuf>,
    stats: S,
    f: F,
) -> orfail::Result<Option<R>>
where
    F: FnOnce(SourceId, PathBuf, S) -> orfail::Result<R>,
{
    if let Some(next_input_file) = remaining_input_files.pop() {
        current_input_file.set(next_input_file.clone());
        let reader = f(source_id, next_input_file, stats).or_fail()?;
        Ok(Some(reader))
    } else {
        current_input_file.clear();
        Ok(None)
    }
}
