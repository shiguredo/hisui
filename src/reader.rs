use std::path::PathBuf;
use std::time::Duration;

use orfail::OrFail;

use crate::{
    audio::AudioData,
    layout::AggregatedSourceInfo,
    legacy_processor_stats::{
        Mp4AudioReaderStats, Mp4VideoReaderStats, ProcessorStats, SharedOption,
        WebmAudioReaderStats, WebmVideoReaderStats,
    },
    media::{MediaSample, MediaStreamId},
    metadata::{ContainerFormat, SourceId},
    processor::{
        MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
        MediaProcessorWorkloadHint,
    },
    reader_mp4::{Mp4AudioReader, Mp4VideoReader},
    reader_webm::{WebmAudioReader, WebmVideoReader},
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
    compose_stats: crate::stats::Stats,
}

impl AudioReader {
    pub async fn run(mut self, handle: crate::ProcessorHandle) -> orfail::Result<()> {
        let track_id = crate::TrackId::new(handle.processor_id().get());
        let mut track_handle = handle.publish_track(track_id).await.or_fail()?;
        handle.notify_ready();
        handle
            .wait_subscribers_ready()
            .await
            .map_err(|e| orfail::Failure::new(e.to_string()))?;

        let mut ack = track_handle.send_syn();
        let mut noacked_sent = 0;
        loop {
            // 100 はとりあえずの暫定値。
            // おそらくこの値は適当に大きい値ならなんでも構わないが、実際に使ってみて問題があれば都度調整する。
            if noacked_sent > 100 {
                ack.await;
                ack = track_handle.send_syn();
                noacked_sent = 0;
            }

            match self.inner.next() {
                None => {
                    if !self.start_next_input_file().or_fail()? {
                        // 全てのファイルの末尾に達した
                        break;
                    }
                    self.timestamp_offset = self.next_timestamp_offset;
                    self.inner.set_timestamp_offset(self.timestamp_offset);
                }
                Some(Err(e)) => return Err(e),
                Some(Ok(mut data)) => {
                    data.timestamp += self.timestamp_offset;
                    self.next_timestamp_offset = data.timestamp + data.duration;

                    if !track_handle.send_media(MediaSample::new_audio(data)) {
                        // パイプライン処理が中断された
                        break;
                    }
                    noacked_sent += 1;
                }
            }
        }

        track_handle.send_eos();

        Ok(())
    }

    pub fn from_source_info(
        output_stream_id: MediaStreamId,
        source_info: &AggregatedSourceInfo,
    ) -> orfail::Result<Self> {
        Self::new_with_stats(
            output_stream_id,
            source_info.id.clone(),
            source_info.format,
            source_info.start_timestamp,
            source_info.timestamp_sorted_media_paths(),
            crate::stats::Stats::new(),
        )
    }

    pub fn new(
        output_stream_id: MediaStreamId,
        source_id: SourceId,
        format: ContainerFormat,
        timestamp_offset: Duration,
        input_files: Vec<PathBuf>,
    ) -> orfail::Result<Self> {
        Self::new_with_stats(
            output_stream_id,
            source_id,
            format,
            timestamp_offset,
            input_files,
            crate::stats::Stats::new(),
        )
    }

    pub fn new_with_stats(
        output_stream_id: MediaStreamId,
        source_id: SourceId,
        format: ContainerFormat,
        timestamp_offset: Duration,
        input_files: Vec<PathBuf>,
        mut compose_stats: crate::stats::Stats,
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
                AudioReaderInner::Mp4(Box::new(
                    Mp4AudioReader::new(source_id.clone(), first_input_file, stats).or_fail()?,
                ))
            }
            ContainerFormat::Webm => {
                let stats = WebmAudioReaderStats {
                    input_files,
                    codec: Some(CodecName::Opus),
                    start_time: timestamp_offset,
                    current_input_file: SharedOption::new(Some(first_input_file.clone())),
                    ..Default::default()
                };
                AudioReaderInner::Webm(Box::new(
                    WebmAudioReader::new(source_id.clone(), first_input_file, stats).or_fail()?,
                ))
            }
        };
        compose_stats
            .gauge_f64("start_time_seconds")
            .set(timestamp_offset.as_secs_f64());
        compose_stats.flag("error").set(false);
        Ok(Self {
            output_stream_id,
            source_id,
            timestamp_offset,
            next_timestamp_offset: timestamp_offset,
            remaining_input_files,
            inner,
            compose_stats,
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
            .map(|reader| reader.map(|reader| **inner = reader).is_some())
            .or_fail(),
            AudioReaderInner::Webm(inner) => start_next_input_file(
                &mut self.remaining_input_files,
                self.source_id.clone(),
                inner.stats().current_input_file.clone(),
                inner.stats().clone(),
                WebmAudioReader::new,
            )
            .map(|reader| reader.map(|reader| **inner = reader).is_some())
            .or_fail(),
        }
    }

    fn update_compose_stats_from_inner(&mut self) {
        let mut stats = self.compose_stats.clone();
        match self.inner.stats() {
            ProcessorStats::Mp4AudioReader(reader) => {
                stats
                    .gauge("total_sample_count")
                    .set(reader.total_sample_count.get() as i64);
                stats.gauge_f64("total_track_seconds").set(
                    (reader.track_duration_offset.get() + reader.total_track_duration.get())
                        .as_secs_f64(),
                );
                if let Some(codec) = reader.codec {
                    stats.string("codec").set(codec.as_str());
                }
                if let Some(path) = reader.current_input_file.get() {
                    stats
                        .string("current_input_file")
                        .set(path.display().to_string());
                }
            }
            ProcessorStats::WebmAudioReader(reader) => {
                stats
                    .gauge("total_cluster_count")
                    .set(reader.total_cluster_count.get() as i64);
                stats
                    .gauge("total_simple_block_count")
                    .set(reader.total_simple_block_count.get() as i64);
                stats.gauge_f64("total_track_seconds").set(
                    (reader.track_duration_offset.get() + reader.total_track_duration.get())
                        .as_secs_f64(),
                );
                if let Some(codec) = reader.codec {
                    stats.string("codec").set(codec.as_str());
                }
                if let Some(path) = reader.current_input_file.get() {
                    stats
                        .string("current_input_file")
                        .set(path.display().to_string());
                }
            }
            _ => {}
        }
    }
}

impl MediaProcessor for AudioReader {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: Vec::new(),
            output_stream_ids: vec![self.output_stream_id],
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
                    self.update_compose_stats_from_inner();
                    return Ok(MediaProcessorOutput::audio_data(
                        self.output_stream_id,
                        data,
                    ));
                }
            }
        }
    }

    fn set_error(&self) {
        self.inner.stats().set_error();
        let mut stats = self.compose_stats.clone();
        stats.flag("error").set(true);
    }
}

#[derive(Debug)]
enum AudioReaderInner {
    Mp4(Box<Mp4AudioReader>),
    Webm(Box<WebmAudioReader>),
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
    compose_stats: crate::stats::Stats,
}

impl VideoReader {
    pub async fn run(mut self, handle: crate::ProcessorHandle) -> orfail::Result<()> {
        let track_id = crate::TrackId::new(handle.processor_id().get());
        let mut track_handle = handle.publish_track(track_id).await.or_fail()?;
        handle.notify_ready();
        handle
            .wait_subscribers_ready()
            .await
            .map_err(|e| orfail::Failure::new(e.to_string()))?;

        let mut ack = track_handle.send_syn();
        let mut noacked_sent = 0;
        loop {
            // 100 はとりあえずの暫定値。
            // おそらくこの値は適当に大きい値ならなんでも構わないが、実際に使ってみて問題があれば都度調整する。
            if noacked_sent > 100 {
                ack.await;
                ack = track_handle.send_syn();
                noacked_sent = 0;
            }

            match self.inner.next() {
                None => {
                    if !self.start_next_input_file().or_fail()? {
                        // 全てのファイルの末尾に達した
                        break;
                    }
                    self.timestamp_offset = self.next_timestamp_offset;
                    self.inner.set_timestamp_offset(self.timestamp_offset);
                }
                Some(Err(e)) => return Err(e),
                Some(Ok(mut frame)) => {
                    frame.timestamp += self.timestamp_offset;
                    self.next_timestamp_offset = frame.timestamp + frame.duration;

                    if !track_handle.send_media(MediaSample::new_video(frame)) {
                        // パイプライン処理が中断された
                        break;
                    }
                    noacked_sent += 1;
                }
            }
        }
        track_handle.send_eos();

        Ok(())
    }

    pub fn from_source_info(
        output_stream_id: MediaStreamId,
        source_info: &AggregatedSourceInfo,
    ) -> orfail::Result<Self> {
        Self::new_with_stats(
            output_stream_id,
            source_info.id.clone(),
            source_info.format,
            source_info.start_timestamp,
            source_info.timestamp_sorted_media_paths(),
            crate::stats::Stats::new(),
        )
    }

    pub fn new(
        output_stream_id: MediaStreamId,
        source_id: SourceId,
        format: ContainerFormat,
        timestamp_offset: Duration,
        input_files: Vec<PathBuf>,
    ) -> orfail::Result<Self> {
        Self::new_with_stats(
            output_stream_id,
            source_id,
            format,
            timestamp_offset,
            input_files,
            crate::stats::Stats::new(),
        )
    }

    pub fn new_with_stats(
        output_stream_id: MediaStreamId,
        source_id: SourceId,
        format: ContainerFormat,
        timestamp_offset: Duration,
        input_files: Vec<PathBuf>,
        mut compose_stats: crate::stats::Stats,
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
        compose_stats
            .gauge_f64("start_time_seconds")
            .set(timestamp_offset.as_secs_f64());
        compose_stats.flag("error").set(false);
        Ok(Self {
            output_stream_id,
            source_id,
            timestamp_offset,
            next_timestamp_offset: timestamp_offset,
            remaining_input_files,
            inner,
            compose_stats,
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
            .map(|reader| reader.map(|reader| **inner = reader).is_some())
            .or_fail(),
            VideoReaderInner::Webm(inner) => start_next_input_file(
                &mut self.remaining_input_files,
                self.source_id.clone(),
                inner.stats().current_input_file.clone(),
                inner.stats().clone(),
                WebmVideoReader::new,
            )
            .map(|reader| reader.map(|reader| **inner = reader).is_some())
            .or_fail(),
        }
    }

    fn update_compose_stats_from_inner(&mut self) {
        let mut stats = self.compose_stats.clone();
        match self.inner.stats() {
            ProcessorStats::Mp4VideoReader(reader) => {
                stats
                    .gauge("total_sample_count")
                    .set(reader.total_sample_count.get() as i64);
                stats.gauge_f64("total_track_seconds").set(
                    (reader.track_duration_offset.get() + reader.total_track_duration.get())
                        .as_secs_f64(),
                );
                if let Some(codec) = reader.codec.get() {
                    stats.string("codec").set(codec.as_str());
                }
                if let Some(path) = reader.current_input_file.get() {
                    stats
                        .string("current_input_file")
                        .set(path.display().to_string());
                }
            }
            ProcessorStats::WebmVideoReader(reader) => {
                stats
                    .gauge("total_cluster_count")
                    .set(reader.total_cluster_count.get() as i64);
                stats
                    .gauge("total_simple_block_count")
                    .set(reader.total_simple_block_count.get() as i64);
                stats.gauge_f64("total_track_seconds").set(
                    (reader.track_duration_offset.get() + reader.total_track_duration.get())
                        .as_secs_f64(),
                );
                if let Some(codec) = reader.codec.get() {
                    stats.string("codec").set(codec.as_str());
                }
                if let Some(path) = reader.current_input_file.get() {
                    stats
                        .string("current_input_file")
                        .set(path.display().to_string());
                }
            }
            _ => {}
        }
    }
}

impl MediaProcessor for VideoReader {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: Vec::new(),
            output_stream_ids: vec![self.output_stream_id],
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
                    self.update_compose_stats_from_inner();
                    return Ok(MediaProcessorOutput::video_frame(
                        self.output_stream_id,
                        frame,
                    ));
                }
            }
        }
    }

    fn set_error(&self) {
        self.inner.stats().set_error();
        let mut stats = self.compose_stats.clone();
        stats.flag("error").set(true);
    }
}

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
