use std::path::PathBuf;
use std::time::Duration;

use crate::{
    audio::AudioData,
    layout::AggregatedSourceInfo,
    media::MediaSample,
    metadata::{ContainerFormat, SourceId},
    reader_mp4::{Mp4AudioReader, Mp4VideoReader},
    reader_webm::{WebmAudioReader, WebmVideoReader},
    types::CodecName,
    video::VideoFrame,
};

#[derive(Debug)]
pub struct AudioReader {
    source_id: SourceId,
    timestamp_offset: Duration,
    next_timestamp_offset: Duration,
    remaining_input_files: Vec<PathBuf>,
    inner: AudioReaderInner,
    total_sample_count_metric: crate::stats::StatsGauge,
    total_cluster_count_metric: crate::stats::StatsGauge,
    total_simple_block_count_metric: crate::stats::StatsGauge,
    total_track_seconds_metric: crate::stats::StatsGaugeF64,
    codec_metric: crate::stats::StatsString,
    current_input_file_metric: crate::stats::StatsString,
    error_flag: crate::stats::StatsFlag,
}

impl AudioReader {
    pub async fn run(mut self, handle: crate::ProcessorHandle) -> crate::Result<()> {
        let track_id = crate::TrackId::new(handle.processor_id().get());
        let mut track_handle = handle.publish_track(track_id).await?;
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

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
                    if !self.start_next_input_file()? {
                        // 全てのファイルの末尾に達した
                        break;
                    }
                    self.timestamp_offset = self.next_timestamp_offset;
                    self.inner.set_timestamp_offset(self.timestamp_offset);
                }
                Some(Err(e)) => {
                    self.error_flag.set(true);
                    return Err(e);
                }
                Some(Ok(mut data)) => {
                    data.timestamp += self.timestamp_offset;
                    self.next_timestamp_offset = data.timestamp + data.duration;
                    self.update_metrics_from_inner();

                    if !track_handle.send_media(MediaSample::new_audio(data)) {
                        // パイプライン処理が中断された
                        break;
                    }
                    noacked_sent += 1;
                }
            }
        }

        self.update_metrics_from_inner();
        track_handle.send_eos();

        Ok(())
    }

    pub fn from_source_info(
        source_info: &AggregatedSourceInfo,
        stats: crate::stats::Stats,
    ) -> crate::Result<Self> {
        Self::new(
            source_info.id.clone(),
            source_info.format,
            source_info.start_timestamp,
            source_info.timestamp_sorted_media_paths(),
            stats,
        )
    }

    pub fn new(
        source_id: SourceId,
        format: ContainerFormat,
        timestamp_offset: Duration,
        input_files: Vec<PathBuf>,
        mut compose_stats: crate::stats::Stats,
    ) -> crate::Result<Self> {
        let mut remaining_input_files = input_files.clone();
        remaining_input_files.reverse();
        let first_input_file = remaining_input_files
            .pop()
            .ok_or_else(|| crate::Error::new("no input file for audio reader"))?;
        let inner = match format {
            ContainerFormat::Mp4 => {
                let mut reader = Mp4AudioReader::new(source_id.clone(), first_input_file.clone())?;
                reader.codec = Some(CodecName::Opus);
                reader.current_input_file = Some(first_input_file.clone());
                AudioReaderInner::Mp4(Box::new(reader))
            }
            ContainerFormat::Webm => {
                let mut reader = WebmAudioReader::new(source_id.clone(), first_input_file.clone())?;
                reader.codec = Some(CodecName::Opus);
                reader.current_input_file = Some(first_input_file.clone());
                AudioReaderInner::Webm(Box::new(reader))
            }
        };
        compose_stats
            .gauge_f64("start_time_seconds")
            .set(timestamp_offset.as_secs_f64());
        let total_sample_count_metric = compose_stats.gauge("total_sample_count");
        let total_cluster_count_metric = compose_stats.gauge("total_cluster_count");
        let total_simple_block_count_metric = compose_stats.gauge("total_simple_block_count");
        let total_track_seconds_metric = compose_stats.gauge_f64("total_track_seconds");
        let codec_metric = compose_stats.string("codec");
        let current_input_file_metric = compose_stats.string("current_input_file");
        let error_flag = compose_stats.flag("error");
        error_flag.set(false);
        Ok(Self {
            source_id,
            timestamp_offset,
            next_timestamp_offset: timestamp_offset,
            remaining_input_files,
            inner,
            total_sample_count_metric,
            total_cluster_count_metric,
            total_simple_block_count_metric,
            total_track_seconds_metric,
            codec_metric,
            current_input_file_metric,
            error_flag,
        })
    }

    fn start_next_input_file(&mut self) -> crate::Result<bool> {
        match &mut self.inner {
            AudioReaderInner::Mp4(inner) => {
                if let Some(next_input_file) = self.remaining_input_files.pop() {
                    let mut reader =
                        Mp4AudioReader::new(self.source_id.clone(), next_input_file.clone())?;
                    reader.inherit_stats_from(inner.stats());
                    reader.current_input_file = Some(next_input_file);
                    **inner = reader;
                    Ok(true)
                } else {
                    inner.stats_mut().current_input_file = None;
                    Ok(false)
                }
            }
            AudioReaderInner::Webm(inner) => {
                if let Some(next_input_file) = self.remaining_input_files.pop() {
                    let mut reader =
                        WebmAudioReader::new(self.source_id.clone(), next_input_file.clone())?;
                    reader.inherit_stats_from(inner.stats());
                    reader.current_input_file = Some(next_input_file);
                    **inner = reader;
                    Ok(true)
                } else {
                    inner.stats_mut().current_input_file = None;
                    Ok(false)
                }
            }
        }
    }

    fn update_metrics_from_inner(&mut self) {
        match &self.inner {
            AudioReaderInner::Mp4(reader) => {
                let reader = reader.stats();
                self.total_sample_count_metric
                    .set(reader.total_sample_count as i64);
                self.total_track_seconds_metric.set(
                    (reader.track_duration_offset + reader.total_track_duration).as_secs_f64(),
                );
                if let Some(codec) = reader.codec {
                    self.codec_metric.set(codec.as_str());
                }
                if let Some(path) = &reader.current_input_file {
                    self.current_input_file_metric
                        .set(path.display().to_string());
                }
            }
            AudioReaderInner::Webm(reader) => {
                let reader = reader.stats();
                self.total_cluster_count_metric
                    .set(reader.total_cluster_count as i64);
                self.total_simple_block_count_metric
                    .set(reader.total_simple_block_count as i64);
                self.total_track_seconds_metric.set(
                    (reader.track_duration_offset + reader.total_track_duration).as_secs_f64(),
                );
                if let Some(codec) = reader.codec {
                    self.codec_metric.set(codec.as_str());
                }
                if let Some(path) = &reader.current_input_file {
                    self.current_input_file_metric
                        .set(path.display().to_string());
                }
            }
        }
    }
}

#[derive(Debug)]
enum AudioReaderInner {
    Mp4(Box<Mp4AudioReader>),
    Webm(Box<WebmAudioReader>),
}

impl AudioReaderInner {
    fn set_timestamp_offset(&mut self, offset: Duration) {
        match self {
            Self::Mp4(r) => r.stats_mut().track_duration_offset = offset,
            Self::Webm(r) => r.stats_mut().track_duration_offset = offset,
        }
    }
}

impl Iterator for AudioReaderInner {
    type Item = crate::Result<AudioData>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Mp4(r) => r.next(),
            Self::Webm(r) => r.next(),
        }
    }
}

#[derive(Debug)]
pub struct VideoReader {
    source_id: SourceId,
    timestamp_offset: Duration,
    next_timestamp_offset: Duration,
    remaining_input_files: Vec<PathBuf>,
    inner: VideoReaderInner,
    total_sample_count_metric: crate::stats::StatsGauge,
    total_cluster_count_metric: crate::stats::StatsGauge,
    total_simple_block_count_metric: crate::stats::StatsGauge,
    total_track_seconds_metric: crate::stats::StatsGaugeF64,
    codec_metric: crate::stats::StatsString,
    current_input_file_metric: crate::stats::StatsString,
    error_flag: crate::stats::StatsFlag,
}

impl VideoReader {
    pub async fn run(mut self, handle: crate::ProcessorHandle) -> crate::Result<()> {
        let track_id = crate::TrackId::new(handle.processor_id().get());
        let mut track_handle = handle.publish_track(track_id).await?;
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

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
                    if !self.start_next_input_file()? {
                        // 全てのファイルの末尾に達した
                        break;
                    }
                    self.timestamp_offset = self.next_timestamp_offset;
                    self.inner.set_timestamp_offset(self.timestamp_offset);
                }
                Some(Err(e)) => {
                    self.error_flag.set(true);
                    return Err(e);
                }
                Some(Ok(mut frame)) => {
                    frame.timestamp += self.timestamp_offset;
                    self.next_timestamp_offset = frame.timestamp + frame.duration;
                    self.update_metrics_from_inner();

                    if !track_handle.send_media(MediaSample::new_video(frame)) {
                        // パイプライン処理が中断された
                        break;
                    }
                    noacked_sent += 1;
                }
            }
        }
        self.update_metrics_from_inner();
        track_handle.send_eos();

        Ok(())
    }

    pub fn from_source_info(
        source_info: &AggregatedSourceInfo,
        stats: crate::stats::Stats,
    ) -> crate::Result<Self> {
        Self::new(
            source_info.id.clone(),
            source_info.format,
            source_info.start_timestamp,
            source_info.timestamp_sorted_media_paths(),
            stats,
        )
    }

    pub fn new(
        source_id: SourceId,
        format: ContainerFormat,
        timestamp_offset: Duration,
        input_files: Vec<PathBuf>,
        mut compose_stats: crate::stats::Stats,
    ) -> crate::Result<Self> {
        let mut remaining_input_files = input_files.clone();
        remaining_input_files.reverse();
        let first_input_file = remaining_input_files
            .pop()
            .ok_or_else(|| crate::Error::new("no input file for video reader"))?;
        let inner = match format {
            ContainerFormat::Mp4 => {
                let mut reader = Mp4VideoReader::new(source_id.clone(), first_input_file.clone())?;
                reader.current_input_file = Some(first_input_file.clone());
                VideoReaderInner::Mp4(Box::new(reader))
            }
            ContainerFormat::Webm => {
                let mut reader = WebmVideoReader::new(source_id.clone(), first_input_file.clone())?;
                reader.current_input_file = Some(first_input_file.clone());
                VideoReaderInner::Webm(Box::new(reader))
            }
        };
        compose_stats
            .gauge_f64("start_time_seconds")
            .set(timestamp_offset.as_secs_f64());
        let total_sample_count_metric = compose_stats.gauge("total_sample_count");
        let total_cluster_count_metric = compose_stats.gauge("total_cluster_count");
        let total_simple_block_count_metric = compose_stats.gauge("total_simple_block_count");
        let total_track_seconds_metric = compose_stats.gauge_f64("total_track_seconds");
        let codec_metric = compose_stats.string("codec");
        let current_input_file_metric = compose_stats.string("current_input_file");
        let error_flag = compose_stats.flag("error");
        error_flag.set(false);
        Ok(Self {
            source_id,
            timestamp_offset,
            next_timestamp_offset: timestamp_offset,
            remaining_input_files,
            inner,
            total_sample_count_metric,
            total_cluster_count_metric,
            total_simple_block_count_metric,
            total_track_seconds_metric,
            codec_metric,
            current_input_file_metric,
            error_flag,
        })
    }

    fn start_next_input_file(&mut self) -> crate::Result<bool> {
        match &mut self.inner {
            VideoReaderInner::Mp4(inner) => {
                if let Some(next_input_file) = self.remaining_input_files.pop() {
                    let mut reader =
                        Mp4VideoReader::new(self.source_id.clone(), next_input_file.clone())?;
                    reader.inherit_stats_from(inner.stats());
                    reader.current_input_file = Some(next_input_file);
                    **inner = reader;
                    Ok(true)
                } else {
                    inner.stats_mut().current_input_file = None;
                    Ok(false)
                }
            }
            VideoReaderInner::Webm(inner) => {
                if let Some(next_input_file) = self.remaining_input_files.pop() {
                    let mut reader =
                        WebmVideoReader::new(self.source_id.clone(), next_input_file.clone())?;
                    reader.inherit_stats_from(inner.stats());
                    reader.current_input_file = Some(next_input_file);
                    **inner = reader;
                    Ok(true)
                } else {
                    inner.stats_mut().current_input_file = None;
                    Ok(false)
                }
            }
        }
    }

    fn update_metrics_from_inner(&mut self) {
        match &self.inner {
            VideoReaderInner::Mp4(reader) => {
                let reader = reader.stats();
                self.total_sample_count_metric
                    .set(reader.total_sample_count as i64);
                self.total_track_seconds_metric.set(
                    (reader.track_duration_offset + reader.total_track_duration).as_secs_f64(),
                );
                if let Some(codec) = reader.codec {
                    self.codec_metric.set(codec.as_str());
                }
                if let Some(path) = &reader.current_input_file {
                    self.current_input_file_metric
                        .set(path.display().to_string());
                }
            }
            VideoReaderInner::Webm(reader) => {
                let reader = reader.stats();
                self.total_cluster_count_metric
                    .set(reader.total_cluster_count as i64);
                self.total_simple_block_count_metric
                    .set(reader.total_simple_block_count as i64);
                self.total_track_seconds_metric.set(
                    (reader.track_duration_offset + reader.total_track_duration).as_secs_f64(),
                );
                if let Some(codec) = reader.codec {
                    self.codec_metric.set(codec.as_str());
                }
                if let Some(path) = &reader.current_input_file {
                    self.current_input_file_metric
                        .set(path.display().to_string());
                }
            }
        }
    }
}

#[derive(Debug)]
enum VideoReaderInner {
    Mp4(Box<Mp4VideoReader>),
    Webm(Box<WebmVideoReader>),
}

impl VideoReaderInner {
    fn set_timestamp_offset(&mut self, offset: Duration) {
        match self {
            Self::Mp4(r) => r.stats_mut().track_duration_offset = offset,
            Self::Webm(r) => r.stats_mut().track_duration_offset = offset,
        }
    }
}

impl Iterator for VideoReaderInner {
    type Item = crate::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Mp4(r) => r.next(),
            Self::Webm(r) => r.next(),
        }
    }
}
