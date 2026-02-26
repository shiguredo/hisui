// Sora の録画ファイル合成処理固有モジュール（sora_recording_ がつかないモジュールからこのモジュールは参照しないこと）
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};

use crate::{
    Error, MediaFrame, Message, ProcessorHandle, Result, TrackId,
    sora_recording_layout::{Layout, Resolution, TrimSpans},
    sora_recording_layout_region::Region,
    sora_recording_metadata::SourceId,
    types::{EvenUsize, PixelPosition},
    video::{FrameRate, VideoFormat, VideoFrame},
};

// 入力がこの期間更新されなかった場合には、以後は合成ではなく
// 一つ前の合成フレームの尺を伸ばすことで対処する
// (入力のバグによって、非現実的な数のフレームが合成されるのを防ぐため）
//
// 値は適当なので必要に応じて調整すること
const TIMESTAMP_GAP_THRESHOLD: Duration = Duration::from_secs(60);

// 入力がこの期間更新されなかった場合には、尺調整ではなくエラーとする（最終的な安全弁）
//
// 値は適当なので必要に応じて調整すること
const TIMESTAMP_GAP_ERROR_THRESHOLD: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug)]
struct ResizeCachedVideoFrame {
    original: Arc<VideoFrame>,
    duration: Duration,
    resized: Vec<((EvenUsize, EvenUsize), VideoFrame)>, // (width, height) => resized frame
    resize_filter_mode: shiguredo_libyuv::FilterMode,
}

impl ResizeCachedVideoFrame {
    fn new(
        original: Arc<VideoFrame>,
        duration: Duration,
        resize_filter_mode: shiguredo_libyuv::FilterMode,
    ) -> Self {
        Self {
            original,
            duration,
            resized: Vec::new(),
            resize_filter_mode,
        }
    }

    fn start_timestamp(&self) -> Duration {
        self.original.timestamp
    }

    fn end_timestamp(&self) -> Duration {
        self.original.timestamp.saturating_add(self.duration)
    }

    fn duration(&self) -> Duration {
        self.duration
    }

    fn set_duration(&mut self, duration: Duration) {
        self.duration = duration;
    }

    fn resize(&mut self, width: EvenUsize, height: EvenUsize) -> crate::Result<&VideoFrame> {
        if self.original.width == width.get() && self.original.height == height.get() {
            // リサイズ不要
            return Ok(&self.original);
        }

        // [NOTE]
        // resized の要素数は、通常は 1 で多くても 2~3 である想定なので線形探索で十分
        if !self.resized.iter().any(|x| x.0 == (width, height)) {
            // キャッシュにないので新規リサイズが必要
            if let Some(resized) = self
                .original
                .resize(width, height, self.resize_filter_mode)?
            {
                self.resized.push(((width, height), resized));
            }
        }

        // キャッシュから対応するサイズのフレームを取得する
        let cached = self
            .resized
            .iter()
            .find_map(|x| (x.0 == (width, height)).then_some(&x.1))
            .expect("infallible");
        Ok(cached)
    }
}

#[derive(Debug)]
struct Canvas {
    width: EvenUsize,
    height: EvenUsize,
    data: Vec<u8>,
}

impl Canvas {
    fn new(width: EvenUsize, height: EvenUsize) -> Self {
        Self {
            width,
            height,
            data: VideoFrame::black(width, height).data,
        }
    }

    fn draw_frame(&mut self, position: PixelPosition, frame: &VideoFrame) -> crate::Result<()> {
        if frame.format != VideoFormat::I420 {
            return Err(crate::Error::new(format!(
                "expected I420 format, got {:?}",
                frame.format
            )));
        }
        if frame.width > self.width.get() {
            return Err(crate::Error::new(format!(
                "frame width {} exceeds canvas width {}",
                frame.width,
                self.width.get()
            )));
        }
        if frame.height > self.height.get() {
            return Err(crate::Error::new(format!(
                "frame height {} exceeds canvas height {}",
                frame.height,
                self.height.get()
            )));
        }

        // セルの解像度は偶数前提なので、奇数になることはない
        // (入力が奇数の場合でもリサイズによって常に偶数解像度になる）
        if !frame.width.is_multiple_of(2) {
            return Err(crate::Error::new(format!(
                "frame width must be even, got {}",
                frame.width
            )));
        }
        if !frame.height.is_multiple_of(2) {
            return Err(crate::Error::new(format!(
                "frame height must be even, got {}",
                frame.height
            )));
        }

        // Y成分の描画
        let offset_x = position.x.get();
        let offset_y = position.y.get();
        let y_size = frame.width * frame.height;
        let y_data = &frame.data[..y_size];
        for y in 0..frame.height {
            let i = y * frame.width;
            self.draw_y_line(offset_x, offset_y + y, &y_data[i..][..frame.width]);
        }

        // U成分の描画
        let offset_x = position.x.get() / 2;
        let offset_y = position.y.get() / 2;
        let u_size = (frame.width / 2) * (frame.height / 2);
        let u_data = &frame.data[y_size..][..u_size];
        for y in 0..frame.height / 2 {
            let i = y * (frame.width / 2);
            self.draw_u_line(offset_x, offset_y + y, &u_data[i..][..frame.width / 2]);
        }

        // V成分の描画
        let v_data = &frame.data[y_size + u_size..][..u_size];
        for y in 0..frame.height / 2 {
            let i = y * (frame.width / 2);
            self.draw_v_line(offset_x, offset_y + y, &v_data[i..][..frame.width / 2]);
        }

        Ok(())
    }

    fn draw_y_line(&mut self, x: usize, y: usize, line: &[u8]) {
        let i = x + y * self.width.get();
        self.data[i..][..line.len()].copy_from_slice(line);
    }

    fn draw_u_line(&mut self, x: usize, y: usize, line: &[u8]) {
        let y_size = self.width.get() * self.height.get();
        let i = x + y * (self.width.get() / 2);
        let offset = y_size + i;
        self.data[offset..][..line.len()].copy_from_slice(line);
    }

    fn draw_v_line(&mut self, x: usize, y: usize, line: &[u8]) {
        let y_size = self.width.get() * self.height.get();
        let u_size = (self.width.get() / 2) * (self.height.get() / 2);
        let i = x + y * (self.width.get() / 2);
        let offset = y_size + u_size + i;
        self.data[offset..][..line.len()].copy_from_slice(line);
    }
}

#[derive(Debug, Clone)]
pub struct VideoMixerSpec {
    pub regions: Vec<Region>,
    pub frame_rate: FrameRate,
    pub resolution: Resolution,
    pub trim_spans: TrimSpans,
    pub resize_filter_mode: shiguredo_libyuv::FilterMode,
    pub input_track_source_ids: HashMap<TrackId, SourceId>,
    pub source_stop_timestamps: HashMap<SourceId, Duration>,
}

impl VideoMixerSpec {
    pub fn from_layout(layout: &Layout) -> Self {
        Self {
            regions: layout.video_regions.clone(),
            frame_rate: layout.frame_rate,
            resolution: layout.resolution,
            trim_spans: layout.trim_spans.clone(),
            resize_filter_mode: shiguredo_libyuv::FilterMode::Box,
            input_track_source_ids: HashMap::new(),
            source_stop_timestamps: layout
                .sources
                .iter()
                .map(|(source_id, source)| (source_id.clone(), source.stop_timestamp))
                .collect(),
        }
    }

    pub fn with_input_track_source_ids(
        mut self,
        input_track_source_ids: HashMap<TrackId, SourceId>,
    ) -> Self {
        self.input_track_source_ids = input_track_source_ids;
        self
    }
}

#[derive(Debug)]
pub struct VideoMixer {
    spec: VideoMixerSpec,
    input_track_ids: Vec<TrackId>,
    input_streams: HashMap<TrackId, InputStream>,
    output_track_id: TrackId,
    last_mixed_frame: Option<VideoFrame>,
    stats: VideoMixerStats,
}

#[derive(Debug)]
pub struct VideoMixerInput {
    pub track_id: TrackId,
    pub sample: Option<MediaFrame>,
}

impl VideoMixerInput {
    pub fn video_frame(track_id: TrackId, frame: VideoFrame) -> Self {
        Self {
            track_id,
            sample: Some(MediaFrame::video(frame)),
        }
    }

    pub fn eos(track_id: TrackId) -> Self {
        Self {
            track_id,
            sample: None,
        }
    }
}

#[derive(Debug)]
pub enum VideoMixerOutput {
    Processed {
        track_id: TrackId,
        sample: MediaFrame,
    },
    Pending {
        awaiting_track_id: Option<TrackId>,
    },
    Finished,
}

impl VideoMixerOutput {
    pub fn expect_processed(self) -> Option<(TrackId, MediaFrame)> {
        if let Self::Processed { track_id, sample } = self {
            Some((track_id, sample))
        } else {
            None
        }
    }
}

#[derive(Debug)]
enum VideoMixerRunOutput {
    Processed(MediaFrame),
    Pending(TrackId),
    Finished,
}

#[derive(Debug)]
pub struct VideoMixerStats {
    total_input_video_frame_count: crate::stats::StatsCounter,
    total_output_video_frame_count: crate::stats::StatsCounter,
    total_output_video_frame_duration: crate::stats::StatsDuration,
    total_trimmed_video_frame_count: crate::stats::StatsCounter,
    total_extended_video_frame_count: crate::stats::StatsCounter,
}

impl VideoMixerStats {
    fn new(stats: &mut crate::stats::Stats) -> Self {
        let total_input_video_frame_count = stats.counter("total_input_video_frame_count");
        let total_output_video_frame_count = stats.counter("total_output_video_frame_count");
        let total_output_video_frame_duration = stats.duration("total_output_video_frame_seconds");
        let total_trimmed_video_frame_count = stats.counter("total_trimmed_video_frame_count");
        let total_extended_video_frame_count = stats.counter("total_extended_video_frame_count");
        stats.flag("error").set(false);
        Self {
            total_input_video_frame_count,
            total_output_video_frame_count,
            total_output_video_frame_duration,
            total_trimmed_video_frame_count,
            total_extended_video_frame_count,
        }
    }

    fn add_input_video_frame_count(&self) {
        self.total_input_video_frame_count.inc();
    }

    fn add_output_video_frame_count(&self) {
        self.total_output_video_frame_count.inc();
    }

    fn add_output_video_frame_duration(&self, duration: Duration) {
        self.total_output_video_frame_duration.add(duration);
    }

    fn add_trimmed_video_frame_count(&self) {
        self.total_trimmed_video_frame_count.inc();
    }

    fn add_extended_video_frame_count(&self) {
        self.total_extended_video_frame_count.inc();
    }

    pub fn total_input_video_frame_count(&self) -> u64 {
        self.total_input_video_frame_count.get()
    }

    pub fn total_output_video_frame_count(&self) -> u64 {
        self.total_output_video_frame_count.get()
    }

    pub fn total_output_video_frame_duration(&self) -> Duration {
        self.total_output_video_frame_duration.get()
    }

    pub fn total_trimmed_video_frame_count(&self) -> u64 {
        self.total_trimmed_video_frame_count.get()
    }

    pub fn total_extended_video_frame_count(&self) -> u64 {
        self.total_extended_video_frame_count.get()
    }
}

impl VideoMixer {
    pub fn new(
        spec: VideoMixerSpec,
        input_track_ids: Vec<TrackId>,
        output_track_id: TrackId,
        mut stats: crate::stats::Stats,
    ) -> Self {
        let resolution = spec.resolution;
        let input_streams = input_track_ids
            .iter()
            .cloned()
            .map(|id| {
                let source_id = spec
                    .input_track_source_ids
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| SourceId::new(id.get()));
                let stop_timestamp = spec.source_stop_timestamps.get(&source_id).copied();
                (id, InputStream::new(source_id, stop_timestamp))
            })
            .collect();
        stats
            .gauge("output_video_width")
            .set(resolution.width().get() as i64);
        stats
            .gauge("output_video_height")
            .set(resolution.height().get() as i64);
        let stats = VideoMixerStats::new(&mut stats);
        Self {
            spec,
            input_track_ids,
            input_streams,
            output_track_id,
            last_mixed_frame: None,
            stats,
        }
    }

    pub fn stats(&self) -> &VideoMixerStats {
        &self.stats
    }

    pub async fn run(
        mut self,
        handle: ProcessorHandle,
        input_track_ids: Vec<TrackId>,
        output_track_id: TrackId,
    ) -> Result<()> {
        if input_track_ids.len() != self.input_track_ids.len() {
            return Err(Error::new(format!(
                "input track count mismatch: expected {}, got {}",
                self.input_track_ids.len(),
                input_track_ids.len()
            )));
        }

        let mut input_tracks = self
            .input_track_ids
            .iter()
            .cloned()
            .zip(input_track_ids.into_iter())
            .map(|(expected_track_id, subscribed_track_id)| {
                (
                    expected_track_id,
                    InputTrack {
                        rx: handle.subscribe_track(subscribed_track_id),
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        let mut output_tx = handle.publish_track(output_track_id).await?;
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        loop {
            match self.poll_output()? {
                VideoMixerRunOutput::Processed(sample) => {
                    if !output_tx.send_media(sample) {
                        break;
                    }
                }
                VideoMixerRunOutput::Pending(track_id) => {
                    let input_track = input_tracks.get_mut(&track_id).ok_or_else(|| {
                        Error::new(format!(
                            "video mixer is waiting for unknown track id: {}",
                            track_id
                        ))
                    })?;
                    let message = input_track.rx.recv().await;
                    self.handle_input_message(&track_id, message)?;
                }
                VideoMixerRunOutput::Finished => {
                    output_tx.send_eos();
                    break;
                }
            }
        }

        Ok(())
    }

    fn next_input_timestamp(&self) -> Duration {
        self.frames_to_timestamp(
            self.stats.total_output_video_frame_count()
                + self.stats.total_extended_video_frame_count()
                + self.stats.total_trimmed_video_frame_count(),
        )
    }

    // フレーム数に対応するタイムスタンプを求める
    fn frames_to_timestamp(&self, frames: u64) -> Duration {
        Duration::from_secs(frames * self.spec.frame_rate.denumerator.get() as u64)
            / self.spec.frame_rate.numerator.get() as u32
    }

    fn next_output_timestamp(&self) -> Duration {
        self.frames_to_timestamp(
            self.stats.total_output_video_frame_count()
                + self.stats.total_extended_video_frame_count(),
        )
    }

    fn next_output_duration(&self) -> Duration {
        // 丸め誤差が蓄積しないように次のフレームのタイスタンプとの差をとる
        self.frames_to_timestamp(
            self.stats.total_output_video_frame_count()
                + self.stats.total_extended_video_frame_count()
                + 1,
        ) - self.next_output_timestamp()
    }

    fn mix(&mut self, now: Duration) -> crate::Result<VideoFrame> {
        let timestamp = self.next_output_timestamp();
        let duration = self.next_output_duration();

        let mut canvas = Canvas::new(self.spec.resolution.width(), self.spec.resolution.height());

        for region in &self.spec.regions {
            Self::mix_region(&mut canvas, region, &mut self.input_streams, now)?;
        }

        self.stats.add_output_video_frame_count();
        self.stats.add_output_video_frame_duration(duration);

        Ok(VideoFrame {
            // 固定値
            sample_entry: None, // 生データにはエンプルエントリーは存在しない
            keyframe: true,     // 生データはすべてキーフレーム扱い
            format: VideoFormat::I420,

            // 可変値
            timestamp,
            width: self.spec.resolution.width().get(),
            height: self.spec.resolution.height().get(),
            data: canvas.data,
        })
    }

    fn mix_region(
        canvas: &mut Canvas,
        region: &Region,
        input_streams: &mut HashMap<TrackId, InputStream>,
        now: Duration,
    ) -> crate::Result<()> {
        // [NOTE] ここで実質的にやりたいのは外枠を引くことだけなので、リージョン全体を塗りつぶすのは少し過剰
        //        (必要に応じて最適化する)
        let background_frame =
            VideoFrame::mono_color(region.background_color, region.width, region.height);
        canvas.draw_frame(region.position, &background_frame)?;

        let mut frames = Vec::new();
        for input_stream in input_streams.values_mut() {
            let Some(frame) = input_stream.frame_queue.front_mut() else {
                continue;
            };
            if now < frame.start_timestamp() {
                continue;
            }
            let Some(source) = region.grid.assigned_sources.get(&input_stream.source_id) else {
                // このグリッドには含まれないソースなので飛ばす
                continue;
            };
            frames.push((source, frame));
        }

        // 同じセルに割りあてられたソースが複数ある場合には
        // 一番優先度が高いものを採用する
        frames.sort_by_key(|(s, _)| (s.cell_index, s.priority));
        frames.dedup_by_key(|(s, _)| s.cell_index);

        for (source, frame) in frames {
            let mut position = region.cell_position(source.cell_index);
            let (frame_width, frame_height) = region.decide_frame_size(&frame.original);
            position.x +=
                EvenUsize::truncating_new((region.grid.cell_width - frame_width).get() / 2);
            position.y +=
                EvenUsize::truncating_new((region.grid.cell_height - frame_height).get() / 2);
            let resized_frame = frame.resize(frame_width, frame_height)?;
            canvas.draw_frame(position, resized_frame)?;
        }

        Ok(())
    }

    fn gap_until_next_frame_change(&self, now: Duration) -> Duration {
        let mut next_start_timestamp = Duration::MAX;
        for input_stream in self.input_streams.values() {
            let Some((current, next)) = input_stream
                .frame_queue
                .front()
                .zip(input_stream.frame_queue.get(1))
            else {
                continue;
            };
            if now < current.start_timestamp() {
                continue;
            }
            next_start_timestamp = next_start_timestamp.min(next.start_timestamp());
        }
        if next_start_timestamp == Duration::MAX {
            Duration::ZERO
        } else {
            next_start_timestamp.saturating_sub(now)
        }
    }

    fn handle_input_message(&mut self, track_id: &TrackId, message: Message) -> Result<()> {
        match message {
            Message::Media(MediaFrame::Video(sample)) => {
                self.handle_input_sample(track_id, Some(MediaFrame::Video(sample)))
            }
            Message::Media(MediaFrame::Audio(_)) => Err(Error::new(format!(
                "expected a video sample on track {}, but got an audio sample",
                track_id.get()
            ))),
            Message::Eos => self.handle_input_sample(track_id, None),
            Message::Syn(_) => Ok(()),
        }
    }

    fn handle_input_sample(
        &mut self,
        track_id: &TrackId,
        sample: Option<MediaFrame>,
    ) -> Result<()> {
        if let Some(sample) = sample {
            // キューに要素を追加する
            let frame = sample.expect_video()?;
            if frame.format != VideoFormat::I420 {
                return Err(crate::Error::new(format!(
                    "expected I420 format, got {:?}",
                    frame.format
                )));
            }

            // 新規フレームの初期 duration は、比較対象がない場合のフォールバック値として
            // 出力フレーム間隔 (next_output_duration) を使う。
            // その後、次フレーム到着時には timestamp 差分で上書きされ、
            // 最終フレームは EOS 時に stop_timestamp で補正されることがある。
            let mut duration = self.next_output_duration();
            let input_stream = self.input_streams.get_mut(track_id).ok_or_else(|| {
                crate::Error::new(format!(
                    "unknown input track id for video mixer: {}",
                    track_id
                ))
            })?;
            if let Some(prev) = input_stream.frame_queue.back_mut() {
                let computed = frame.timestamp.saturating_sub(prev.start_timestamp());
                if !computed.is_zero() {
                    prev.set_duration(computed);
                    duration = computed;
                } else {
                    duration = prev.duration();
                }
            }

            input_stream
                .frame_queue
                .push_back(ResizeCachedVideoFrame::new(
                    frame,
                    duration,
                    self.spec.resize_filter_mode,
                ));
            self.stats.add_input_video_frame_count();
        } else {
            let input_stream = self.input_streams.get_mut(track_id).ok_or_else(|| {
                crate::Error::new(format!(
                    "unknown input track id for video mixer: {}",
                    track_id
                ))
            })?;
            if let (Some(frame), Some(stop_timestamp)) = (
                input_stream.frame_queue.back_mut(),
                input_stream.stop_timestamp,
            ) {
                let duration = stop_timestamp.saturating_sub(frame.start_timestamp());
                if !duration.is_zero() {
                    frame.set_duration(duration);
                }
            }
            input_stream.eos = true;
        }
        Ok(())
    }

    fn poll_output(&mut self) -> Result<VideoMixerRunOutput> {
        loop {
            let mut now = self.next_input_timestamp();

            // トリム対象期間ならその分はスキップする
            while self.spec.trim_spans.contains(now) {
                self.stats.add_trimmed_video_frame_count();
                now = self.next_input_timestamp();
            }

            // 表示対象のフレームを更新する
            for (input_track_id, input_stream) in &mut self.input_streams {
                if !input_stream.pop_outdated_frame(now) {
                    // 十分な数のフレームが溜まっていないので入力を待つ
                    return Ok(VideoMixerRunOutput::Pending(input_track_id.clone()));
                }
            }

            // EOS 判定
            if self
                .input_streams
                .values()
                .all(|s| s.eos && s.frame_queue.is_empty())
            {
                if let Some(frame) = self.last_mixed_frame.take() {
                    // バッファにフレームが残っていたらそれを返す
                    return Ok(VideoMixerRunOutput::Processed(MediaFrame::video(frame)));
                }
                return Ok(VideoMixerRunOutput::Finished);
            }

            // 入力のタイムスタンプに極端なギャップがある場合の対応
            let gap = self.gap_until_next_frame_change(now);
            if gap > TIMESTAMP_GAP_THRESHOLD {
                if gap > TIMESTAMP_GAP_ERROR_THRESHOLD {
                    return Err(crate::Error::new("too large timestamp gap"));
                }

                // 一定期間、入力の更新がない場合には、合成ではなく一つ前のフレームの尺を調整することで対応する
                if self.last_mixed_frame.is_none() {
                    // 先頭フレームがまだない場合には、まず一度だけ通常合成を行って基準フレームを作る。
                    // (このとき output 系の統計値も mix() 内で更新される)
                    self.last_mixed_frame = Some(self.mix(now)?);
                    continue;
                }
                let duration = self.next_output_duration();
                self.stats.add_extended_video_frame_count();
                // 出力フレーム数は増えないが、出力尺は伸びる。
                // VideoFrame には duration フィールドがないため、尺の延長は統計値と次フレーム timestamp 差分で表現する。
                self.stats.add_output_video_frame_duration(duration);

                continue;
            }

            // 現在のフレームを合成する
            let mixed_frame = self.mix(now)?;

            if let Some(frame) = self.last_mixed_frame.replace(mixed_frame) {
                return Ok(VideoMixerRunOutput::Processed(MediaFrame::video(frame)));
            }
        }
    }

    pub fn push_input(&mut self, track_id: TrackId, sample: Option<MediaFrame>) -> Result<()> {
        self.handle_input_sample(&track_id, sample)
    }

    pub fn process_input(&mut self, input: VideoMixerInput) -> Result<()> {
        self.push_input(input.track_id, input.sample)
    }

    pub fn next_output(&mut self) -> Result<VideoMixerOutput> {
        match self.poll_output()? {
            VideoMixerRunOutput::Processed(sample) => Ok(VideoMixerOutput::Processed {
                track_id: self.output_track_id.clone(),
                sample,
            }),
            VideoMixerRunOutput::Pending(track_id) => Ok(VideoMixerOutput::Pending {
                awaiting_track_id: Some(track_id),
            }),
            VideoMixerRunOutput::Finished => Ok(VideoMixerOutput::Finished),
        }
    }

    pub fn process_output(&mut self) -> Result<VideoMixerOutput> {
        self.next_output()
    }
}

#[derive(Debug)]
struct InputTrack {
    rx: crate::MessageReceiver,
}

#[derive(Debug)]
struct InputStream {
    source_id: SourceId,
    stop_timestamp: Option<Duration>,
    eos: bool,
    frame_queue: VecDeque<ResizeCachedVideoFrame>,
}

impl InputStream {
    fn new(source_id: SourceId, stop_timestamp: Option<Duration>) -> Self {
        Self {
            source_id,
            stop_timestamp,
            eos: false,
            frame_queue: VecDeque::new(),
        }
    }

    fn pop_outdated_frame(&mut self, now: Duration) -> bool {
        loop {
            let Some(current_frame) = self.frame_queue.front() else {
                if self.eos {
                    // EOS & キューにフレームが残っていない
                    return true;
                } else {
                    // EOS ではないけどキューが空なので入力待ち
                    return false;
                }
            };
            if now < current_frame.end_timestamp() {
                // まだ現在のフレームの表示時刻範囲内
                return true;
            }

            let Some(next_frame) = self.frame_queue.get(1) else {
                if self.eos {
                    // EOS に到達し、表示時刻も超過した
                    self.frame_queue.clear();
                    return true;
                } else {
                    // EOS ではないなら、現在のフレームの破棄タイミングを判断するために次の入力を待つ
                    return false;
                }
            };
            if now < next_frame.start_timestamp() {
                // まだ現在のフレームの表示時刻範囲内
                //
                // [NOTE]
                // 前のフレームの表示時刻を過ぎていても、
                // 次のフレームの表示時刻に到達するまでは前のフレームを使い続ける
                //
                // これは、入力ファイル作成時の数値計算誤差などで、
                // 僅かなギャップが生じたとしても、その部分を黒塗りにしたくはないため
                return true;
            }

            // 次のフレームの表示時刻になった
            //
            // なお、入力 FPS よりも出力 FPS の方が小さい場合には、
            // 次のフレームがすぐに破棄される可能性もあるので、
            // ここで終わりにしないで、ループしている
            self.frame_queue.pop_front();
        }
    }
}
