use std::path::PathBuf;
use std::time::Duration;

use orfail::OrFail;

use crate::{
    audio::AudioData,
    media::MediaStreamId,
    metadata::{ContainerFormat, SourceId},
    processor::{MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec},
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
            AudioReaderInner::Mp4(inner) => {
                if let Some(next_input_file) = self.remaining_input_files.pop() {
                    inner
                        .stats()
                        .current_input_file
                        .set(next_input_file.clone());
                    *inner = Mp4AudioReader::new(
                        self.source_id.clone(),
                        next_input_file,
                        inner.stats().clone(),
                    )
                    .or_fail()?;
                    Ok(true)
                } else {
                    inner.stats().current_input_file.clear();
                    Ok(false)
                }
            }
            AudioReaderInner::Webm(inner) => {
                if let Some(next_input_file) = self.remaining_input_files.pop() {
                    inner
                        .stats()
                        .current_input_file
                        .set(next_input_file.clone());
                    *inner = WebmAudioReader::new(
                        self.source_id.clone(),
                        next_input_file,
                        inner.stats().clone(),
                    )
                    .or_fail()?;
                    Ok(true)
                } else {
                    inner.stats().current_input_file.clear();
                    Ok(false)
                }
            }
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
        loop {
            match self.inner.next() {
                None => {
                    if !self.start_next_input_file().or_fail()? {
                        return Ok(MediaProcessorOutput::Finished);
                    }
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
                VideoReaderInner::Mp4(
                    Mp4VideoReader::new(source_id.clone(), first_input_file, stats).or_fail()?,
                )
            }
            ContainerFormat::Webm => {
                let stats = WebmVideoReaderStats {
                    input_files,
                    current_input_file: SharedOption::new(Some(first_input_file.clone())),
                    start_time: timestamp_offset,
                    ..Default::default()
                };
                VideoReaderInner::Webm(
                    WebmVideoReader::new(source_id.clone(), first_input_file, stats).or_fail()?,
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
            VideoReaderInner::Mp4(inner) => {
                if let Some(next_input_file) = self.remaining_input_files.pop() {
                    inner
                        .stats()
                        .current_input_file
                        .set(next_input_file.clone());
                    *inner = Mp4VideoReader::new(
                        self.source_id.clone(),
                        next_input_file,
                        inner.stats().clone(),
                    )
                    .or_fail()?;
                    Ok(true)
                } else {
                    inner.stats().current_input_file.clear();
                    Ok(false)
                }
            }
            VideoReaderInner::Webm(inner) => {
                if let Some(next_input_file) = self.remaining_input_files.pop() {
                    inner
                        .stats()
                        .current_input_file
                        .set(next_input_file.clone());
                    *inner = WebmVideoReader::new(
                        self.source_id.clone(),
                        next_input_file,
                        inner.stats().clone(),
                    )
                    .or_fail()?;
                    Ok(true)
                } else {
                    inner.stats().current_input_file.clear();
                    Ok(false)
                }
            }
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
        loop {
            match self.inner.next() {
                None => {
                    if !self.start_next_input_file().or_fail()? {
                        return Ok(MediaProcessorOutput::Finished);
                    }
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

impl Iterator for VideoReaderInner {
    type Item = orfail::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Mp4(r) => r.next(),
            Self::Webm(r) => r.next(),
        }
    }
}
