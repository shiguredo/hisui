use std::{
    collections::VecDeque,
    fs::File,
    io::{Seek, SeekFrom, Write},
    path::Path,
    sync::Arc,
    time::Duration,
};

use orfail::OrFail;
use shiguredo_mp4::TrackKind;
use shiguredo_mp4::mux::{Mp4FileMuxer, Mp4FileMuxerOptions, Sample};

use crate::{
    audio::AudioData,
    layout::{Layout, Resolution},
    media::{MediaSample, MediaStreamId},
    processor::{
        MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
        MediaProcessorWorkloadHint,
    },
    stats::{Mp4WriterStats, ProcessorStats},
    video::{FrameRate, VideoFrame},
};

#[derive(Debug, Clone)]
pub struct Mp4WriterOptions {
    pub resolution: Resolution,
    pub duration: Duration,
    pub frame_rate: FrameRate,
}

impl Mp4WriterOptions {
    pub fn from_layout(layout: &Layout) -> Self {
        Self {
            resolution: layout.resolution,
            duration: layout.duration(),
            frame_rate: layout.frame_rate,
        }
    }
}

/// 合成結果を含んだ MP4 ファイルを書き出すための構造体
#[derive(Debug)]
pub struct Mp4Writer {
    file: File,
    muxer: Mp4FileMuxer,
    current_offset: u64,
    input_audio_stream_id: Option<MediaStreamId>,
    input_video_stream_id: Option<MediaStreamId>,
    input_audio_queue: VecDeque<Arc<AudioData>>,
    input_video_queue: VecDeque<Arc<VideoFrame>>,
    appending_video_chunk: bool,
    stats: Mp4WriterStats,
}

impl Mp4Writer {
    /// [`Mp4Writer`] インスタンスを生成する
    pub fn new<P: AsRef<Path>>(
        path: P,
        _options: &Mp4WriterOptions,
        input_audio_stream_id: Option<MediaStreamId>,
        input_video_stream_id: Option<MediaStreamId>,
    ) -> orfail::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .or_fail()?;

        let muxer_options = Mp4FileMuxerOptions {
            reserved_moov_box_size: 65536, // 64KB buffer for moov box // TODO: calculate from options
            ..Default::default()
        };

        let stats = Mp4WriterStats::default();
        stats
            .reserved_moov_box_size
            .set(muxer_options.reserved_moov_box_size as u64);

        let muxer = Mp4FileMuxer::with_options(muxer_options).or_fail()?;
        let mut this = Self {
            file,
            muxer,
            current_offset: 0,
            input_audio_stream_id,
            input_video_stream_id,
            input_audio_queue: VecDeque::new(),
            input_video_queue: VecDeque::new(),
            appending_video_chunk: true,
            stats,
        };

        // Write initial boxes
        let initial_bytes = this.muxer.initial_boxes_bytes();
        this.file.write_all(initial_bytes).or_fail()?;
        this.current_offset = initial_bytes.len() as u64;

        Ok(this)
    }

    /// 統計情報を返す
    pub fn stats(&self) -> &Mp4WriterStats {
        &self.stats
    }

    fn append_video_frame(&mut self, _new_chunk: bool) -> orfail::Result<()> {
        let frame = self.input_video_queue.pop_front().or_fail()?;

        if self.stats.video_codec.get().is_none()
            && let Some(name) = frame.format.codec_name()
        {
            self.stats.video_codec.set(name);
        }

        let sample = Sample {
            track_kind: TrackKind::Video,
            sample_entry: frame.sample_entry.clone(),
            keyframe: frame.keyframe,
            duration: frame.duration,
            data_offset: self.current_offset,
            data_size: frame.data.len(),
        };

        self.file.write_all(&frame.data).or_fail()?;
        self.current_offset += frame.data.len() as u64;

        self.muxer.append_sample(&sample).or_fail()?;
        self.stats.total_video_sample_count.add(1);
        self.stats
            .total_video_sample_data_byte_size
            .add(frame.data.len() as u64);
        self.stats.total_video_track_duration.add(frame.duration);
        self.appending_video_chunk = true;

        Ok(())
    }

    fn append_audio_data(&mut self, _new_chunk: bool) -> orfail::Result<()> {
        let data = self.input_audio_queue.pop_front().or_fail()?;

        if self.stats.audio_codec.get().is_none()
            && let Some(name) = data.format.codec_name()
        {
            self.stats.audio_codec.set(name);
        }

        let sample = Sample {
            track_kind: TrackKind::Audio,
            sample_entry: data.sample_entry.clone(),
            keyframe: false,
            duration: data.duration,
            data_offset: self.current_offset,
            data_size: data.data.len(),
        };

        self.file.write_all(&data.data).or_fail()?;
        self.current_offset += data.data.len() as u64;

        self.muxer.append_sample(&sample).or_fail()?;
        self.stats.total_audio_sample_count.add(1);
        self.stats
            .total_audio_sample_data_byte_size
            .add(data.data.len() as u64);
        self.stats.total_audio_track_duration.add(data.duration);
        self.appending_video_chunk = false;

        Ok(())
    }

    fn finalize(&mut self) -> orfail::Result<()> {
        let finalized = self.muxer.finalize().or_fail()?;
        self.stats
            .actual_moov_box_size
            .set(finalized.moov_box_size() as u64);

        // Update chunk counts from finalized data
        self.stats
            .total_audio_chunk_count
            .set(finalized.audio_chunk_count() as u64);
        self.stats
            .total_video_chunk_count
            .set(finalized.video_chunk_count() as u64);

        // Write finalized boxes
        for (offset, bytes) in finalized.offset_and_bytes_pairs() {
            self.file.seek(SeekFrom::Start(offset)).or_fail()?;
            self.file.write_all(bytes).or_fail()?;
        }

        self.file.flush().or_fail()?;
        Ok(())
    }

    pub fn current_duration(&self) -> Duration {
        self.stats
            .total_audio_track_duration
            .get()
            .max(self.stats.total_video_track_duration.get())
    }

    fn handle_next_audio_and_video(
        &mut self,
        audio_timestamp: Option<Duration>,
        video_timestamp: Option<Duration>,
    ) -> orfail::Result<bool> {
        match (audio_timestamp, video_timestamp) {
            (None, None) => {
                self.finalize()?;
                Ok(false)
            }
            (None, Some(_)) => {
                self.append_video_frame(false)?;
                Ok(true)
            }
            (Some(_), None) => {
                self.append_audio_data(false)?;
                Ok(true)
            }
            (Some(_audio_ts), Some(_video_ts)) => {
                self.append_video_frame(false)?;
                Ok(true)
            }
        }
    }
}

impl MediaProcessor for Mp4Writer {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: self
                .input_audio_stream_id
                .into_iter()
                .chain(self.input_video_stream_id)
                .collect(),
            output_stream_ids: Vec::new(),
            stats: ProcessorStats::Mp4Writer(self.stats.clone()),
            workload_hint: MediaProcessorWorkloadHint::WRITER,
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        match input.sample {
            Some(MediaSample::Audio(sample))
                if Some(input.stream_id) == self.input_audio_stream_id =>
            {
                self.input_audio_queue.push_back(sample);
            }
            None if Some(input.stream_id) == self.input_audio_stream_id => {
                self.input_audio_stream_id = None;
            }
            Some(MediaSample::Video(sample))
                if Some(input.stream_id) == self.input_video_stream_id =>
            {
                self.input_video_queue.push_back(sample);
            }
            None if Some(input.stream_id) == self.input_video_stream_id => {
                self.input_video_stream_id = None;
            }
            _ => return Err(orfail::Failure::new("BUG: unexpected input stream")),
        }
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        loop {
            if let Some(id) = self.input_video_stream_id
                && self.input_video_queue.is_empty()
            {
                return Ok(MediaProcessorOutput::pending(id));
            } else if let Some(id) = self.input_audio_stream_id
                && self.input_audio_queue.is_empty()
            {
                return Ok(MediaProcessorOutput::pending(id));
            }

            let audio_timestamp = self.input_audio_queue.front().map(|x| x.timestamp);
            let video_timestamp = self.input_video_queue.front().map(|x| x.timestamp);

            let in_progress = self.handle_next_audio_and_video(audio_timestamp, video_timestamp)?;

            if !in_progress {
                return Ok(MediaProcessorOutput::Finished);
            }
        }
    }
}
