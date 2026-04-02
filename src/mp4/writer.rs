use std::{
    collections::VecDeque,
    fs::File,
    io::{BufWriter, Seek, SeekFrom, Write},
    num::NonZeroU32,
    path::Path,
    sync::Arc,
    time::Duration,
};

use shiguredo_mp4::boxes::{Brand, FreeBox, FtypBox, HdlrBox, MdatBox};
use shiguredo_mp4::mux::{
    Fmp4SegmentMuxer, Mp4FileMuxer, Mp4FileMuxerOptions, SegmentMuxerOptions,
};
use shiguredo_mp4::{BoxHeader, BoxSize, Either, Encode};

use crate::{
    TrackId,
    audio::AudioFrame,
    media::MediaFrame,
    types::CodecName,
    video::{FrameRate, VideoFrame},
};

// Hisui では出力 MP4 のタイムスケールはマイクロ秒固定にする
const TIMESCALE: NonZeroU32 = NonZeroU32::MIN.saturating_add(1_000_000 - 1);

// 映像・音声混在時のチャンクの尺の最大値（映像か音声の片方だけの場合はチャンクは一つだけ）
const MAX_CHUNK_DURATION: Duration = Duration::from_secs(10);
// 末尾サンプルなどで前後関係から duration を再計算できない場合に使う既定値
const DEFAULT_SAMPLE_DURATION: Duration = Duration::from_millis(20);

// 入力がリアルタイムではなくファイルで、
// 映像・音声キューの件数差が大きい場合に、軽い音声側だけが先行して
// メモリを消費し続ける事態を避けるために、件数差が閾値を超えたら
// 大きい方の rx 受信を一時的に抑制するための閾値
//
// 適当に大きな値ならなんでもいい
const MAX_INPUT_QUEUE_GAP: usize = 200;

enum WriterRunOutput {
    Pending {
        awaiting_track_kind: Option<InputTrackKind>,
    },
    Finished,
}

#[derive(Debug, Clone, Copy)]
enum InputTrackKind {
    Audio,
    Video,
}

#[derive(Debug)]
pub enum Mp4WriterRpcMessage {
    Pause {
        reply_tx: tokio::sync::oneshot::Sender<crate::Result<()>>,
    },
    Resume {
        reply_tx: tokio::sync::oneshot::Sender<crate::Result<()>>,
    },
    /// writer を finalize して正常終了する
    Finish {
        reply_tx: tokio::sync::oneshot::Sender<()>,
    },
}

#[derive(Debug, Clone)]
pub struct Mp4WriterOptions {
    pub duration: Duration,
    pub frame_rate: FrameRate,
}

#[derive(Debug)]
pub struct Mp4WriterStats {
    audio_codec: crate::stats::StatsString,
    video_codec: crate::stats::StatsString,
    reserved_moov_box_size: crate::stats::StatsGauge,
    actual_moov_box_size: crate::stats::StatsGauge,
    total_audio_chunk_count: crate::stats::StatsGauge,
    total_video_chunk_count: crate::stats::StatsGauge,
    total_audio_sample_count: crate::stats::StatsCounter,
    total_video_sample_count: crate::stats::StatsCounter,
    total_audio_sample_data_byte_size: crate::stats::StatsCounter,
    total_video_sample_data_byte_size: crate::stats::StatsCounter,
    total_audio_track_duration: crate::stats::StatsDuration,
    total_video_track_duration: crate::stats::StatsDuration,
    total_keyframe_wait_dropped_audio_sample_count: crate::stats::StatsCounter,
    total_keyframe_wait_dropped_video_frame_count: crate::stats::StatsCounter,
    total_received_audio_data_count: crate::stats::StatsCounter,
    total_received_audio_eos_count: crate::stats::StatsCounter,
    total_received_video_data_count: crate::stats::StatsCounter,
    total_received_video_eos_count: crate::stats::StatsCounter,
    error: crate::stats::StatsFlag,
}

impl Mp4WriterStats {
    fn new(stats: &mut crate::stats::Stats, reserved_moov_box_size: u64) -> Self {
        let reserved_moov_box_size_metric = stats.gauge("reserved_moov_box_size");
        reserved_moov_box_size_metric.set(reserved_moov_box_size as i64);
        let actual_moov_box_size = stats.gauge("actual_moov_box_size");
        let total_audio_chunk_count = stats.gauge("total_audio_chunk_count");
        let total_video_chunk_count = stats.gauge("total_video_chunk_count");
        let video_codec = stats.string("video_codec");
        let total_video_sample_count = stats.counter("total_video_sample_count");
        let total_video_sample_data_byte_size = stats.counter("total_video_sample_data_byte_size");
        let total_video_track_duration = stats.duration("total_video_track_seconds");
        let audio_codec = stats.string("audio_codec");
        let total_audio_sample_count = stats.counter("total_audio_sample_count");
        let total_audio_sample_data_byte_size = stats.counter("total_audio_sample_data_byte_size");
        let total_audio_track_duration = stats.duration("total_audio_track_seconds");
        let total_keyframe_wait_dropped_audio_sample_count =
            stats.counter("total_keyframe_wait_dropped_audio_sample_count");
        let total_keyframe_wait_dropped_video_frame_count =
            stats.counter("total_keyframe_wait_dropped_video_frame_count");
        let total_received_audio_data_count = stats.counter("total_received_audio_data_count");
        let total_received_audio_eos_count = stats.counter("total_received_audio_eos_count");
        let total_received_video_data_count = stats.counter("total_received_video_data_count");
        let total_received_video_eos_count = stats.counter("total_received_video_eos_count");
        let error = stats.flag("error");
        error.set(false);
        Self {
            audio_codec,
            video_codec,
            reserved_moov_box_size: reserved_moov_box_size_metric,
            actual_moov_box_size,
            total_audio_chunk_count,
            total_video_chunk_count,
            total_audio_sample_count,
            total_video_sample_count,
            total_audio_sample_data_byte_size,
            total_video_sample_data_byte_size,
            total_audio_track_duration,
            total_video_track_duration,
            total_keyframe_wait_dropped_audio_sample_count,
            total_keyframe_wait_dropped_video_frame_count,
            total_received_audio_data_count,
            total_received_audio_eos_count,
            total_received_video_data_count,
            total_received_video_eos_count,
            error,
        }
    }

    fn set_error(&self) {
        self.error.set(true);
    }

    fn set_actual_moov_box_size(&self, size: u64) {
        self.actual_moov_box_size.set(size as i64);
    }

    fn set_total_audio_chunk_count(&self, count: u64) {
        self.total_audio_chunk_count.set(count as i64);
    }

    fn set_total_video_chunk_count(&self, count: u64) {
        self.total_video_chunk_count.set(count as i64);
    }

    fn set_audio_codec(&self, codec: CodecName) {
        self.audio_codec.set(codec.as_str());
    }

    fn set_video_codec(&self, codec: CodecName) {
        self.video_codec.set(codec.as_str());
    }

    fn add_video_sample(&self, data_size: usize, duration: Duration) {
        self.total_video_sample_count.inc();
        self.total_video_sample_data_byte_size.add(data_size as u64);
        self.total_video_track_duration.add(duration);
    }

    fn add_audio_sample(&self, data_size: usize, duration: Duration) {
        self.total_audio_sample_count.inc();
        self.total_audio_sample_data_byte_size.add(data_size as u64);
        self.total_audio_track_duration.add(duration);
    }

    fn add_keyframe_wait_dropped_video_frame(&self) {
        self.total_keyframe_wait_dropped_video_frame_count.inc();
    }

    fn add_keyframe_wait_dropped_audio_sample(&self) {
        self.total_keyframe_wait_dropped_audio_sample_count.inc();
    }

    fn add_received_audio_data(&self) {
        self.total_received_audio_data_count.inc();
    }

    fn add_received_audio_eos(&self) {
        self.total_received_audio_eos_count.inc();
    }

    fn add_received_video_data(&self) {
        self.total_received_video_data_count.inc();
    }

    fn add_received_video_eos(&self) {
        self.total_received_video_eos_count.inc();
    }

    pub fn audio_codec(&self) -> Option<CodecName> {
        self.audio_codec.get().parse().ok()
    }

    pub fn video_codec(&self) -> Option<CodecName> {
        self.video_codec.get().parse().ok()
    }

    pub fn reserved_moov_box_size(&self) -> u64 {
        self.reserved_moov_box_size.get().max(0) as u64
    }

    pub fn actual_moov_box_size(&self) -> u64 {
        self.actual_moov_box_size.get().max(0) as u64
    }

    pub fn total_audio_chunk_count(&self) -> u64 {
        self.total_audio_chunk_count.get().max(0) as u64
    }

    pub fn total_video_chunk_count(&self) -> u64 {
        self.total_video_chunk_count.get().max(0) as u64
    }

    pub fn total_audio_sample_count(&self) -> u64 {
        self.total_audio_sample_count.get()
    }

    pub fn total_video_sample_count(&self) -> u64 {
        self.total_video_sample_count.get()
    }

    pub fn total_audio_sample_data_byte_size(&self) -> u64 {
        self.total_audio_sample_data_byte_size.get()
    }

    pub fn total_video_sample_data_byte_size(&self) -> u64 {
        self.total_video_sample_data_byte_size.get()
    }

    pub fn total_audio_track_duration(&self) -> Duration {
        self.total_audio_track_duration.get()
    }

    pub fn total_video_track_duration(&self) -> Duration {
        self.total_video_track_duration.get()
    }

    pub fn total_keyframe_wait_dropped_video_frame_count(&self) -> u64 {
        self.total_keyframe_wait_dropped_video_frame_count.get()
    }

    pub fn total_keyframe_wait_dropped_audio_sample_count(&self) -> u64 {
        self.total_keyframe_wait_dropped_audio_sample_count.get()
    }
}

/// 合成結果を含んだ MP4 ファイルを書き出すための構造体
#[derive(Debug)]
pub struct Mp4Writer {
    file: BufWriter<File>,
    muxer: Mp4FileMuxer,
    next_position: u64,
    input_audio_track_id: Option<TrackId>,
    input_video_track_id: Option<TrackId>,
    input_audio_queue: VecDeque<Arc<AudioFrame>>,
    input_video_queue: VecDeque<Arc<VideoFrame>>,
    pending_audio_sample: Option<Arc<AudioFrame>>,
    pending_video_frame: Option<Arc<VideoFrame>>,
    last_audio_duration: Option<Duration>,
    last_video_duration: Option<Duration>,
    paused: bool,
    resume_waiting_for_keyframe: bool,
    resume_offset_update_pending: bool,
    pause_anchor_timestamp: Option<Duration>,
    timeline_timestamp_offset: Duration,
    appending_video_chunk: bool,
    stats: Mp4WriterStats,
}

impl Mp4Writer {
    /// [`Mp4Writer`] インスタンスを生成する
    pub fn new<P: AsRef<Path>>(
        path: P,
        options: Option<Mp4WriterOptions>, // ライブの場合は None になる
        input_audio_track_id: Option<TrackId>,
        input_video_track_id: Option<TrackId>,
        mut stats: crate::stats::Stats,
    ) -> crate::Result<Self> {
        let reserved_moov_box_size = if let Some(options) = options {
            // 事前に尺などが分かっている場合には fast start 用の領域を計算する

            let mut sample_counts = Vec::new();
            if input_audio_track_id.is_some() {
                // 音声サンプルの尺は 20 ms と仮定する（つまり一秒に 50 sample）
                let count = options.duration.as_secs() * 50;
                sample_counts.push(count as usize);
            }
            if input_video_track_id.is_some() {
                let count = options.duration.as_secs() as f64 * options.frame_rate.as_f64();
                sample_counts.push(count.ceil() as usize);
            }
            shiguredo_mp4::mux::estimate_maximum_moov_box_size(&sample_counts)
        } else {
            0
        };
        let muxer_options = Mp4FileMuxerOptions {
            creation_timestamp: std::time::UNIX_EPOCH.elapsed()?,
            reserved_moov_box_size,
        };
        let muxer = Mp4FileMuxer::with_options(muxer_options)?;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)?;
        let initial_bytes = muxer.initial_boxes_bytes();
        file.write_all(initial_bytes)?;

        let next_position = initial_bytes.len() as u64;
        let stats = Mp4WriterStats::new(&mut stats, reserved_moov_box_size as u64);

        Ok(Self {
            file: BufWriter::new(file),
            muxer,
            next_position,
            input_audio_track_id,
            input_video_track_id,
            input_audio_queue: VecDeque::new(),
            input_video_queue: VecDeque::new(),
            pending_audio_sample: None,
            pending_video_frame: None,
            last_audio_duration: None,
            last_video_duration: None,
            paused: false,
            resume_waiting_for_keyframe: false,
            resume_offset_update_pending: false,
            pause_anchor_timestamp: None,
            timeline_timestamp_offset: Duration::ZERO,
            appending_video_chunk: true,
            stats,
        })
    }

    /// 統計情報を返す
    pub fn stats(&self) -> &Mp4WriterStats {
        &self.stats
    }

    pub fn current_duration(&self) -> Duration {
        self.stats
            .total_audio_track_duration()
            .max(self.stats.total_video_track_duration())
    }

    fn pause_recording(&mut self) -> crate::Result<()> {
        if self.paused {
            return Err(crate::Error::new("recording is already paused"));
        }
        self.paused = true;
        self.resume_waiting_for_keyframe = false;
        self.resume_offset_update_pending = false;
        Ok(())
    }

    fn resume_recording(&mut self) -> crate::Result<()> {
        if !self.paused {
            return Err(crate::Error::new("recording is not paused"));
        }
        self.paused = false;
        self.resume_waiting_for_keyframe = self.input_video_track_id.is_some();
        self.resume_offset_update_pending = true;
        Ok(())
    }

    fn maybe_set_pause_anchor(&mut self, timestamp: Duration) {
        if self.pause_anchor_timestamp.is_none() {
            self.pause_anchor_timestamp = Some(timestamp);
        }
    }

    fn maybe_apply_pause_offset(&mut self, resume_timestamp: Duration) {
        if !self.resume_offset_update_pending {
            return;
        }
        if let Some(pause_anchor_timestamp) = self.pause_anchor_timestamp.take() {
            let paused_duration = resume_timestamp.saturating_sub(pause_anchor_timestamp);
            self.timeline_timestamp_offset += paused_duration;
        }
        self.resume_offset_update_pending = false;
    }

    fn apply_timestamp_offset(&self, timestamp: Duration) -> Duration {
        timestamp.saturating_sub(self.timeline_timestamp_offset)
    }

    fn prepare_audio_for_queue(&mut self, sample: Arc<AudioFrame>) -> Option<Arc<AudioFrame>> {
        if self.paused {
            self.maybe_set_pause_anchor(sample.timestamp);
            return None;
        }
        if self.resume_waiting_for_keyframe {
            self.stats.add_keyframe_wait_dropped_audio_sample();
            return None;
        }
        self.maybe_apply_pause_offset(sample.timestamp);
        let mut sample = sample.as_ref().clone();
        sample.timestamp = self.apply_timestamp_offset(sample.timestamp);
        Some(Arc::new(sample))
    }

    fn prepare_video_for_queue(&mut self, frame: Arc<VideoFrame>) -> Option<Arc<VideoFrame>> {
        if self.paused {
            self.maybe_set_pause_anchor(frame.timestamp);
            return None;
        }
        if self.resume_waiting_for_keyframe {
            if !frame.keyframe {
                self.stats.add_keyframe_wait_dropped_video_frame();
                return None;
            }
            self.maybe_apply_pause_offset(frame.timestamp);
            self.resume_waiting_for_keyframe = false;
        } else {
            self.maybe_apply_pause_offset(frame.timestamp);
        }
        let mut frame = frame.as_ref().clone();
        frame.timestamp = self.apply_timestamp_offset(frame.timestamp);
        Some(Arc::new(frame))
    }

    fn handle_next_audio_and_video(&mut self) -> crate::Result<bool> {
        self.flush_pending_audio_if_ready()?;
        self.flush_pending_video_if_ready()?;

        let audio_timestamp = self.input_audio_queue.front().map(|x| x.timestamp);
        let video_timestamp = self.input_video_queue.front().map(|x| x.timestamp);
        match (audio_timestamp, video_timestamp) {
            (None, None) => {
                if self.pending_audio_sample.is_some() || self.pending_video_frame.is_some() {
                    // pending が残っている場合はフラッシュ後に再評価する
                    return Ok(true);
                }
                // 全部の入力の処理が完了した
                let finalized = self.muxer.finalize()?;

                let actual_moov_size = finalized.moov_box_size() as u64;
                self.stats.set_actual_moov_box_size(actual_moov_size);

                for (offset, bytes) in finalized.offset_and_bytes_pairs() {
                    self.file.seek(SeekFrom::Start(offset))?;
                    self.file.write_all(bytes)?;
                }
                self.file.flush()?;

                self.update_finalized_chunk_counts()?;

                return Ok(false);
            }
            (None, Some(_)) => {
                // 残りは映像のみ
                self.process_next_video_frame()?;
            }
            (Some(_), None) => {
                // 残りは音声のみ
                self.process_next_audio_sample()?;
            }
            (Some(audio_timestamp), Some(video_timestamp)) => {
                if self.appending_video_chunk
                    && video_timestamp.saturating_sub(audio_timestamp) > MAX_CHUNK_DURATION
                {
                    // 音声が一定以上遅れている場合は映像に追従する
                    self.process_next_audio_sample()?;
                } else if !self.appending_video_chunk && video_timestamp > audio_timestamp {
                    // 一度音声追記モードに入った場合には、映像に追いつくまでは音声を追記し続ける
                    self.process_next_audio_sample()?;
                } else {
                    // 音声との差が一定以内の場合は、映像の処理を進める
                    self.process_next_video_frame()?;
                }
            }
        }

        Ok(true)
    }

    // 確定したチャンク数を統計値に反映する
    fn update_finalized_chunk_counts(&mut self) -> crate::Result<()> {
        let finalized = self
            .muxer
            .finalized_boxes()
            .ok_or_else(|| crate::Error::new("muxer finalized boxes are not available"))?;
        let moov_box = finalized.moov_box();

        for trak in &moov_box.trak_boxes {
            let stbl = &trak.mdia_box.minf_box.stbl_box;

            let chunk_count = match &stbl.stco_or_co64_box {
                Either::A(stco) => stco.chunk_offsets.len() as u64,
                Either::B(co64) => co64.chunk_offsets.len() as u64,
            };

            match trak.mdia_box.hdlr_box.handler_type {
                HdlrBox::HANDLER_TYPE_SOUN => {
                    self.stats.set_total_audio_chunk_count(chunk_count);
                }
                HdlrBox::HANDLER_TYPE_VIDE => {
                    self.stats.set_total_video_chunk_count(chunk_count);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn sample_duration_from_timestamps(
        current_timestamp: Duration,
        next_timestamp: Duration,
        last_duration: Option<Duration>,
    ) -> Duration {
        if next_timestamp > current_timestamp {
            next_timestamp.saturating_sub(current_timestamp)
        } else {
            last_duration.unwrap_or(DEFAULT_SAMPLE_DURATION)
        }
    }

    fn append_pending_video_frame(&mut self, duration: Duration) -> crate::Result<()> {
        let frame = self
            .pending_video_frame
            .take()
            .ok_or_else(|| crate::Error::new("pending video frame is unexpectedly empty"))?;

        if self.stats.video_codec().is_none()
            && let Some(name) = frame.format.codec_name()
        {
            self.stats.set_video_codec(name);
        }

        self.file.write_all(&frame.data)?;
        let data_offset = self.next_position;
        let sample = shiguredo_mp4::mux::Sample {
            track_kind: shiguredo_mp4::TrackKind::Video,
            sample_entry: frame.sample_entry.clone(),
            keyframe: frame.keyframe,
            timescale: TIMESCALE,
            duration: duration.as_micros() as u32,
            composition_time_offset: None,
            data_offset,
            data_size: frame.data.len(),
        };
        self.muxer.append_sample(&sample)?;
        self.next_position += frame.data.len() as u64;
        self.stats.add_video_sample(frame.data.len(), duration);
        self.last_video_duration = Some(duration);
        Ok(())
    }

    fn append_pending_audio_sample(&mut self, duration: Duration) -> crate::Result<()> {
        let data = self
            .pending_audio_sample
            .take()
            .ok_or_else(|| crate::Error::new("pending audio sample is unexpectedly empty"))?;

        if self.stats.audio_codec().is_none()
            && let Some(name) = data.format.codec_name()
        {
            self.stats.set_audio_codec(name);
        }

        self.file.write_all(&data.data)?;
        let data_offset = self.next_position;
        let sample = shiguredo_mp4::mux::Sample {
            track_kind: shiguredo_mp4::TrackKind::Audio,
            sample_entry: data.sample_entry.clone(),
            keyframe: true,
            timescale: TIMESCALE,
            duration: duration.as_micros() as u32,
            composition_time_offset: None,
            data_offset,
            data_size: data.data.len(),
        };
        self.muxer.append_sample(&sample)?;
        self.next_position += data.data.len() as u64;
        self.stats.add_audio_sample(data.data.len(), duration);
        self.last_audio_duration = Some(duration);
        Ok(())
    }

    fn process_next_video_frame(&mut self) -> crate::Result<()> {
        let frame = self
            .input_video_queue
            .pop_front()
            .ok_or_else(|| crate::Error::new("video input queue is unexpectedly empty"))?;

        if let Some(pending) = self.pending_video_frame.as_ref() {
            let duration = Self::sample_duration_from_timestamps(
                pending.timestamp,
                frame.timestamp,
                self.last_video_duration,
            );
            self.append_pending_video_frame(duration)?;
        }
        self.pending_video_frame = Some(frame);
        self.appending_video_chunk = true;
        Ok(())
    }

    fn process_next_audio_sample(&mut self) -> crate::Result<()> {
        let data = self
            .input_audio_queue
            .pop_front()
            .ok_or_else(|| crate::Error::new("audio input queue is unexpectedly empty"))?;

        if let Some(pending) = self.pending_audio_sample.as_ref() {
            let duration = Self::sample_duration_from_timestamps(
                pending.timestamp,
                data.timestamp,
                self.last_audio_duration,
            );
            self.append_pending_audio_sample(duration)?;
        }
        self.pending_audio_sample = Some(data);
        self.appending_video_chunk = false;
        Ok(())
    }

    fn flush_pending_audio_if_ready(&mut self) -> crate::Result<()> {
        if self.input_audio_track_id.is_none()
            && self.input_audio_queue.is_empty()
            && self.pending_audio_sample.is_some()
        {
            let duration = self.last_audio_duration.unwrap_or(DEFAULT_SAMPLE_DURATION);
            self.append_pending_audio_sample(duration)?;
        }
        Ok(())
    }

    fn flush_pending_video_if_ready(&mut self) -> crate::Result<()> {
        if self.input_video_track_id.is_none()
            && self.input_video_queue.is_empty()
            && self.pending_video_frame.is_some()
        {
            let duration = self.last_video_duration.unwrap_or(DEFAULT_SAMPLE_DURATION);
            self.append_pending_video_frame(duration)?;
        }
        Ok(())
    }
}

impl Mp4Writer {
    fn handle_input_sample(
        &mut self,
        track_kind: InputTrackKind,
        sample: Option<MediaFrame>,
    ) -> crate::Result<()> {
        match (track_kind, sample) {
            (InputTrackKind::Audio, Some(MediaFrame::Audio(sample))) => {
                if let Some(sample) = self.prepare_audio_for_queue(sample) {
                    self.input_audio_queue.push_back(sample);
                }
            }
            (InputTrackKind::Audio, None) => {
                self.input_audio_track_id = None;
            }
            (InputTrackKind::Video, Some(MediaFrame::Video(sample))) => {
                if let Some(sample) = self.prepare_video_for_queue(sample) {
                    self.input_video_queue.push_back(sample);
                }
            }
            (InputTrackKind::Video, None) => {
                self.input_video_track_id = None;
                self.resume_waiting_for_keyframe = false;
            }
            _ => {
                self.stats.set_error();
                return Err(crate::Error::new("BUG: unexpected input stream"));
            }
        }
        Ok(())
    }

    fn poll_output(&mut self) -> crate::Result<WriterRunOutput> {
        loop {
            if self.input_video_track_id.is_some() && self.input_video_queue.is_empty() {
                return Ok(WriterRunOutput::Pending {
                    awaiting_track_kind: Some(InputTrackKind::Video),
                });
            } else if self.input_audio_track_id.is_some() && self.input_audio_queue.is_empty() {
                return Ok(WriterRunOutput::Pending {
                    awaiting_track_kind: Some(InputTrackKind::Audio),
                });
            }

            let in_progress = self.handle_next_audio_and_video()?;

            if !in_progress {
                return Ok(WriterRunOutput::Finished);
            }
        }
    }
    pub async fn run(
        mut self,
        handle: crate::ProcessorHandle,
        input_audio_track_id: Option<crate::TrackId>,
        input_video_track_id: Option<crate::TrackId>,
    ) -> crate::Result<()> {
        let mut audio_rx = input_audio_track_id.map(|id| handle.subscribe_track(id));
        let mut video_rx = input_video_track_id.map(|id| handle.subscribe_track(id));
        let (rpc_tx, mut rpc_rx) = tokio::sync::mpsc::unbounded_channel();
        handle.register_rpc_sender(rpc_tx).await.map_err(|e| {
            crate::Error::new(format!("failed to register mp4 writer RPC sender: {e}"))
        })?;

        handle.notify_ready();

        // 起動直後に上流 video encoder へキーフレーム要求を送る
        if video_rx.is_some()
            && let Err(e) = crate::encoder::request_upstream_video_keyframe(
                &handle.pipeline_handle(),
                handle.processor_id(),
                "mp4_writer_start",
            )
            .await
        {
            tracing::warn!(
                "failed to request keyframe for mp4 writer start: {}",
                e.display()
            );
        }
        let mut rpc_rx_enabled = true;

        // 入力トラックが 0 本でも finalize まで到達する。
        let mut output = self.poll_output()?;
        loop {
            match output {
                WriterRunOutput::Finished => break,
                WriterRunOutput::Pending {
                    awaiting_track_kind,
                } => {
                    if audio_rx.is_none() && video_rx.is_none() {
                        output = self.poll_output()?;
                        continue;
                    }

                    match awaiting_track_kind {
                        Some(InputTrackKind::Audio) if audio_rx.is_some() => {
                            tokio::select! {
                                msg = crate::future::recv_or_pending(&mut audio_rx) => {
                                    self.handle_audio_message(msg, &mut audio_rx)?;
                                }
                                rpc_message = recv_mp4_writer_rpc_message_or_pending(
                                    rpc_rx_enabled.then_some(&mut rpc_rx)
                                ) => {
                                    if self.handle_rpc_message(rpc_message, &mut rpc_rx_enabled)? {
                                        audio_rx = None;
                                        video_rx = None;
                                    }
                                }
                            }
                        }
                        Some(InputTrackKind::Video) if video_rx.is_some() => {
                            tokio::select! {
                                msg = crate::future::recv_or_pending(&mut video_rx) => {
                                    self.handle_video_message(msg, &mut video_rx)?;
                                }
                                rpc_message = recv_mp4_writer_rpc_message_or_pending(
                                    rpc_rx_enabled.then_some(&mut rpc_rx)
                                ) => {
                                    if self.handle_rpc_message(rpc_message, &mut rpc_rx_enabled)? {
                                        audio_rx = None;
                                        video_rx = None;
                                    }
                                }
                            }
                        }
                        _ => {
                            let audio_len = self.input_audio_queue.len();
                            let video_len = self.input_video_queue.len();
                            let mut suppress_audio = false;
                            let mut suppress_video = false;
                            if audio_rx.is_some() && video_rx.is_some() {
                                if audio_len > video_len + MAX_INPUT_QUEUE_GAP {
                                    suppress_audio = true;
                                } else if video_len > audio_len + MAX_INPUT_QUEUE_GAP {
                                    suppress_video = true;
                                }
                            }

                            tokio::select! {
                                msg = crate::future::recv_or_pending(&mut audio_rx), if !suppress_audio => {
                                    self.handle_audio_message(msg, &mut audio_rx)?;
                                }
                                msg = crate::future::recv_or_pending(&mut video_rx), if !suppress_video => {
                                    self.handle_video_message(msg, &mut video_rx)?;
                                }
                                rpc_message = recv_mp4_writer_rpc_message_or_pending(
                                    rpc_rx_enabled.then_some(&mut rpc_rx)
                                ) => {
                                    if self.handle_rpc_message(rpc_message, &mut rpc_rx_enabled)? {
                                        audio_rx = None;
                                        video_rx = None;
                                    }
                                }
                            }
                        }
                    }
                    output = self.poll_output()?;
                }
            }
        }

        Ok(())
    }

    /// RPC メッセージを処理する。Finish を受け取った場合は true を返す。
    fn handle_rpc_message(
        &mut self,
        rpc_message: Option<Mp4WriterRpcMessage>,
        rpc_rx_enabled: &mut bool,
    ) -> crate::Result<bool> {
        let Some(rpc_message) = rpc_message else {
            *rpc_rx_enabled = false;
            return Ok(false);
        };

        match rpc_message {
            Mp4WriterRpcMessage::Pause { reply_tx } => {
                let _ = reply_tx.send(self.pause_recording());
            }
            Mp4WriterRpcMessage::Resume { reply_tx } => {
                let _ = reply_tx.send(self.resume_recording());
            }
            Mp4WriterRpcMessage::Finish { reply_tx } => {
                let _ = reply_tx.send(());
                *rpc_rx_enabled = false;
                // 入力トラックを閉じて finalize に遷移させる
                self.input_video_track_id = None;
                self.input_audio_track_id = None;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn handle_audio_message(
        &mut self,
        msg: crate::Message,
        audio_rx: &mut Option<crate::MessageReceiver>,
    ) -> crate::Result<()> {
        match msg {
            crate::Message::Media(crate::MediaFrame::Audio(sample)) => {
                self.stats.add_received_audio_data();
                if self.input_audio_track_id.is_some() {
                    self.handle_input_sample(
                        InputTrackKind::Audio,
                        Some(crate::MediaFrame::Audio(sample)),
                    )?;
                }
            }
            crate::Message::Eos => {
                self.stats.add_received_audio_eos();
                if self.input_audio_track_id.is_some() {
                    self.handle_input_sample(InputTrackKind::Audio, None)?;
                }
                *audio_rx = None;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_video_message(
        &mut self,
        msg: crate::Message,
        video_rx: &mut Option<crate::MessageReceiver>,
    ) -> crate::Result<()> {
        match msg {
            crate::Message::Media(crate::MediaFrame::Video(sample)) => {
                self.stats.add_received_video_data();
                if self.input_video_track_id.is_some() {
                    self.handle_input_sample(
                        InputTrackKind::Video,
                        Some(crate::MediaFrame::Video(sample)),
                    )?;
                }
            }
            crate::Message::Eos => {
                self.stats.add_received_video_eos();
                if self.input_video_track_id.is_some() {
                    self.handle_input_sample(InputTrackKind::Video, None)?;
                }
                *video_rx = None;
            }
            _ => {}
        }
        Ok(())
    }
}

async fn recv_mp4_writer_rpc_message_or_pending(
    rpc_rx: Option<&mut tokio::sync::mpsc::UnboundedReceiver<Mp4WriterRpcMessage>>,
) -> Option<Mp4WriterRpcMessage> {
    if let Some(rpc_rx) = rpc_rx {
        rpc_rx.recv().await
    } else {
        std::future::pending().await
    }
}

pub async fn create_processor(
    handle: &crate::MediaPipelineHandle,
    output_path: std::path::PathBuf,
    input_audio_track_id: Option<crate::TrackId>,
    input_video_track_id: Option<crate::TrackId>,
    processor_id: Option<crate::ProcessorId>,
) -> crate::Result<crate::ProcessorId> {
    if input_audio_track_id.is_none() && input_video_track_id.is_none() {
        return Err(crate::Error::new(
            "inputAudioTrackId or inputVideoTrackId is required".to_owned(),
        ));
    }

    let is_mp4 = output_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"));
    if !is_mp4 {
        return Err(crate::Error::new(format!(
            "outputPath must be an mp4 file: {}",
            output_path.display()
        )));
    }

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        return Err(crate::Error::new(format!(
            "outputPath parent directory does not exist: {}",
            parent.display()
        )));
    }

    let processor_id = processor_id.unwrap_or_else(|| crate::ProcessorId::new("mp4Writer"));
    handle
        .spawn_processor(
            processor_id.clone(),
            crate::ProcessorMetadata::new("mp4_writer"),
            move |h| async move {
                let writer = Mp4Writer::new(
                    &output_path,
                    None,
                    input_audio_track_id.clone(),
                    input_video_track_id.clone(),
                    h.stats(),
                )?;
                writer
                    .run(h, input_audio_track_id, input_video_track_id)
                    .await
            },
        )
        .await
        .map_err(|e| crate::Error::new(format!("{e}: {processor_id}")))?;
    Ok(processor_id)
}

// hybrid MP4 のリカバリ用 moov を格納するための free ボックスの予約サイズ（バイト単位）
//
// 録画中に fMP4 用の moov をこの領域内に定期的に上書きすることで、
// クラッシュ時にも再生可能なファイルを維持する。
// moov のサイズはトラック数やサンプル数に依存するが、
// fMP4 の moov は mvex/trex ベースで stbl が空のため比較的小さい。
const HYBRID_FREE_BOX_RESERVED_SIZE: usize = 512 * 1024;

// フラグメントを自動フラッシュするまでの最大蓄積時間
const HYBRID_FRAGMENT_MAX_DURATION: Duration = Duration::from_secs(2);

/// Hybrid MP4 ライター
///
/// 録画中は fMP4（fragmented MP4）形式で書き込み、
/// 正常終了時に標準 MP4 に変換する。
/// プロセスクラッシュ時にはフラグメント単位で再生可能なファイルが残る。
///
/// ファイルレイアウト:
/// - 録画中: `[ftyp][moov(fMP4用)][free][moof1][mdat1][moof2][mdat2]...`
/// - finalize 後: `[ftyp][mdat(全データ)][moov(標準MP4)]`
#[derive(Debug)]
pub struct HybridMp4Writer {
    file: BufWriter<File>,

    // fMP4 フラグメント生成用
    fmp4_muxer: Fmp4SegmentMuxer,
    // finalize 時の標準 MP4 moov 生成用
    mp4_muxer: Mp4FileMuxer,

    // ファイルレイアウト
    ftyp_end_offset: u64,
    free_box_total_size: u64,
    next_position: u64,

    // フラグメントバッファ（トラック別に管理する）
    // Fmp4SegmentMuxer は同一トラックのサンプルが mdat 内で連続配置されることを要求するため、
    // 映像と音声を分離して蓄積し、フラッシュ時に [映像データ][音声データ] として結合する。
    fragment_video_payload: Vec<u8>,
    fragment_audio_payload: Vec<u8>,
    fragment_video_samples: Vec<shiguredo_mp4::mux::Sample>,
    fragment_audio_samples: Vec<shiguredo_mp4::mux::Sample>,
    // フラグメント内の蓄積時間（自動フラッシュ判定用）
    fragment_accumulated_duration: Duration,
    // エンコーダーは sample_entry を初回のみ付与することがあるため保持する
    last_audio_sample_entry: Option<shiguredo_mp4::boxes::SampleEntry>,
    last_video_sample_entry: Option<shiguredo_mp4::boxes::SampleEntry>,

    // 既存 Mp4Writer と同じフィールド群
    input_audio_track_id: Option<TrackId>,
    input_video_track_id: Option<TrackId>,
    input_audio_queue: VecDeque<Arc<AudioFrame>>,
    input_video_queue: VecDeque<Arc<VideoFrame>>,
    pending_audio_sample: Option<Arc<AudioFrame>>,
    pending_video_frame: Option<Arc<VideoFrame>>,
    last_audio_duration: Option<Duration>,
    last_video_duration: Option<Duration>,
    paused: bool,
    resume_waiting_for_keyframe: bool,
    resume_offset_update_pending: bool,
    pause_anchor_timestamp: Option<Duration>,
    timeline_timestamp_offset: Duration,
    appending_video_chunk: bool,
    stats: Mp4WriterStats,
}

impl HybridMp4Writer {
    pub fn new<P: AsRef<Path>>(
        path: P,
        input_audio_track_id: Option<TrackId>,
        input_video_track_id: Option<TrackId>,
        mut stats: crate::stats::Stats,
    ) -> crate::Result<Self> {
        let creation_timestamp = std::time::UNIX_EPOCH.elapsed()?;

        // fMP4 フラグメント生成用 muxer
        let fmp4_muxer =
            Fmp4SegmentMuxer::with_options(SegmentMuxerOptions { creation_timestamp })?;

        // finalize 時の標準 MP4 moov 生成用 muxer
        let mut mp4_muxer = Mp4FileMuxer::with_options(Mp4FileMuxerOptions {
            creation_timestamp,
            reserved_moov_box_size: 0,
        })?;

        // ファイルを作成
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(path)?;

        // ftyp ボックスを書き出す
        let ftyp_box = FtypBox {
            major_brand: Brand::ISOM,
            minor_version: 0,
            compatible_brands: vec![Brand::ISOM, Brand::ISO2, Brand::MP41],
        };
        let ftyp_bytes = ftyp_box.encode_to_vec()?;
        file.write_all(&ftyp_bytes)?;
        let ftyp_end_offset = ftyp_bytes.len() as u64;

        // free ボックスを書き出す（リカバリ用 moov の予約領域）
        let free_payload_size = HYBRID_FREE_BOX_RESERVED_SIZE;
        let free_box = FreeBox {
            payload: vec![0; free_payload_size],
        };
        let free_bytes = free_box.encode_to_vec()?;
        let free_box_total_size = free_bytes.len() as u64;
        file.write_all(&free_bytes)?;

        let next_position = ftyp_end_offset + free_box_total_size;

        // mp4_muxer の内部 next_position を実ファイル位置に合わせる
        // mp4_muxer は initial_boxes_bytes 分の next_position を持っているので、
        // 実ファイル位置との差分を advance_position で調整する
        let mp4_initial_size = mp4_muxer.initial_boxes_bytes().len() as u64;
        if next_position > mp4_initial_size {
            mp4_muxer.advance_position(next_position - mp4_initial_size)?;
        }

        let stats = Mp4WriterStats::new(&mut stats, 0);

        Ok(Self {
            file: BufWriter::new(file),
            fmp4_muxer,
            mp4_muxer,
            ftyp_end_offset,
            free_box_total_size,
            next_position,
            fragment_video_payload: Vec::new(),
            fragment_audio_payload: Vec::new(),
            fragment_video_samples: Vec::new(),
            fragment_audio_samples: Vec::new(),
            fragment_accumulated_duration: Duration::ZERO,
            last_audio_sample_entry: None,
            last_video_sample_entry: None,
            input_audio_track_id,
            input_video_track_id,
            input_audio_queue: VecDeque::new(),
            input_video_queue: VecDeque::new(),
            pending_audio_sample: None,
            pending_video_frame: None,
            last_audio_duration: None,
            last_video_duration: None,
            paused: false,
            resume_waiting_for_keyframe: false,
            resume_offset_update_pending: false,
            pause_anchor_timestamp: None,
            timeline_timestamp_offset: Duration::ZERO,
            appending_video_chunk: true,
            stats,
        })
    }

    pub fn stats(&self) -> &Mp4WriterStats {
        &self.stats
    }

    pub fn current_duration(&self) -> Duration {
        self.stats
            .total_audio_track_duration()
            .max(self.stats.total_video_track_duration())
    }

    fn pause_recording(&mut self) -> crate::Result<()> {
        if self.paused {
            return Err(crate::Error::new("recording is already paused"));
        }
        self.paused = true;
        self.resume_waiting_for_keyframe = false;
        self.resume_offset_update_pending = false;
        Ok(())
    }

    fn resume_recording(&mut self) -> crate::Result<()> {
        if !self.paused {
            return Err(crate::Error::new("recording is not paused"));
        }
        self.paused = false;
        self.resume_waiting_for_keyframe = self.input_video_track_id.is_some();
        self.resume_offset_update_pending = true;
        Ok(())
    }

    fn maybe_set_pause_anchor(&mut self, timestamp: Duration) {
        if self.pause_anchor_timestamp.is_none() {
            self.pause_anchor_timestamp = Some(timestamp);
        }
    }

    fn maybe_apply_pause_offset(&mut self, resume_timestamp: Duration) {
        if !self.resume_offset_update_pending {
            return;
        }
        if let Some(pause_anchor_timestamp) = self.pause_anchor_timestamp.take() {
            let paused_duration = resume_timestamp.saturating_sub(pause_anchor_timestamp);
            self.timeline_timestamp_offset += paused_duration;
        }
        self.resume_offset_update_pending = false;
    }

    fn apply_timestamp_offset(&self, timestamp: Duration) -> Duration {
        timestamp.saturating_sub(self.timeline_timestamp_offset)
    }

    fn prepare_audio_for_queue(&mut self, sample: Arc<AudioFrame>) -> Option<Arc<AudioFrame>> {
        if self.paused {
            self.maybe_set_pause_anchor(sample.timestamp);
            return None;
        }
        if self.resume_waiting_for_keyframe {
            self.stats.add_keyframe_wait_dropped_audio_sample();
            return None;
        }
        self.maybe_apply_pause_offset(sample.timestamp);
        let mut sample = sample.as_ref().clone();
        sample.timestamp = self.apply_timestamp_offset(sample.timestamp);
        Some(Arc::new(sample))
    }

    fn prepare_video_for_queue(&mut self, frame: Arc<VideoFrame>) -> Option<Arc<VideoFrame>> {
        if self.paused {
            self.maybe_set_pause_anchor(frame.timestamp);
            return None;
        }
        if self.resume_waiting_for_keyframe {
            if !frame.keyframe {
                self.stats.add_keyframe_wait_dropped_video_frame();
                return None;
            }
            self.maybe_apply_pause_offset(frame.timestamp);
            self.resume_waiting_for_keyframe = false;
        } else {
            self.maybe_apply_pause_offset(frame.timestamp);
        }
        let mut frame = frame.as_ref().clone();
        frame.timestamp = self.apply_timestamp_offset(frame.timestamp);
        Some(Arc::new(frame))
    }

    /// フラグメントにビデオサンプルを追加する
    fn append_video_to_fragment(&mut self, frame: &VideoFrame, duration: Duration) {
        // 映像 payload 内の相対オフセット（フラッシュ時に音声分をオフセットして最終位置を計算する）
        let offset_in_video = self.fragment_video_payload.len() as u64;
        self.fragment_video_payload.extend_from_slice(&frame.data);

        if self.stats.video_codec().is_none()
            && let Some(name) = frame.format.codec_name()
        {
            self.stats.set_video_codec(name);
        }

        if frame.sample_entry.is_some() {
            self.last_video_sample_entry.clone_from(&frame.sample_entry);
        }

        self.fragment_video_samples
            .push(shiguredo_mp4::mux::Sample {
                track_kind: shiguredo_mp4::TrackKind::Video,
                sample_entry: frame
                    .sample_entry
                    .clone()
                    .or_else(|| self.last_video_sample_entry.clone()),
                keyframe: frame.keyframe,
                timescale: TIMESCALE,
                duration: duration.as_micros() as u32,
                composition_time_offset: None,
                data_offset: offset_in_video,
                data_size: frame.data.len(),
            });

        self.stats.add_video_sample(frame.data.len(), duration);
        self.last_video_duration = Some(duration);
        self.fragment_accumulated_duration += duration;
    }

    /// フラグメントにオーディオサンプルを追加する
    fn append_audio_to_fragment(&mut self, sample: &AudioFrame, duration: Duration) {
        let offset_in_audio = self.fragment_audio_payload.len() as u64;
        self.fragment_audio_payload.extend_from_slice(&sample.data);

        if self.stats.audio_codec().is_none()
            && let Some(name) = sample.format.codec_name()
        {
            self.stats.set_audio_codec(name);
        }

        if sample.sample_entry.is_some() {
            self.last_audio_sample_entry
                .clone_from(&sample.sample_entry);
        }

        self.fragment_audio_samples
            .push(shiguredo_mp4::mux::Sample {
                track_kind: shiguredo_mp4::TrackKind::Audio,
                sample_entry: sample
                    .sample_entry
                    .clone()
                    .or_else(|| self.last_audio_sample_entry.clone()),
                keyframe: true,
                timescale: TIMESCALE,
                duration: duration.as_micros() as u32,
                composition_time_offset: None,
                data_offset: offset_in_audio,
                data_size: sample.data.len(),
            });

        self.stats.add_audio_sample(sample.data.len(), duration);
        self.last_audio_duration = Some(duration);
        self.fragment_accumulated_duration += duration;
    }

    fn has_fragment_samples(&self) -> bool {
        !self.fragment_video_samples.is_empty() || !self.fragment_audio_samples.is_empty()
    }

    /// 蓄積されたフラグメントをファイルに書き出す
    ///
    /// mdat payload 内のレイアウトは [映像データ][音声データ] の順。
    /// Fmp4SegmentMuxer は同一トラックのサンプルが連続配置されることを要求するため、
    /// トラックごとに分離して蓄積したデータをこの順序で結合する。
    fn flush_fragment(&mut self) -> crate::Result<()> {
        if !self.has_fragment_samples() {
            return Ok(());
        }

        // fmp4_muxer 用のサンプルリストを構築する
        // レイアウト: [映像サンプル群][音声サンプル群]
        // 音声の data_offset は映像データの合計サイズ分だけオフセットする
        let video_total_size = self.fragment_video_payload.len() as u64;
        let mut fmp4_samples = self.fragment_video_samples.clone();
        for sample in &self.fragment_audio_samples {
            let mut s = sample.clone();
            s.data_offset += video_total_size;
            fmp4_samples.push(s);
        }

        // fmp4_muxer でメディアセグメントメタデータ（moof + mdat ヘッダ）を生成する
        let metadata_bytes = self
            .fmp4_muxer
            .create_media_segment_metadata(&fmp4_samples)?;

        // ファイルに metadata + payload を書き出す
        // payload は [映像データ][音声データ] の順
        self.file.seek(SeekFrom::Start(self.next_position))?;
        self.file.write_all(&metadata_bytes)?;
        let payload_start = self.next_position + metadata_bytes.len() as u64;
        self.file.write_all(&self.fragment_video_payload)?;
        self.file.write_all(&self.fragment_audio_payload)?;

        // mp4_muxer: metadata 分のギャップを進める
        self.mp4_muxer
            .advance_position(metadata_bytes.len() as u64)?;

        // mp4_muxer: 映像サンプル → 音声サンプルの順で追加する
        // data_offset は実ファイル上の絶対位置
        let mut offset = payload_start;
        for sample in &self.fragment_video_samples {
            let mut s = sample.clone();
            s.data_offset = offset;
            self.mp4_muxer.append_sample(&s)?;
            offset += s.data_size as u64;
        }
        for sample in &self.fragment_audio_samples {
            let mut s = sample.clone();
            s.data_offset = offset;
            self.mp4_muxer.append_sample(&s)?;
            offset += s.data_size as u64;
        }

        self.next_position = offset;

        // フラグメントバッファをクリア
        self.fragment_video_payload.clear();
        self.fragment_audio_payload.clear();
        self.fragment_video_samples.clear();
        self.fragment_audio_samples.clear();
        self.fragment_accumulated_duration = Duration::ZERO;

        // リカバリ用 moov を更新する
        self.update_recovery_moov()?;

        Ok(())
    }

    /// 蓄積時間が閾値を超えた場合にフラグメントをフラッシュする
    fn maybe_flush_fragment_by_duration(&mut self) -> crate::Result<()> {
        if self.fragment_accumulated_duration >= HYBRID_FRAGMENT_MAX_DURATION
            && self.has_fragment_samples()
        {
            self.flush_fragment()?;
        }
        Ok(())
    }

    /// free ボックス領域にリカバリ用の fMP4 moov を書き込む
    fn update_recovery_moov(&mut self) -> crate::Result<()> {
        let init_bytes = match self.fmp4_muxer.init_segment_bytes() {
            Ok(bytes) => bytes,
            Err(shiguredo_mp4::mux::MuxError::EmptyTracks) => {
                return Ok(());
            }
            Err(e) => {
                return Err(crate::Error::new(format!(
                    "failed to get init segment: {e}"
                )));
            }
        };

        // init_segment_bytes（ftyp + moov）から moov 部分を取得する
        let moov_bytes = extract_moov_from_init_segment(&init_bytes)?;
        let moov_size = moov_bytes.len() as u64;

        // free ボックスの最小サイズは 8 バイト（ヘッダのみ）
        const MIN_FREE_BOX_SIZE: u64 = 8;

        if moov_size + MIN_FREE_BOX_SIZE > self.free_box_total_size
            && moov_size != self.free_box_total_size
        {
            tracing::warn!(
                moov_size,
                free_box_total_size = self.free_box_total_size,
                "recovery moov exceeds reserved free box size, skipping update"
            );
            return Ok(());
        }

        // free 領域に [moov][free(残余パディング)] を書き込む
        self.file.seek(SeekFrom::Start(self.ftyp_end_offset))?;
        self.file.write_all(moov_bytes)?;

        let remaining = self.free_box_total_size - moov_size;
        if remaining > 0 {
            let free_box = FreeBox {
                payload: vec![0; remaining as usize - 8],
            };
            let free_bytes = free_box.encode_to_vec()?;
            self.file.write_all(&free_bytes)?;
        }

        self.file.flush()?;

        Ok(())
    }

    /// 録画を finalize して標準 MP4 に変換する
    fn finalize(&mut self) -> crate::Result<()> {
        // 残りのフラグメントをフラッシュ
        self.flush_fragment()?;

        // free ボックスのヘッダを mdat ヘッダに書き換える
        // mdat のサイズ = next_position - ftyp_end_offset
        let mdat_size = self.next_position - self.ftyp_end_offset;
        let mdat_header = BoxHeader {
            box_type: MdatBox::TYPE,
            box_size: BoxSize::U64(mdat_size),
        };
        let mdat_header_bytes = mdat_header.encode_to_vec()?;

        self.file.seek(SeekFrom::Start(self.ftyp_end_offset))?;
        self.file.write_all(&mdat_header_bytes)?;

        // mp4_muxer を finalize して標準 MP4 の moov を取得する
        let finalized = self.mp4_muxer.finalize()?;
        let actual_moov_size = finalized.moov_box_size() as u64;
        self.stats.set_actual_moov_box_size(actual_moov_size);

        // moov を EOF に追記する
        // offset_and_bytes_pairs は (0, head_boxes), (moov_offset, moov_bytes),
        // (mdat_offset, mdat_header) の順で返す。
        // hybrid MP4 では moov_bytes のみを使う（head_boxes と mdat_header は不要）。
        // moov は offset が最大のエントリとして取得する。
        self.file.seek(SeekFrom::Start(self.next_position))?;
        if let Some((_offset, moov_bytes)) = finalized
            .offset_and_bytes_pairs()
            .max_by_key(|(offset, _)| *offset)
        {
            self.file.write_all(moov_bytes)?;
        }

        self.file.flush()?;

        Ok(())
    }

    fn handle_next_audio_and_video(&mut self) -> crate::Result<bool> {
        self.flush_pending_audio_if_ready()?;
        self.flush_pending_video_if_ready()?;

        let audio_timestamp = self.input_audio_queue.front().map(|x| x.timestamp);
        let video_timestamp = self.input_video_queue.front().map(|x| x.timestamp);
        match (audio_timestamp, video_timestamp) {
            (None, None) => {
                if self.pending_audio_sample.is_some() || self.pending_video_frame.is_some() {
                    return Ok(true);
                }
                // 全入力の処理が完了 → finalize
                self.finalize()?;
                return Ok(false);
            }
            (None, Some(_)) => {
                self.process_next_video_frame()?;
            }
            (Some(_), None) => {
                self.process_next_audio_sample()?;
            }
            (Some(audio_timestamp), Some(video_timestamp)) => {
                let should_process_audio = (self.appending_video_chunk
                    && video_timestamp.saturating_sub(audio_timestamp) > MAX_CHUNK_DURATION)
                    || (!self.appending_video_chunk && video_timestamp > audio_timestamp);
                if should_process_audio {
                    self.process_next_audio_sample()?;
                } else {
                    self.process_next_video_frame()?;
                }
            }
        }

        Ok(true)
    }

    fn process_next_video_frame(&mut self) -> crate::Result<()> {
        let frame = self
            .input_video_queue
            .pop_front()
            .ok_or_else(|| crate::Error::new("video input queue is unexpectedly empty"))?;

        // キーフレーム到着時にフラグメントを区切る
        if frame.keyframe && self.has_fragment_samples() {
            self.flush_fragment()?;
        }

        if let Some(pending) = self.pending_video_frame.as_ref() {
            let duration = Self::sample_duration_from_timestamps(
                pending.timestamp,
                frame.timestamp,
                self.last_video_duration,
            );
            let pending = pending.clone();
            self.append_video_to_fragment(&pending, duration);

            // 蓄積時間が閾値を超えた場合もフラッシュする
            self.maybe_flush_fragment_by_duration()?;
        }
        self.pending_video_frame = Some(frame);
        self.appending_video_chunk = true;
        Ok(())
    }

    fn process_next_audio_sample(&mut self) -> crate::Result<()> {
        let data = self
            .input_audio_queue
            .pop_front()
            .ok_or_else(|| crate::Error::new("audio input queue is unexpectedly empty"))?;

        if let Some(pending) = self.pending_audio_sample.as_ref() {
            let duration = Self::sample_duration_from_timestamps(
                pending.timestamp,
                data.timestamp,
                self.last_audio_duration,
            );
            let pending = pending.clone();
            self.append_audio_to_fragment(&pending, duration);

            self.maybe_flush_fragment_by_duration()?;
        }
        self.pending_audio_sample = Some(data);
        self.appending_video_chunk = false;
        Ok(())
    }

    fn flush_pending_audio_if_ready(&mut self) -> crate::Result<()> {
        if self.input_audio_track_id.is_none()
            && self.input_audio_queue.is_empty()
            && self.pending_audio_sample.is_some()
        {
            let duration = self.last_audio_duration.unwrap_or(DEFAULT_SAMPLE_DURATION);
            let pending = self
                .pending_audio_sample
                .take()
                .expect("pending audio sample is unexpectedly empty");
            self.append_audio_to_fragment(&pending, duration);
        }
        Ok(())
    }

    fn flush_pending_video_if_ready(&mut self) -> crate::Result<()> {
        if self.input_video_track_id.is_none()
            && self.input_video_queue.is_empty()
            && self.pending_video_frame.is_some()
        {
            let duration = self.last_video_duration.unwrap_or(DEFAULT_SAMPLE_DURATION);
            let pending = self
                .pending_video_frame
                .take()
                .expect("pending video frame is unexpectedly empty");
            self.append_video_to_fragment(&pending, duration);
        }
        Ok(())
    }

    fn sample_duration_from_timestamps(
        current_timestamp: Duration,
        next_timestamp: Duration,
        last_duration: Option<Duration>,
    ) -> Duration {
        if next_timestamp > current_timestamp {
            next_timestamp.saturating_sub(current_timestamp)
        } else {
            last_duration.unwrap_or(DEFAULT_SAMPLE_DURATION)
        }
    }
}

/// init_segment_bytes（ftyp + moov）から moov 部分のバイト列を取得する
fn extract_moov_from_init_segment(init_bytes: &[u8]) -> crate::Result<&[u8]> {
    if init_bytes.len() < 8 {
        return Err(crate::Error::new(
            "init segment is too small to contain ftyp box",
        ));
    }
    let ftyp_size =
        u32::from_be_bytes([init_bytes[0], init_bytes[1], init_bytes[2], init_bytes[3]]) as usize;
    if ftyp_size > init_bytes.len() {
        return Err(crate::Error::new("ftyp box size exceeds init segment size"));
    }
    Ok(&init_bytes[ftyp_size..])
}

// HybridMp4Writer の async 入力処理と RPC ハンドリング
impl HybridMp4Writer {
    fn handle_input_sample(
        &mut self,
        track_kind: InputTrackKind,
        sample: Option<MediaFrame>,
    ) -> crate::Result<()> {
        match (track_kind, sample) {
            (InputTrackKind::Audio, Some(MediaFrame::Audio(sample))) => {
                if let Some(sample) = self.prepare_audio_for_queue(sample) {
                    self.input_audio_queue.push_back(sample);
                }
            }
            (InputTrackKind::Audio, None) => {
                self.input_audio_track_id = None;
            }
            (InputTrackKind::Video, Some(MediaFrame::Video(sample))) => {
                if let Some(sample) = self.prepare_video_for_queue(sample) {
                    self.input_video_queue.push_back(sample);
                }
            }
            (InputTrackKind::Video, None) => {
                self.input_video_track_id = None;
                self.resume_waiting_for_keyframe = false;
            }
            _ => {
                self.stats.set_error();
                return Err(crate::Error::new("BUG: unexpected input stream"));
            }
        }
        Ok(())
    }

    fn poll_output(&mut self) -> crate::Result<WriterRunOutput> {
        loop {
            let waiting_video =
                self.input_video_track_id.is_some() && self.input_video_queue.is_empty();
            let waiting_audio =
                self.input_audio_track_id.is_some() && self.input_audio_queue.is_empty();

            if waiting_video && waiting_audio {
                return Ok(WriterRunOutput::Pending {
                    awaiting_track_kind: None,
                });
            } else if waiting_video && self.input_audio_track_id.is_none() {
                return Ok(WriterRunOutput::Pending {
                    awaiting_track_kind: Some(InputTrackKind::Video),
                });
            } else if waiting_audio && self.input_video_track_id.is_none() {
                return Ok(WriterRunOutput::Pending {
                    awaiting_track_kind: Some(InputTrackKind::Audio),
                });
            }

            let in_progress = self.handle_next_audio_and_video()?;

            if !in_progress {
                return Ok(WriterRunOutput::Finished);
            }
        }
    }

    pub async fn run(
        mut self,
        handle: crate::ProcessorHandle,
        input_audio_track_id: Option<crate::TrackId>,
        input_video_track_id: Option<crate::TrackId>,
    ) -> crate::Result<()> {
        let mut audio_rx = input_audio_track_id.map(|id| handle.subscribe_track(id));
        let mut video_rx = input_video_track_id.map(|id| handle.subscribe_track(id));
        let (rpc_tx, mut rpc_rx) = tokio::sync::mpsc::unbounded_channel();
        handle.register_rpc_sender(rpc_tx).await.map_err(|e| {
            crate::Error::new(format!("failed to register mp4 writer RPC sender: {e}"))
        })?;

        handle.notify_ready();

        if video_rx.is_some()
            && let Err(e) = crate::encoder::request_upstream_video_keyframe(
                &handle.pipeline_handle(),
                handle.processor_id(),
                "hybrid_mp4_writer_start",
            )
            .await
        {
            tracing::warn!(
                "failed to request keyframe for hybrid mp4 writer start: {}",
                e.display()
            );
        }
        let mut rpc_rx_enabled = true;

        let mut output = self.poll_output()?;
        loop {
            match output {
                WriterRunOutput::Finished => break,
                WriterRunOutput::Pending {
                    awaiting_track_kind,
                } => {
                    if audio_rx.is_none() && video_rx.is_none() {
                        output = self.poll_output()?;
                        continue;
                    }

                    match awaiting_track_kind {
                        Some(InputTrackKind::Audio) if audio_rx.is_some() => {
                            tokio::select! {
                                msg = crate::future::recv_or_pending(&mut audio_rx) => {
                                    self.handle_audio_message(msg, &mut audio_rx)?;
                                }
                                rpc_message = recv_mp4_writer_rpc_message_or_pending(
                                    rpc_rx_enabled.then_some(&mut rpc_rx)
                                ) => {
                                    if self.handle_rpc_message(rpc_message, &mut rpc_rx_enabled)? {
                                        audio_rx = None;
                                        video_rx = None;
                                    }
                                }
                            }
                        }
                        Some(InputTrackKind::Video) if video_rx.is_some() => {
                            tokio::select! {
                                msg = crate::future::recv_or_pending(&mut video_rx) => {
                                    self.handle_video_message(msg, &mut video_rx)?;
                                }
                                rpc_message = recv_mp4_writer_rpc_message_or_pending(
                                    rpc_rx_enabled.then_some(&mut rpc_rx)
                                ) => {
                                    if self.handle_rpc_message(rpc_message, &mut rpc_rx_enabled)? {
                                        audio_rx = None;
                                        video_rx = None;
                                    }
                                }
                            }
                        }
                        _ => {
                            let audio_len = self.input_audio_queue.len();
                            let video_len = self.input_video_queue.len();
                            let mut suppress_audio = false;
                            let mut suppress_video = false;
                            if audio_rx.is_some() && video_rx.is_some() {
                                if audio_len > video_len + MAX_INPUT_QUEUE_GAP {
                                    suppress_audio = true;
                                } else if video_len > audio_len + MAX_INPUT_QUEUE_GAP {
                                    suppress_video = true;
                                }
                            }

                            tokio::select! {
                                msg = crate::future::recv_or_pending(&mut audio_rx), if !suppress_audio => {
                                    self.handle_audio_message(msg, &mut audio_rx)?;
                                }
                                msg = crate::future::recv_or_pending(&mut video_rx), if !suppress_video => {
                                    self.handle_video_message(msg, &mut video_rx)?;
                                }
                                rpc_message = recv_mp4_writer_rpc_message_or_pending(
                                    rpc_rx_enabled.then_some(&mut rpc_rx)
                                ) => {
                                    if self.handle_rpc_message(rpc_message, &mut rpc_rx_enabled)? {
                                        audio_rx = None;
                                        video_rx = None;
                                    }
                                }
                            }
                        }
                    }
                    output = self.poll_output()?;
                }
            }
        }

        Ok(())
    }

    fn handle_rpc_message(
        &mut self,
        rpc_message: Option<Mp4WriterRpcMessage>,
        rpc_rx_enabled: &mut bool,
    ) -> crate::Result<bool> {
        let Some(rpc_message) = rpc_message else {
            *rpc_rx_enabled = false;
            return Ok(false);
        };

        match rpc_message {
            Mp4WriterRpcMessage::Pause { reply_tx } => {
                let _ = reply_tx.send(self.pause_recording());
            }
            Mp4WriterRpcMessage::Resume { reply_tx } => {
                let _ = reply_tx.send(self.resume_recording());
            }
            Mp4WriterRpcMessage::Finish { reply_tx } => {
                let _ = reply_tx.send(());
                *rpc_rx_enabled = false;
                self.input_video_track_id = None;
                self.input_audio_track_id = None;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn handle_audio_message(
        &mut self,
        msg: crate::Message,
        audio_rx: &mut Option<crate::MessageReceiver>,
    ) -> crate::Result<()> {
        match msg {
            crate::Message::Media(crate::MediaFrame::Audio(sample)) => {
                self.stats.add_received_audio_data();
                if self.input_audio_track_id.is_some() {
                    self.handle_input_sample(
                        InputTrackKind::Audio,
                        Some(crate::MediaFrame::Audio(sample)),
                    )?;
                }
            }
            crate::Message::Eos => {
                self.stats.add_received_audio_eos();
                if self.input_audio_track_id.is_some() {
                    self.handle_input_sample(InputTrackKind::Audio, None)?;
                }
                *audio_rx = None;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_video_message(
        &mut self,
        msg: crate::Message,
        video_rx: &mut Option<crate::MessageReceiver>,
    ) -> crate::Result<()> {
        match msg {
            crate::Message::Media(crate::MediaFrame::Video(sample)) => {
                self.stats.add_received_video_data();
                if self.input_video_track_id.is_some() {
                    self.handle_input_sample(
                        InputTrackKind::Video,
                        Some(crate::MediaFrame::Video(sample)),
                    )?;
                }
            }
            crate::Message::Eos => {
                self.stats.add_received_video_eos();
                if self.input_video_track_id.is_some() {
                    self.handle_input_sample(InputTrackKind::Video, None)?;
                }
                *video_rx = None;
            }
            _ => {}
        }
        Ok(())
    }
}

pub async fn create_hybrid_processor(
    handle: &crate::MediaPipelineHandle,
    output_path: std::path::PathBuf,
    input_audio_track_id: Option<crate::TrackId>,
    input_video_track_id: Option<crate::TrackId>,
    processor_id: Option<crate::ProcessorId>,
) -> crate::Result<crate::ProcessorId> {
    if input_audio_track_id.is_none() && input_video_track_id.is_none() {
        return Err(crate::Error::new(
            "inputAudioTrackId or inputVideoTrackId is required".to_owned(),
        ));
    }

    let is_mp4 = output_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"));
    if !is_mp4 {
        return Err(crate::Error::new(format!(
            "outputPath must be an mp4 file: {}",
            output_path.display()
        )));
    }

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        return Err(crate::Error::new(format!(
            "outputPath parent directory does not exist: {}",
            parent.display()
        )));
    }

    let processor_id = processor_id.unwrap_or_else(|| crate::ProcessorId::new("hybridMp4Writer"));
    handle
        .spawn_processor(
            processor_id.clone(),
            crate::ProcessorMetadata::new("hybrid_mp4_writer"),
            move |h| async move {
                let writer = HybridMp4Writer::new(
                    &output_path,
                    input_audio_track_id.clone(),
                    input_video_track_id.clone(),
                    h.stats(),
                )?;
                writer
                    .run(h, input_audio_track_id, input_video_track_id)
                    .await
            },
        )
        .await
        .map_err(|e| crate::Error::new(format!("{e}: {processor_id}")))?;
    Ok(processor_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{
        audio::{AudioFormat, Channels, SampleRate},
        types::EvenUsize,
        video::VideoFormat,
    };

    fn make_hybrid_writer() -> crate::Result<(tempfile::TempDir, HybridMp4Writer)> {
        let temp_dir = tempfile::tempdir()?;
        let output_path = temp_dir.path().join("test.mp4");
        let writer = HybridMp4Writer::new(
            &output_path,
            Some(TrackId::new("audio")),
            Some(TrackId::new("video")),
            crate::stats::Stats::new(),
        )?;
        Ok((temp_dir, writer))
    }

    fn make_audio_frame(sample_entry: Option<shiguredo_mp4::boxes::SampleEntry>) -> AudioFrame {
        AudioFrame {
            data: vec![0x11, 0x22, 0x33],
            format: AudioFormat::Aac,
            channels: Channels::STEREO,
            sample_rate: SampleRate::HZ_48000,
            timestamp: Duration::ZERO,
            sample_entry,
        }
    }

    fn make_video_frame(sample_entry: Option<shiguredo_mp4::boxes::SampleEntry>) -> VideoFrame {
        VideoFrame {
            data: vec![0x00, 0x00, 0x00, 0x01],
            format: VideoFormat::Av1,
            keyframe: true,
            size: Some(crate::video::VideoFrameSize {
                width: 16,
                height: 16,
            }),
            timestamp: Duration::ZERO,
            sample_entry,
        }
    }

    #[test]
    fn hybrid_writer_keeps_audio_sample_entry_across_fragments() -> crate::Result<()> {
        let (_temp_dir, mut writer) = make_hybrid_writer()?;
        let sample_entry = crate::audio::aac::create_mp4a_sample_entry(
            &[0x12, 0x10],
            SampleRate::HZ_48000,
            Channels::STEREO,
        )?;

        writer.append_audio_to_fragment(
            &make_audio_frame(Some(sample_entry.clone())),
            DEFAULT_SAMPLE_DURATION,
        );
        writer.flush_fragment()?;
        writer.append_audio_to_fragment(&make_audio_frame(None), DEFAULT_SAMPLE_DURATION);

        assert_eq!(writer.fragment_audio_samples.len(), 1);
        assert_eq!(
            writer.fragment_audio_samples[0].sample_entry,
            Some(sample_entry)
        );
        Ok(())
    }

    #[test]
    fn hybrid_writer_keeps_video_sample_entry_across_fragments() -> crate::Result<()> {
        let (_temp_dir, mut writer) = make_hybrid_writer()?;
        let sample_entry = crate::video::av1::av1_sample_entry(
            EvenUsize::MIN_CELL_SIZE,
            EvenUsize::MIN_CELL_SIZE,
            &[0x0A],
        );

        writer.append_video_to_fragment(
            &make_video_frame(Some(sample_entry.clone())),
            DEFAULT_SAMPLE_DURATION,
        );
        writer.flush_fragment()?;
        writer.append_video_to_fragment(&make_video_frame(None), DEFAULT_SAMPLE_DURATION);

        assert_eq!(writer.fragment_video_samples.len(), 1);
        assert_eq!(
            writer.fragment_video_samples[0].sample_entry,
            Some(sample_entry)
        );
        Ok(())
    }

    #[test]
    fn hybrid_writer_consumes_audio_queue_before_waiting_for_video() -> crate::Result<()> {
        let (_temp_dir, mut writer) = make_hybrid_writer()?;
        writer
            .input_audio_queue
            .push_back(Arc::new(make_audio_frame(None)));

        let output = writer.poll_output()?;

        assert!(matches!(
            output,
            WriterRunOutput::Pending {
                awaiting_track_kind: None
            }
        ));
        assert!(writer.pending_audio_sample.is_some());
        Ok(())
    }
}
