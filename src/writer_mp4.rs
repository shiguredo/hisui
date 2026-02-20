use std::{
    collections::VecDeque,
    fs::File,
    io::{BufWriter, Seek, SeekFrom, Write},
    num::NonZeroU32,
    path::Path,
    sync::Arc,
    time::Duration,
};

use orfail::OrFail;
use shiguredo_mp4::Either;
use shiguredo_mp4::boxes::HdlrBox;
use shiguredo_mp4::mux::{Mp4FileMuxer, Mp4FileMuxerOptions};

use crate::{
    audio::AudioData,
    layout::Layout,
    media::{MediaSample, MediaStreamId},
    processor::{
        MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
        MediaProcessorWorkloadHint,
    },
    types::CodecName,
    video::{FrameRate, VideoFrame},
};

// Hisui では出力 MP4 のタイムスケールはマイクロ秒固定にする
const TIMESCALE: NonZeroU32 = NonZeroU32::MIN.saturating_add(1_000_000 - 1);

// 映像・音声混在時のチャンクの尺の最大値（映像か音声の片方だけの場合はチャンクは一つだけ）
const MAX_CHUNK_DURATION: Duration = Duration::from_secs(10);

// 入力がリアルタイムではなくファイルで、
// 映像・音声キューの件数差が大きい場合に、軽い音声側だけが先行して
// メモリを消費し続ける事態を避けるために、件数差が閾値を超えたら
// 大きい方の rx 受信を一時的に抑制するための閾値
//
// 適当に大きな値ならなんでもいい
const MAX_INPUT_QUEUE_GAP: usize = 200;

#[derive(Debug, Clone)]
pub struct Mp4WriterOptions {
    pub duration: Duration,
    pub frame_rate: FrameRate,
}

impl Mp4WriterOptions {
    pub fn from_layout(layout: &Layout) -> Self {
        Self {
            duration: layout.duration(),
            frame_rate: layout.frame_rate,
        }
    }
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
}

/// 合成結果を含んだ MP4 ファイルを書き出すための構造体
#[derive(Debug)]
pub struct Mp4Writer {
    file: BufWriter<File>,
    muxer: Mp4FileMuxer,
    next_position: u64,
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
        options: Option<Mp4WriterOptions>, // ライブの場合は None になる
        input_audio_stream_id: Option<MediaStreamId>,
        input_video_stream_id: Option<MediaStreamId>,
    ) -> orfail::Result<Self> {
        Self::new_with_stats(
            path,
            options,
            input_audio_stream_id,
            input_video_stream_id,
            crate::stats::Stats::new(),
        )
    }

    pub fn new_with_stats<P: AsRef<Path>>(
        path: P,
        options: Option<Mp4WriterOptions>, // ライブの場合は None になる
        input_audio_stream_id: Option<MediaStreamId>,
        input_video_stream_id: Option<MediaStreamId>,
        mut compose_stats: crate::stats::Stats,
    ) -> orfail::Result<Self> {
        let reserved_moov_box_size = if let Some(options) = options {
            // 事前に尺などが分かっている場合には fast start 用の領域を計算する

            let mut sample_counts = Vec::new();
            if input_audio_stream_id.is_some() {
                // 音声サンプルの尺は 20 ms と仮定する（つまり一秒に 50 sample）
                let count = options.duration.as_secs() * 50;
                sample_counts.push(count as usize);
            }
            if input_video_stream_id.is_some() {
                let count = options.duration.as_secs() as f64 * options.frame_rate.as_f64();
                sample_counts.push(count.ceil() as usize);
            }
            shiguredo_mp4::mux::estimate_maximum_moov_box_size(&sample_counts)
        } else {
            0
        };
        let muxer_options = Mp4FileMuxerOptions {
            creation_timestamp: std::time::UNIX_EPOCH.elapsed().or_fail()?,
            reserved_moov_box_size,
        };
        let muxer = Mp4FileMuxer::with_options(muxer_options).or_fail()?;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .or_fail()?;
        let initial_bytes = muxer.initial_boxes_bytes();
        file.write_all(initial_bytes).or_fail()?;

        let next_position = initial_bytes.len() as u64;
        let stats = Mp4WriterStats::new(&mut compose_stats, reserved_moov_box_size as u64);

        Ok(Self {
            file: BufWriter::new(file),
            muxer,
            next_position,
            input_audio_stream_id,
            input_video_stream_id,
            input_audio_queue: VecDeque::new(),
            input_video_queue: VecDeque::new(),
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

    fn handle_next_audio_and_video(
        &mut self,
        audio_timestamp: Option<Duration>,
        video_timestamp: Option<Duration>,
    ) -> orfail::Result<bool> {
        match (audio_timestamp, video_timestamp) {
            (None, None) => {
                // 全部の入力の処理が完了した
                let finalized = self.muxer.finalize().or_fail()?;

                let actual_moov_size = finalized.moov_box_size() as u64;
                self.stats.set_actual_moov_box_size(actual_moov_size);

                for (offset, bytes) in finalized.offset_and_bytes_pairs() {
                    self.file.seek(SeekFrom::Start(offset)).or_fail()?;
                    self.file.write_all(bytes).or_fail()?;
                }
                self.file.flush().or_fail()?;

                self.update_finalized_chunk_counts()?;

                return Ok(false);
            }
            (None, Some(_)) => {
                // 残りは映像のみ
                self.append_video_frame().or_fail()?;
            }
            (Some(_), None) => {
                // 残りは音声のみ
                self.append_audio_data().or_fail()?;
            }
            (Some(audio_timestamp), Some(video_timestamp)) => {
                if self.appending_video_chunk
                    && video_timestamp.saturating_sub(audio_timestamp) > MAX_CHUNK_DURATION
                {
                    // 音声が一定以上遅れている場合は映像に追従する
                    self.append_audio_data().or_fail()?;
                } else if !self.appending_video_chunk && video_timestamp > audio_timestamp {
                    // 一度音声追記モードに入った場合には、映像に追いつくまでは音声を追記し続ける
                    self.append_audio_data().or_fail()?;
                } else {
                    // 音声との差が一定以内の場合は、映像の処理を進める
                    self.append_video_frame().or_fail()?;
                }
            }
        }

        Ok(true)
    }

    // 確定したチャンク数を統計値に反映する
    fn update_finalized_chunk_counts(&mut self) -> orfail::Result<()> {
        let moov_box = self.muxer.finalized_boxes().or_fail()?.moov_box();

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

    fn append_video_frame(&mut self) -> orfail::Result<()> {
        // 次の入力を取り出す（これは常に成功する）
        let frame = self.input_video_queue.pop_front().or_fail()?;

        if self.stats.video_codec().is_none()
            && let Some(name) = frame.format.codec_name()
        {
            self.stats.set_video_codec(name);
        }

        // ファイルへのデータ追記
        self.file.write_all(&frame.data).or_fail()?;
        let data_offset = self.next_position;

        // muxer へのサンプル登録
        let sample = shiguredo_mp4::mux::Sample {
            track_kind: shiguredo_mp4::TrackKind::Video,
            sample_entry: frame.sample_entry.clone(),
            keyframe: frame.keyframe,
            timescale: TIMESCALE,
            duration: frame.duration.as_micros() as u32,
            data_offset,
            data_size: frame.data.len(),
        };
        self.muxer.append_sample(&sample).or_fail()?;

        // ポジションを更新
        self.next_position += frame.data.len() as u64;

        self.stats
            .add_video_sample(frame.data.len(), frame.duration);
        self.appending_video_chunk = true;
        Ok(())
    }

    fn append_audio_data(&mut self) -> orfail::Result<()> {
        // 次の入力を取り出す（これは常に成功する）
        let data = self.input_audio_queue.pop_front().or_fail()?;

        if self.stats.audio_codec().is_none()
            && let Some(name) = data.format.codec_name()
        {
            self.stats.set_audio_codec(name);
        }

        // ファイルへのデータ追記
        self.file.write_all(&data.data).or_fail()?;
        let data_offset = self.next_position;

        // muxer へのサンプル登録
        let sample = shiguredo_mp4::mux::Sample {
            track_kind: shiguredo_mp4::TrackKind::Audio,
            sample_entry: data.sample_entry.clone(),
            keyframe: true,
            timescale: TIMESCALE,
            duration: data.duration.as_micros() as u32,
            data_offset,
            data_size: data.data.len(),
        };
        self.muxer.append_sample(&sample).or_fail()?;

        // ポジションを更新
        self.next_position += data.data.len() as u64;

        self.stats.add_audio_sample(data.data.len(), data.duration);
        self.appending_video_chunk = false;
        Ok(())
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

            let in_progress = self
                .handle_next_audio_and_video(audio_timestamp, video_timestamp)
                .or_fail()?;

            if !in_progress {
                return Ok(MediaProcessorOutput::Finished);
            }
        }
    }

    fn set_error(&self) {
        self.stats.set_error();
    }
}

impl Mp4Writer {
    pub async fn run(
        mut self,
        handle: crate::ProcessorHandle,
        input_audio_track_id: Option<crate::TrackId>,
        input_video_track_id: Option<crate::TrackId>,
    ) -> crate::Result<()> {
        let mut audio_rx = input_audio_track_id.map(|id| handle.subscribe_track(id));
        let mut video_rx = input_video_track_id.map(|id| handle.subscribe_track(id));
        handle.notify_ready();

        let mut in_progress = audio_rx.is_some() || video_rx.is_some();
        while in_progress {
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
                    self.handle_audio_message(msg, &mut audio_rx);
                }
                msg = crate::future::recv_or_pending(&mut video_rx), if !suppress_video => {
                    self.handle_video_message(msg, &mut video_rx);
                }
            }

            let audio_timestamp = self.input_audio_queue.front().map(|x| x.timestamp);
            let video_timestamp = self.input_video_queue.front().map(|x| x.timestamp);

            in_progress = self
                .handle_next_audio_and_video(audio_timestamp, video_timestamp)
                .map_err(|e| crate::Error::new(e.to_string()))?;
        }

        Ok(())
    }

    fn handle_audio_message(
        &mut self,
        msg: crate::Message,
        audio_rx: &mut Option<crate::MessageReceiver>,
    ) {
        match msg {
            crate::Message::Media(crate::MediaSample::Audio(sample)) => {
                self.input_audio_queue.push_back(sample);
            }
            crate::Message::Eos => {
                self.input_audio_stream_id = None;
                *audio_rx = None;
            }
            _ => {}
        }
    }

    fn handle_video_message(
        &mut self,
        msg: crate::Message,
        video_rx: &mut Option<crate::MessageReceiver>,
    ) {
        match msg {
            crate::Message::Media(crate::MediaSample::Video(sample)) => {
                self.input_video_queue.push_back(sample);
            }
            crate::Message::Eos => {
                self.input_video_stream_id = None;
                *video_rx = None;
            }
            _ => {}
        }
    }
}
