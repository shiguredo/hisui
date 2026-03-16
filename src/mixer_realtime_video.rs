use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};

use crate::{
    Error, MediaFrame, Message, ProcessorHandle, TrackId,
    types::EvenUsize,
    video::{FrameRate, RawVideoFrame, VideoFormat, VideoFrame, VideoFrameSize},
};

const MAX_NOACKED_COUNT: u64 = 100;

#[derive(Debug)]
pub struct VideoRealtimeMixer {
    pub canvas_width: EvenUsize,
    pub canvas_height: EvenUsize,
    pub frame_rate: FrameRate,
    pub input_tracks: Vec<InputTrack>,
    pub output_track_id: TrackId,
}

impl nojson::DisplayJson for VideoRealtimeMixer {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("canvasWidth", self.canvas_width)?;
            f.member("canvasHeight", self.canvas_height)?;
            f.member("frameRate", self.frame_rate)?;
            f.member("inputTracks", &self.input_tracks)?;
            f.member("outputTrackId", &self.output_track_id)
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for VideoRealtimeMixer {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let canvas_width = value.to_member("canvasWidth")?.required()?.try_into()?;
        let canvas_height = value.to_member("canvasHeight")?.required()?.try_into()?;
        let frame_rate: Option<FrameRate> = value.to_member("frameRate")?.try_into()?;
        let input_tracks: Vec<InputTrack> =
            value.to_member("inputTracks")?.required()?.try_into()?;
        validate_unique_input_tracks_for_json(&input_tracks, value)?;
        let output_track_id = value.to_member("outputTrackId")?.required()?.try_into()?;
        Ok(Self {
            canvas_width,
            canvas_height,
            frame_rate: frame_rate.unwrap_or(FrameRate::FPS_30),
            input_tracks,
            output_track_id,
        })
    }
}

impl VideoRealtimeMixer {
    pub async fn run(self, handle: ProcessorHandle) -> crate::Result<()> {
        let Self {
            canvas_width,
            canvas_height,
            frame_rate,
            input_tracks,
            output_track_id,
        } = self;

        let mut stats = handle.stats();
        let stats = VideoRealtimeMixerStats::new(&mut stats);
        let output_tx = handle.publish_track(output_track_id).await?;
        let draw_order = build_draw_order(&input_tracks);
        let mut states = HashMap::with_capacity(input_tracks.len());
        for input_track in &input_tracks {
            let state = InputTrackState::new(input_track.clone())?;
            states.insert(input_track.track_id.clone(), state);
        }

        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (rpc_tx, rpc_rx) = tokio::sync::mpsc::unbounded_channel();
        handle
            .register_rpc_sender(rpc_tx)
            .await
            .map_err(|e| Error::new(format!("failed to register video mixer RPC sender: {}", e)))?;
        let mixer_start = tokio::time::Instant::now();
        let mut input_receivers = HashMap::with_capacity(input_tracks.len());
        for track in &draw_order {
            let input_rx = handle.subscribe_track(track.track_id.clone());
            let receiver = spawn_input_receiver(
                track.track_id.clone(),
                input_rx,
                event_tx.clone(),
                mixer_start,
            );
            input_receivers.insert(receiver.track_id.clone(), receiver);
        }
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;
        stats.set_runtime_config(
            canvas_width.get(),
            canvas_height.get(),
            frame_rate,
            input_tracks.len(),
        );

        let mut output_tx = output_tx;
        let ack = Some(output_tx.send_syn());
        VideoRealtimeMixerRunner {
            processor_handle: handle,
            canvas_width: canvas_width.get(),
            canvas_height: canvas_height.get(),
            frame_rate,
            output_tx,
            input_tracks,
            draw_order,
            states,
            input_receivers,
            track_event_tx: event_tx,
            event_rx: Some(event_rx),
            rpc_rx: Some(rpc_rx),
            mixer_start,
            output_frame_index: 0,
            noacked_sent: 0,
            ack,
            stats,
        }
        .run()
        .await
    }
}

#[derive(Debug, Clone)]
pub struct InputTrack {
    pub track_id: TrackId,
    pub x: isize,
    pub y: isize,
    pub z: isize,
    pub width: Option<EvenUsize>,
    pub height: Option<EvenUsize>,
    pub scale_x: Option<f64>,
    pub scale_y: Option<f64>,
    pub crop_top: usize,
    pub crop_bottom: usize,
    pub crop_left: usize,
    pub crop_right: usize,
}

impl nojson::DisplayJson for InputTrack {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("trackId", &self.track_id)?;
            f.member("x", self.x)?;
            f.member("y", self.y)?;
            f.member("z", self.z)?;
            if let Some(width) = self.width {
                f.member("width", width)?;
            }
            if let Some(height) = self.height {
                f.member("height", height)?;
            }
            if let Some(scale_x) = self.scale_x {
                f.member("scaleX", scale_x)?;
            }
            if let Some(scale_y) = self.scale_y {
                f.member("scaleY", scale_y)?;
            }
            if self.crop_top != 0 {
                f.member("cropTop", self.crop_top)?;
            }
            if self.crop_bottom != 0 {
                f.member("cropBottom", self.crop_bottom)?;
            }
            if self.crop_left != 0 {
                f.member("cropLeft", self.crop_left)?;
            }
            if self.crop_right != 0 {
                f.member("cropRight", self.crop_right)?;
            }
            Ok(())
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for InputTrack {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let track_id: TrackId = value.to_member("trackId")?.required()?.try_into()?;
        let x: Option<isize> = value.to_member("x")?.try_into()?;
        let y: Option<isize> = value.to_member("y")?.try_into()?;
        let z: Option<isize> = value.to_member("z")?.try_into()?;
        let width: Option<EvenUsize> = value.to_member("width")?.try_into()?;
        let height: Option<EvenUsize> = value.to_member("height")?.try_into()?;
        let scale_x: Option<f64> = value.to_member("scaleX")?.try_into()?;
        let scale_y: Option<f64> = value.to_member("scaleY")?.try_into()?;
        let crop_top: Option<usize> = value.to_member("cropTop")?.try_into()?;
        let crop_bottom: Option<usize> = value.to_member("cropBottom")?.try_into()?;
        let crop_left: Option<usize> = value.to_member("cropLeft")?.try_into()?;
        let crop_right: Option<usize> = value.to_member("cropRight")?.try_into()?;

        Ok(Self {
            track_id,
            x: x.unwrap_or_default(),
            y: y.unwrap_or_default(),
            z: z.unwrap_or_default(),
            width,
            height,
            scale_x,
            scale_y,
            crop_top: crop_top.unwrap_or_default(),
            crop_bottom: crop_bottom.unwrap_or_default(),
            crop_left: crop_left.unwrap_or_default(),
            crop_right: crop_right.unwrap_or_default(),
        })
    }
}

#[derive(Debug)]
pub enum VideoRealtimeMixerRpcMessage {
    UpdateConfig {
        request: VideoRealtimeMixerUpdateConfigRequest,
        reply_tx: tokio::sync::oneshot::Sender<crate::Result<VideoRealtimeMixerUpdateConfigResult>>,
    },
}

#[derive(Debug, Clone)]
pub struct VideoRealtimeMixerUpdateConfigRequest {
    pub canvas_width: EvenUsize,
    pub canvas_height: EvenUsize,
    pub frame_rate: FrameRate,
    pub input_tracks: Vec<InputTrack>,
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>>
    for VideoRealtimeMixerUpdateConfigRequest
{
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let canvas_width = value.to_member("canvasWidth")?.required()?.try_into()?;
        let canvas_height = value.to_member("canvasHeight")?.required()?.try_into()?;
        let frame_rate = value.to_member("frameRate")?.required()?.try_into()?;
        let input_tracks: Vec<InputTrack> =
            value.to_member("inputTracks")?.required()?.try_into()?;
        validate_unique_input_tracks_for_json(&input_tracks, value)?;
        Ok(Self {
            canvas_width,
            canvas_height,
            frame_rate,
            input_tracks,
        })
    }
}

#[derive(Debug, Clone)]
pub struct VideoRealtimeMixerUpdateConfigResult {
    pub previous_canvas_width: usize,
    pub previous_canvas_height: usize,
    pub previous_frame_rate: FrameRate,
    pub previous_input_tracks: Vec<InputTrack>,
}

#[derive(Debug)]
struct DrawOrder {
    track_id: TrackId,
    z: isize,
    index: usize,
}

#[derive(Debug)]
struct VideoRealtimeMixerRunner {
    processor_handle: ProcessorHandle,
    canvas_width: usize,
    canvas_height: usize,
    frame_rate: FrameRate,
    output_tx: crate::MessageSender,
    input_tracks: Vec<InputTrack>,
    draw_order: Vec<DrawOrder>,
    states: HashMap<TrackId, InputTrackState>,
    input_receivers: HashMap<TrackId, InputReceiverHandle>,
    track_event_tx: tokio::sync::mpsc::UnboundedSender<TrackEvent>,
    event_rx: Option<tokio::sync::mpsc::UnboundedReceiver<TrackEvent>>,
    rpc_rx: Option<tokio::sync::mpsc::UnboundedReceiver<VideoRealtimeMixerRpcMessage>>,
    mixer_start: tokio::time::Instant,
    output_frame_index: u64,
    noacked_sent: u64,
    ack: Option<crate::Ack>,
    stats: VideoRealtimeMixerStats,
}

#[derive(Debug)]
struct VideoRealtimeMixerStats {
    current_input_track_count: crate::stats::StatsGauge,
    current_canvas_width: crate::stats::StatsGauge,
    current_canvas_height: crate::stats::StatsGauge,
    current_frame_rate_numerator: crate::stats::StatsGauge,
    current_frame_rate_denumerator: crate::stats::StatsGauge,
}

impl VideoRealtimeMixerStats {
    fn new(stats: &mut crate::stats::Stats) -> Self {
        Self {
            current_input_track_count: stats.gauge("current_input_track_count"),
            current_canvas_width: stats.gauge("current_canvas_width"),
            current_canvas_height: stats.gauge("current_canvas_height"),
            current_frame_rate_numerator: stats.gauge("current_frame_rate_numerator"),
            current_frame_rate_denumerator: stats.gauge("current_frame_rate_denumerator"),
        }
    }

    fn set_runtime_config(
        &self,
        canvas_width: usize,
        canvas_height: usize,
        frame_rate: FrameRate,
        input_track_count: usize,
    ) {
        self.current_canvas_width.set(canvas_width as i64);
        self.current_canvas_height.set(canvas_height as i64);
        self.current_frame_rate_numerator
            .set(frame_rate.numerator.get() as i64);
        self.current_frame_rate_denumerator
            .set(frame_rate.denumerator.get() as i64);
        self.current_input_track_count.set(input_track_count as i64);
    }
}

impl VideoRealtimeMixerRunner {
    async fn run(mut self) -> crate::Result<()> {
        let mut event_rx = self.event_rx.take();
        let mut rpc_rx = self.rpc_rx.take();
        loop {
            let next_instant = self.next_output_instant();
            tokio::select! {
                _ = tokio::time::sleep_until(next_instant) => {
                    if !self.handle_output_tick().await? {
                        break;
                    }
                }
                event = recv_track_event_or_pending(&mut event_rx) => {
                    self.handle_event(event, &mut event_rx)?;
                }
                rpc_message = recv_rpc_message_or_pending(&mut rpc_rx) => {
                    self.handle_rpc_message(rpc_message, &mut rpc_rx)?;
                }
            }
        }
        Ok(())
    }

    fn next_output_instant(&mut self) -> tokio::time::Instant {
        let now = self.mixer_start.elapsed();
        self.output_frame_index =
            catch_up_output_frame_index(self.frame_rate, self.output_frame_index, now);
        let next_timestamp = frames_to_timestamp(self.frame_rate, self.output_frame_index);
        self.mixer_start + next_timestamp
    }

    async fn handle_output_tick(&mut self) -> crate::Result<bool> {
        let now = self.mixer_start.elapsed();
        for state in self.states.values_mut() {
            state.advance(now);
        }

        if self.noacked_sent > MAX_NOACKED_COUNT {
            if let Some(waiting_ack) = self.ack.take() {
                waiting_ack.await;
            }
            self.ack = Some(self.output_tx.send_syn());
            self.noacked_sent = 0;
        }

        let timestamp = frames_to_timestamp(self.frame_rate, self.output_frame_index);
        self.output_frame_index = self.output_frame_index.saturating_add(1);

        let frame = compose_frame(
            self.canvas_width,
            self.canvas_height,
            timestamp,
            &self.draw_order,
            &self.states,
        )?;

        if !self.output_tx.send_video(frame) {
            return Ok(false);
        }
        self.noacked_sent = self.noacked_sent.saturating_add(1);

        Ok(true)
    }

    fn handle_event(
        &mut self,
        event: Option<TrackEvent>,
        event_rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<TrackEvent>>,
    ) -> crate::Result<()> {
        let Some(event) = event else {
            *event_rx = None;
            return Ok(());
        };
        handle_track_event(event, &mut self.states)
    }

    fn handle_rpc_message(
        &mut self,
        rpc_message: Option<VideoRealtimeMixerRpcMessage>,
        rpc_rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<VideoRealtimeMixerRpcMessage>>,
    ) -> crate::Result<()> {
        let Some(rpc_message) = rpc_message else {
            *rpc_rx = None;
            return Ok(());
        };

        match rpc_message {
            VideoRealtimeMixerRpcMessage::UpdateConfig { request, reply_tx } => {
                let result = self.update_config(request);
                let _ = reply_tx.send(result);
            }
        }

        Ok(())
    }

    fn update_config(
        &mut self,
        request: VideoRealtimeMixerUpdateConfigRequest,
    ) -> crate::Result<VideoRealtimeMixerUpdateConfigResult> {
        let previous_canvas_width = self.canvas_width;
        let previous_canvas_height = self.canvas_height;
        let previous_frame_rate = self.frame_rate;
        let previous_input_tracks = self.input_tracks.clone();
        let requested_track_ids: HashSet<TrackId> = request
            .input_tracks
            .iter()
            .map(|input_track| input_track.track_id.clone())
            .collect();
        let removed_track_ids = self
            .input_tracks
            .iter()
            .map(|input_track| input_track.track_id.clone())
            .filter(|track_id| !requested_track_ids.contains(track_id))
            .collect::<Vec<_>>();

        for track_id in removed_track_ids {
            self.states.remove(&track_id);
            if let Some(receiver) = self.input_receivers.remove(&track_id) {
                receiver.shutdown();
            }
        }

        for input_track in &request.input_tracks {
            if let Some(state) = self.states.get_mut(&input_track.track_id) {
                state.update_input_track(input_track.clone());
                continue;
            }

            self.states.insert(
                input_track.track_id.clone(),
                InputTrackState::new(input_track.clone())?,
            );
            let input_rx = self
                .processor_handle
                .subscribe_track(input_track.track_id.clone());
            let receiver = spawn_input_receiver(
                input_track.track_id.clone(),
                input_rx,
                self.track_event_tx.clone(),
                self.mixer_start,
            );
            self.input_receivers
                .insert(receiver.track_id.clone(), receiver);
        }

        self.draw_order = build_draw_order(&request.input_tracks);
        self.input_tracks = request.input_tracks;
        self.canvas_width = request.canvas_width.get();
        self.canvas_height = request.canvas_height.get();
        self.frame_rate = request.frame_rate;
        self.stats.set_runtime_config(
            self.canvas_width,
            self.canvas_height,
            self.frame_rate,
            self.input_tracks.len(),
        );

        Ok(VideoRealtimeMixerUpdateConfigResult {
            previous_canvas_width,
            previous_canvas_height,
            previous_frame_rate,
            previous_input_tracks,
        })
    }
}

#[derive(Debug)]
struct PendingVideoFrame {
    timestamp: Duration,
    frame: RawVideoFrame,
}

#[derive(Debug)]
struct InputTrackState {
    input_track: InputTrack,
    target_width: Option<EvenUsize>,
    target_height: Option<EvenUsize>,
    scale_x: Option<f64>,
    scale_y: Option<f64>,
    first_input_sample_timestamp: Option<Duration>,
    first_input_elapsed: Option<Duration>,
    pending_frames: VecDeque<PendingVideoFrame>,
    current_frame: Option<PendingVideoFrame>,
    eos: bool,
}

impl InputTrackState {
    fn new(input_track: InputTrack) -> crate::Result<Self> {
        let target_width = input_track.width;
        let target_height = input_track.height;
        let scale_x = input_track.scale_x;
        let scale_y = input_track.scale_y;

        Ok(Self {
            input_track,
            target_width,
            target_height,
            scale_x,
            scale_y,
            first_input_sample_timestamp: None,
            first_input_elapsed: None,
            pending_frames: VecDeque::new(),
            current_frame: None,
            eos: false,
        })
    }

    fn handle_video(&mut self, frame: Arc<VideoFrame>, received_at: Duration) -> crate::Result<()> {
        let frame = RawVideoFrame::from_video_frame(frame)?;
        let video_frame = frame.as_video_frame();

        let first_sample_timestamp = *self
            .first_input_sample_timestamp
            .get_or_insert(video_frame.timestamp);
        let first_input_elapsed = *self.first_input_elapsed.get_or_insert(received_at);

        // TODO: 入力 source 側で実時刻補正を担保する方針に統一できたら、
        //       mixer 側の補正ロジックは削除する。
        let adjusted_timestamp = video_frame
            .timestamp
            .saturating_sub(first_sample_timestamp)
            .saturating_add(first_input_elapsed);

        self.pending_frames.push_back(PendingVideoFrame {
            timestamp: adjusted_timestamp,
            frame,
        });
        self.eos = false;

        Ok(())
    }

    fn handle_eos(&mut self) {
        // realtime 用途では低遅延を優先するため、EOS 到達時点で未表示フレームを破棄し、
        // 現在フレームも消して即座にレイアウトから除外する。
        self.eos = true;
        self.pending_frames.clear();
        self.current_frame = None;
    }

    fn update_input_track(&mut self, input_track: InputTrack) {
        self.target_width = input_track.width;
        self.target_height = input_track.height;
        self.scale_x = input_track.scale_x;
        self.scale_y = input_track.scale_y;
        self.input_track = input_track;
    }

    fn advance(&mut self, now: Duration) {
        while let Some(next) = self.pending_frames.front() {
            if next.timestamp <= now {
                self.current_frame = self.pending_frames.pop_front();
            } else {
                break;
            }
        }
    }
}

#[derive(Debug)]
enum TrackEvent {
    Video {
        track_id: TrackId,
        frame: Arc<VideoFrame>,
        received_at: Duration,
    },
    Syn(crate::Syn),
    Eos {
        track_id: TrackId,
    },
    Error {
        track_id: TrackId,
        reason: String,
    },
}

#[derive(Debug)]
struct InputReceiverHandle {
    track_id: TrackId,
    // 明示的な Drop 実装は持たない
    // shutdown_tx を drop しても受信側は完了として解除され、spawn タスクは終了する
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl InputReceiverHandle {
    fn shutdown(mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
    }
}

fn spawn_input_receiver(
    track_id: TrackId,
    mut input_rx: crate::MessageReceiver,
    event_tx: tokio::sync::mpsc::UnboundedSender<TrackEvent>,
    mixer_start: tokio::time::Instant,
) -> InputReceiverHandle {
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
    let task_track_id = track_id.clone();
    tokio::spawn(async move {
        loop {
            let message = tokio::select! {
                _ = &mut shutdown_rx => {
                    break;
                }
                message = input_rx.recv() => message,
            };

            match message {
                Message::Media(sample) => match sample {
                    MediaFrame::Video(frame) => {
                        let _ = event_tx.send(TrackEvent::Video {
                            track_id: track_id.clone(),
                            frame,
                            received_at: mixer_start.elapsed(),
                        });
                    }
                    MediaFrame::Audio(_) => {
                        let _ = event_tx.send(TrackEvent::Error {
                            track_id: track_id.clone(),
                            reason: "expected a video sample, but got an audio sample".to_owned(),
                        });
                        break;
                    }
                },
                Message::Eos => {
                    let _ = event_tx.send(TrackEvent::Eos {
                        track_id: track_id.clone(),
                    });
                    break;
                }
                Message::Syn(syn) => {
                    let _ = event_tx.send(TrackEvent::Syn(syn));
                }
            }
        }
    });

    InputReceiverHandle {
        track_id: task_track_id,
        shutdown_tx: Some(shutdown_tx),
    }
}

async fn recv_track_event_or_pending(
    event_rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<TrackEvent>>,
) -> Option<TrackEvent> {
    if let Some(event_rx) = event_rx {
        event_rx.recv().await
    } else {
        std::future::pending().await
    }
}

async fn recv_rpc_message_or_pending(
    rpc_rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<VideoRealtimeMixerRpcMessage>>,
) -> Option<VideoRealtimeMixerRpcMessage> {
    if let Some(rpc_rx) = rpc_rx {
        rpc_rx.recv().await
    } else {
        std::future::pending().await
    }
}

fn handle_track_event(
    event: TrackEvent,
    states: &mut HashMap<TrackId, InputTrackState>,
) -> crate::Result<()> {
    match event {
        TrackEvent::Video {
            track_id,
            frame,
            received_at,
        } => {
            let Some(state) = states.get_mut(&track_id) else {
                return Ok(());
            };
            state.handle_video(frame, received_at)?;
        }
        TrackEvent::Eos { track_id } => {
            if let Some(state) = states.get_mut(&track_id) {
                state.handle_eos();
            }
        }
        TrackEvent::Syn(_syn) => {}
        TrackEvent::Error { track_id, reason } => {
            if !states.contains_key(&track_id) {
                return Ok(());
            }
            return Err(Error::new(format!("input track {}: {}", track_id, reason)));
        }
    }

    Ok(())
}

/// ソースフレームにクロップを適用する。
/// クロップ値がすべて 0 の場合は `None` を返す（元フレームをそのまま使用）。
/// I420 のクロマサブサンプリング制約のため、crop_left / crop_top は偶数に切り下げる。
fn apply_crop(
    frame: &RawVideoFrame,
    input_track: &InputTrack,
) -> crate::Result<Option<RawVideoFrame>> {
    let crop_top = input_track.crop_top;
    let crop_bottom = input_track.crop_bottom;
    let crop_left = input_track.crop_left;
    let crop_right = input_track.crop_right;

    if crop_top == 0 && crop_bottom == 0 && crop_left == 0 && crop_right == 0 {
        return Ok(None);
    }

    let size = frame.size();

    // I420 のクロマサブサンプリング制約のため偶数に丸める
    let crop_left = crop_left & !1;
    let crop_top = crop_top & !1;
    let crop_right = crop_right & !1;
    let crop_bottom = crop_bottom & !1;

    let effective_width = size.width.saturating_sub(crop_left + crop_right);
    let effective_height = size.height.saturating_sub(crop_top + crop_bottom);

    // クロップ後のサイズが不正な場合はスキップ
    if effective_width < 2 || effective_height < 2 {
        return Ok(None);
    }

    // 偶数に切り下げ
    let effective_width = effective_width & !1;
    let effective_height = effective_height & !1;

    let video_frame = frame.as_video_frame();
    let (src_y, src_u, src_v, src_a) = match video_frame.format {
        VideoFormat::I420 => {
            let (y, u, v) = video_frame
                .as_yuv_planes()
                .ok_or_else(|| Error::new("invalid I420 frame size"))?;
            (y, u, v, None)
        }
        VideoFormat::I420A => {
            let (y, u, v, a) = video_frame
                .as_i420a_planes()
                .ok_or_else(|| Error::new("invalid I420A frame size"))?;
            (y, u, v, Some(a))
        }
        _ => {
            return Err(Error::new(format!(
                "unsupported video format for crop: expected I420 or I420A, got {}",
                video_frame.format
            )));
        }
    };

    let src_width = size.width;
    let src_uv_width = src_width.div_ceil(2);

    // Y プレーンのクロップ
    let mut y_plane = Vec::with_capacity(effective_width * effective_height);
    for row in 0..effective_height {
        let src_row = crop_top + row;
        let src_offset = src_row * src_width + crop_left;
        y_plane.extend_from_slice(&src_y[src_offset..src_offset + effective_width]);
    }

    // U/V プレーンのクロップ
    let eff_uv_width = effective_width.div_ceil(2);
    let eff_uv_height = effective_height.div_ceil(2);
    let crop_uv_left = crop_left / 2;
    let crop_uv_top = crop_top / 2;

    let mut u_plane = Vec::with_capacity(eff_uv_width * eff_uv_height);
    let mut v_plane = Vec::with_capacity(eff_uv_width * eff_uv_height);
    for row in 0..eff_uv_height {
        let src_row = crop_uv_top + row;
        let src_offset = src_row * src_uv_width + crop_uv_left;
        u_plane.extend_from_slice(&src_u[src_offset..src_offset + eff_uv_width]);
        v_plane.extend_from_slice(&src_v[src_offset..src_offset + eff_uv_width]);
    }

    // A プレーンのクロップ（I420A の場合）
    let mut a_plane = if let Some(src_a) = src_a {
        let mut a = Vec::with_capacity(effective_width * effective_height);
        for row in 0..effective_height {
            let src_row = crop_top + row;
            let src_offset = src_row * src_width + crop_left;
            a.extend_from_slice(&src_a[src_offset..src_offset + effective_width]);
        }
        Some(a)
    } else {
        None
    };

    let format = video_frame.format;
    let mut data = Vec::with_capacity(
        y_plane.len() + u_plane.len() + v_plane.len() + a_plane.as_ref().map_or(0, |a| a.len()),
    );
    data.append(&mut y_plane);
    data.append(&mut u_plane);
    data.append(&mut v_plane);
    if let Some(ref mut a) = a_plane {
        data.append(a);
    }

    let cropped_frame = VideoFrame {
        sample_entry: None,
        keyframe: true,
        format,
        timestamp: video_frame.timestamp,
        size: Some(VideoFrameSize {
            width: effective_width,
            height: effective_height,
        }),
        data,
    };

    Ok(Some(RawVideoFrame::from_video_frame(Arc::new(
        cropped_frame,
    ))?))
}

/// ソースフレームサイズにスケール係数を乗算して偶数に丸める
fn scale_to_even(source_size: usize, scale: f64) -> usize {
    let scaled = (source_size as f64 * scale).round() as usize;
    // 偶数に切り上げ
    if scaled.is_multiple_of(2) {
        scaled
    } else {
        scaled + 1
    }
}

fn compose_frame(
    canvas_width: usize,
    canvas_height: usize,
    timestamp: Duration,
    draw_order: &[DrawOrder],
    states: &HashMap<TrackId, InputTrackState>,
) -> crate::Result<VideoFrame> {
    let mut canvas = RealtimeI420Canvas::new(canvas_width, canvas_height);

    for draw in draw_order {
        let Some(state) = states.get(&draw.track_id) else {
            continue;
        };
        let Some(current) = state.current_frame.as_ref() else {
            continue;
        };

        let x = state.input_track.x;
        let y = state.input_track.y;

        // クロップ適用: ソースフレームからクロップ領域を切り出す
        let source_frame = apply_crop(&current.frame, &state.input_track)?;
        let source_frame_ref = source_frame.as_ref().unwrap_or(&current.frame);

        // ターゲットサイズの決定:
        // 1. width/height が明示されていればそれを使う
        // 2. scale_x/scale_y が指定されていればソースフレームサイズに乗算する
        // 3. どちらもなければソースフレームの元サイズを使う
        let target_width = if let Some(w) = state.target_width {
            w.get()
        } else if let Some(sx) = state.scale_x {
            scale_to_even(source_frame_ref.size().width, sx)
        } else {
            source_frame_ref.size().width
        };
        let target_height = if let Some(h) = state.target_height {
            h.get()
        } else if let Some(sy) = state.scale_y {
            scale_to_even(source_frame_ref.size().height, sy)
        } else {
            source_frame_ref.size().height
        };

        let size = source_frame_ref.size();
        if size.width == target_width && size.height == target_height {
            canvas.draw_frame_clipped(x, y, source_frame_ref)?;
            continue;
        }

        let resize_width = EvenUsize::truncating_new(target_width);
        let resize_height = EvenUsize::truncating_new(target_height);
        if resize_width.get() == 0 || resize_height.get() == 0 {
            return Err(Error::new(format!(
                "invalid target size: width={} height={}",
                target_width, target_height
            )));
        }

        let resized = source_frame_ref
            .as_video_frame()
            .resize(
                resize_width,
                resize_height,
                shiguredo_libyuv::FilterMode::Bilinear,
            )?
            .ok_or_else(|| Error::new("failed to resize input frame"))?;

        let resized = RawVideoFrame::from_video_frame(Arc::new(resized))?;
        canvas.draw_frame_clipped(x, y, &resized)?;
    }

    Ok(VideoFrame {
        sample_entry: None,
        keyframe: true,
        format: VideoFormat::I420,
        timestamp,
        size: Some(VideoFrameSize {
            width: canvas_width,
            height: canvas_height,
        }),
        data: canvas.into_data(),
    })
}

#[derive(Debug)]
struct RealtimeI420Canvas {
    width: usize,
    height: usize,
    y_plane: Vec<u8>,
    u_plane: Vec<u8>,
    v_plane: Vec<u8>,
}

impl RealtimeI420Canvas {
    fn new(width: usize, height: usize) -> Self {
        let y_size = width.saturating_mul(height);
        let uv_size = width.div_ceil(2).saturating_mul(height.div_ceil(2));
        Self {
            width,
            height,
            y_plane: vec![0; y_size],
            u_plane: vec![128; uv_size],
            v_plane: vec![128; uv_size],
        }
    }

    fn draw_frame_clipped(
        &mut self,
        x: isize,
        y: isize,
        frame: &RawVideoFrame,
    ) -> crate::Result<()> {
        let size = frame.size();
        let frame = frame.as_video_frame();
        let (src_y, src_u, src_v, src_a) = match frame.format {
            VideoFormat::I420 => {
                let (src_y, src_u, src_v) = frame
                    .as_yuv_planes()
                    .ok_or_else(|| Error::new("invalid I420 frame size"))?;
                (src_y, src_u, src_v, None)
            }
            VideoFormat::I420A => {
                let (src_y, src_u, src_v, src_a) = frame
                    .as_i420a_planes()
                    .ok_or_else(|| Error::new("invalid I420A frame size"))?;
                (src_y, src_u, src_v, Some(src_a))
            }
            _ => {
                return Err(Error::new(format!(
                    "unsupported video format: expected I420 or I420A, got {}",
                    frame.format
                )));
            }
        };

        let (src_x0, dst_x0, copy_width) = clipped_span(size.width, self.width, x);
        let (src_y0, dst_y0, copy_height) = clipped_span(size.height, self.height, y);
        if copy_width == 0 || copy_height == 0 {
            return Ok(());
        }

        for row in 0..copy_height {
            for col in 0..copy_width {
                let src_x = src_x0 + col;
                let src_y_pos = src_y0 + row;
                let dst_x = dst_x0 + col;
                let dst_y_pos = dst_y0 + row;
                let src_index = src_y_pos * size.width + src_x;
                let dst_index = dst_y_pos * self.width + dst_x;
                let alpha = alpha_for_luma(src_a, size.width, src_x, src_y_pos);
                self.y_plane[dst_index] =
                    blend_component(src_y[src_index], self.y_plane[dst_index], alpha);
            }
        }

        let src_uv_width = size.width.div_ceil(2);
        let dst_uv_width = self.width.div_ceil(2);
        let src_uv_x0 = src_x0 / 2;
        let src_uv_y0 = src_y0 / 2;
        let dst_uv_x0 = dst_x0 / 2;
        let dst_uv_y0 = dst_y0 / 2;
        let copy_uv_width = copy_width.div_ceil(2);
        let copy_uv_height = copy_height.div_ceil(2);

        for row in 0..copy_uv_height {
            for col in 0..copy_uv_width {
                let src_uv_x = src_uv_x0 + col;
                let src_uv_y = src_uv_y0 + row;
                let src_index = src_uv_y * src_uv_width + src_uv_x;
                let dst_index = (dst_uv_y0 + row) * dst_uv_width + (dst_uv_x0 + col);
                let alpha = alpha_for_chroma(src_a, size.width, src_uv_x, src_uv_y);

                self.u_plane[dst_index] =
                    blend_component(src_u[src_index], self.u_plane[dst_index], alpha);
                self.v_plane[dst_index] =
                    blend_component(src_v[src_index], self.v_plane[dst_index], alpha);
            }
        }

        Ok(())
    }

    fn into_data(self) -> Vec<u8> {
        let mut data =
            Vec::with_capacity(self.y_plane.len() + self.u_plane.len() + self.v_plane.len());
        data.extend_from_slice(&self.y_plane);
        data.extend_from_slice(&self.u_plane);
        data.extend_from_slice(&self.v_plane);
        data
    }
}

fn alpha_for_luma(src_a: Option<&[u8]>, src_width: usize, src_x: usize, src_y: usize) -> u8 {
    let Some(src_a) = src_a else {
        return u8::MAX;
    };
    let index = src_y * src_width + src_x;
    src_a[index]
}

fn alpha_for_chroma(
    src_a: Option<&[u8]>,
    src_width: usize,
    src_uv_x: usize,
    src_uv_y: usize,
) -> u8 {
    let Some(src_a) = src_a else {
        return u8::MAX;
    };
    let src_x = src_uv_x * 2;
    let src_y = src_uv_y * 2;
    src_a[src_y * src_width + src_x]
}

fn blend_component(src: u8, dst: u8, alpha: u8) -> u8 {
    if alpha == u8::MAX {
        return src;
    }
    if alpha == 0 {
        return dst;
    }
    let src = u16::from(src);
    let dst = u16::from(dst);
    let alpha = u16::from(alpha);
    let blended = (src * alpha + dst * (u16::from(u8::MAX) - alpha) + 127) / u16::from(u8::MAX);
    blended as u8
}

fn clipped_span(src_len: usize, dst_len: usize, dst_pos: isize) -> (usize, usize, usize) {
    let dst_start = dst_pos.max(0) as usize;
    let src_start = if dst_pos < 0 {
        dst_pos.unsigned_abs()
    } else {
        0
    };

    let src_remaining = src_len.saturating_sub(src_start);
    let dst_remaining = dst_len.saturating_sub(dst_start);
    let copy_len = src_remaining.min(dst_remaining);

    (src_start, dst_start, copy_len)
}

fn frames_to_timestamp(frame_rate: FrameRate, frames: u64) -> Duration {
    Duration::from_secs(frames.saturating_mul(frame_rate.denumerator.get() as u64))
        / frame_rate.numerator.get() as u32
}

fn catch_up_output_frame_index(frame_rate: FrameRate, mut frame_index: u64, now: Duration) -> u64 {
    loop {
        let next = frame_index.saturating_add(1);
        if frames_to_timestamp(frame_rate, next) > now {
            break frame_index;
        }
        frame_index = next;
    }
}

fn build_draw_order(input_tracks: &[InputTrack]) -> Vec<DrawOrder> {
    let mut draw_order = input_tracks
        .iter()
        .enumerate()
        .map(|(index, input_track)| DrawOrder {
            track_id: input_track.track_id.clone(),
            z: input_track.z,
            index,
        })
        .collect::<Vec<_>>();
    draw_order.sort_by_key(|entry| (entry.z, entry.index));
    draw_order
}

fn validate_unique_input_tracks_for_json(
    input_tracks: &[InputTrack],
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<(), nojson::JsonParseError> {
    let mut seen_track_ids = HashSet::new();
    for input_track in input_tracks {
        if !seen_track_ids.insert(input_track.track_id.clone()) {
            return Err(value.invalid(format!(
                "duplicate input track ID: {}",
                input_track.track_id
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn video_realtime_mixer_json_parse() -> crate::Result<()> {
        let mixer: VideoRealtimeMixer = crate::json::parse_str(
            r#"{
                "canvasWidth": 1280,
                "canvasHeight": 720,
                "frameRate": 30,
                "inputTracks": [
                    {
                        "trackId": "input-1",
                        "x": 0,
                        "y": 0,
                        "z": 1,
                        "width": 640,
                        "height": 360
                    }
                ],
                "outputTrackId": "output"
            }"#,
        )?;

        assert_eq!(mixer.canvas_width.get(), 1280);
        assert_eq!(mixer.canvas_height.get(), 720);
        assert_eq!(mixer.frame_rate.numerator.get(), 30);
        assert_eq!(mixer.input_tracks.len(), 1);
        assert_eq!(mixer.input_tracks[0].z, 1);

        Ok(())
    }

    #[test]
    fn video_realtime_mixer_json_parse_without_z() {
        let result = crate::json::parse_str::<VideoRealtimeMixer>(
            r#"{
                "canvasWidth": 1280,
                "canvasHeight": 720,
                "frameRate": 30,
                "inputTracks": [
                    {
                        "trackId": "input-1",
                        "x": 0,
                        "y": 0,
                        "width": 640,
                        "height": 360
                    }
                ],
                "outputTrackId": "output"
            }"#,
        );

        let mixer = result.expect("infallible");
        assert_eq!(mixer.input_tracks[0].z, 0);
    }

    #[test]
    fn video_realtime_mixer_json_parse_without_frame_rate() -> crate::Result<()> {
        let mixer = crate::json::parse_str::<VideoRealtimeMixer>(
            r#"{
                "canvasWidth": 1280,
                "canvasHeight": 720,
                "inputTracks": [
                    {
                        "trackId": "input-1",
                        "x": 0,
                        "y": 0,
                        "z": 0,
                        "width": 640,
                        "height": 360
                    }
                ],
                "outputTrackId": "output"
            }"#,
        )?;

        assert_eq!(mixer.frame_rate.numerator.get(), 30);
        assert_eq!(mixer.frame_rate.denumerator.get(), 1);
        Ok(())
    }

    #[test]
    fn video_realtime_mixer_json_parse_defaults_for_optional_position_and_size() -> crate::Result<()>
    {
        let mixer: VideoRealtimeMixer = crate::json::parse_str(
            r#"{
                "canvasWidth": 1280,
                "canvasHeight": 720,
                "frameRate": 30,
                "inputTracks": [
                    {
                        "trackId": "input-1"
                    }
                ],
                "outputTrackId": "output"
            }"#,
        )?;

        let track = &mixer.input_tracks[0];
        assert_eq!(track.x, 0);
        assert_eq!(track.y, 0);
        assert_eq!(track.z, 0);
        assert_eq!(track.width, None);
        assert_eq!(track.height, None);
        Ok(())
    }

    #[test]
    fn video_realtime_mixer_json_parse_fraction_string_frame_rate() -> crate::Result<()> {
        let mixer: VideoRealtimeMixer = crate::json::parse_str(
            r#"{
                "canvasWidth": 1280,
                "canvasHeight": 720,
                "frameRate": "30000/1001",
                "inputTracks": [
                    {
                        "trackId": "input-1",
                        "x": 0,
                        "y": 0,
                        "z": 0,
                        "width": 640,
                        "height": 360
                    }
                ],
                "outputTrackId": "output"
            }"#,
        )?;

        assert_eq!(mixer.frame_rate.numerator.get(), 30000);
        assert_eq!(mixer.frame_rate.denumerator.get(), 1001);
        Ok(())
    }

    #[test]
    fn video_realtime_mixer_json_parse_error_with_integer_string_frame_rate() {
        let result = crate::json::parse_str::<VideoRealtimeMixer>(
            r#"{
                "canvasWidth": 1280,
                "canvasHeight": 720,
                "frameRate": "30",
                "inputTracks": [
                    {
                        "trackId": "input-1",
                        "x": 0,
                        "y": 0,
                        "z": 0,
                        "width": 640,
                        "height": 360
                    }
                ],
                "outputTrackId": "output"
            }"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn video_realtime_mixer_json_parse_error_with_too_high_frame_rate() {
        let result = crate::json::parse_str::<VideoRealtimeMixer>(
            r#"{
                "canvasWidth": 1280,
                "canvasHeight": 720,
                "frameRate": 4294967296,
                "inputTracks": [
                    {
                        "trackId": "input-1",
                        "x": 0,
                        "y": 0,
                        "z": 0,
                        "width": 640,
                        "height": 360
                    }
                ],
                "outputTrackId": "output"
            }"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn video_realtime_mixer_json_parse_error_with_duplicate_input_track_id() {
        let result = crate::json::parse_str::<VideoRealtimeMixer>(
            r#"{
                "canvasWidth": 1280,
                "canvasHeight": 720,
                "frameRate": 30,
                "inputTracks": [
                    {
                        "trackId": "input-1",
                        "x": 0,
                        "y": 0,
                        "z": 0
                    },
                    {
                        "trackId": "input-1",
                        "x": 10,
                        "y": 10,
                        "z": 1
                    }
                ],
                "outputTrackId": "output"
            }"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn video_realtime_mixer_json_parse_error_with_odd_canvas_size() {
        let result = crate::json::parse_str::<VideoRealtimeMixer>(
            r#"{
                "canvasWidth": 1279,
                "canvasHeight": 720,
                "frameRate": 30,
                "inputTracks": [
                    {
                        "trackId": "input-1"
                    }
                ],
                "outputTrackId": "output"
            }"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn video_realtime_mixer_json_parse_with_zero_canvas_size() -> crate::Result<()> {
        let mixer = crate::json::parse_str::<VideoRealtimeMixer>(
            r#"{
                "canvasWidth": 0,
                "canvasHeight": 720,
                "frameRate": 30,
                "inputTracks": [
                    {
                        "trackId": "input-1"
                    }
                ],
                "outputTrackId": "output"
            }"#,
        )?;

        assert_eq!(mixer.canvas_width.get(), 0);
        Ok(())
    }

    #[test]
    fn video_realtime_mixer_json_parse_error_with_odd_input_size() {
        let result = crate::json::parse_str::<VideoRealtimeMixer>(
            r#"{
                "canvasWidth": 1280,
                "canvasHeight": 720,
                "frameRate": 30,
                "inputTracks": [
                    {
                        "trackId": "input-1",
                        "width": 639
                    }
                ],
                "outputTrackId": "output"
            }"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn video_realtime_mixer_json_parse_with_zero_input_size() -> crate::Result<()> {
        let mixer = crate::json::parse_str::<VideoRealtimeMixer>(
            r#"{
                "canvasWidth": 1280,
                "canvasHeight": 720,
                "frameRate": 30,
                "inputTracks": [
                    {
                        "trackId": "input-1",
                        "height": 0
                    }
                ],
                "outputTrackId": "output"
            }"#,
        )?;

        assert_eq!(mixer.input_tracks[0].height.map(EvenUsize::get), Some(0));
        Ok(())
    }

    #[tokio::test]
    async fn video_realtime_mixer_two_tracks_smoke() -> crate::Result<()> {
        let pipeline = crate::MediaPipeline::new()?;
        let pipeline_handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let output_track_id = TrackId::new("mixed-output");
        let input_track_id_1 = TrackId::new("input-1");
        let input_track_id_2 = TrackId::new("input-2");

        let mixer = VideoRealtimeMixer {
            canvas_width: EvenUsize::new(320).expect("infallible"),
            canvas_height: EvenUsize::new(240).expect("infallible"),
            frame_rate: FrameRate::FPS_25,
            input_tracks: vec![
                InputTrack {
                    track_id: input_track_id_1.clone(),
                    x: 0,
                    y: 0,
                    z: 0,
                    width: Some(EvenUsize::new(160).expect("infallible")),
                    height: Some(EvenUsize::new(120).expect("infallible")),
                    scale_x: None,
                    scale_y: None,
                    crop_top: 0,
                    crop_bottom: 0,
                    crop_left: 0,
                    crop_right: 0,
                },
                InputTrack {
                    track_id: input_track_id_2.clone(),
                    x: 80,
                    y: 60,
                    z: 1,
                    width: Some(EvenUsize::new(160).expect("infallible")),
                    height: Some(EvenUsize::new(120).expect("infallible")),
                    scale_x: None,
                    scale_y: None,
                    crop_top: 0,
                    crop_bottom: 0,
                    crop_left: 0,
                    crop_right: 0,
                },
            ],
            output_track_id: output_track_id.clone(),
        };

        let mixer_processor = pipeline_handle
            .register_processor(
                crate::ProcessorId::new("mixer"),
                crate::ProcessorMetadata::new("video_mixer"),
            )
            .await?;
        let sink_processor = pipeline_handle
            .register_processor(
                crate::ProcessorId::new("sink"),
                crate::ProcessorMetadata::new("test_sink"),
            )
            .await?;
        let sender1_processor = pipeline_handle
            .register_processor(
                crate::ProcessorId::new("sender1"),
                crate::ProcessorMetadata::new("test_sender"),
            )
            .await?;
        let sender2_processor = pipeline_handle
            .register_processor(
                crate::ProcessorId::new("sender2"),
                crate::ProcessorMetadata::new("test_sender"),
            )
            .await?;
        mixer_processor.notify_ready();
        sink_processor.notify_ready();
        sender1_processor.notify_ready();
        sender2_processor.notify_ready();
        assert!(
            pipeline_handle
                .trigger_start()
                .await
                .expect("trigger_start must succeed")
        );

        let mixer_task = tokio::spawn(async move { mixer.run(mixer_processor).await });

        let sender1_task = tokio::spawn(async move {
            let mut tx = sender1_processor.publish_track(input_track_id_1).await?;
            for i in 0..5 {
                tx.send_video(dummy_frame(Duration::from_millis(i * 40)));
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            tx.send_eos();
            Ok::<(), crate::Error>(())
        });

        let sender2_task = tokio::spawn(async move {
            let mut tx = sender2_processor.publish_track(input_track_id_2).await?;
            for i in 0..5 {
                tx.send_video(dummy_frame(Duration::from_millis(100 + i * 40)));
                tokio::time::sleep(Duration::from_millis(12)).await;
            }
            tx.send_eos();
            Ok::<(), crate::Error>(())
        });

        let mut mixed_rx = sink_processor.subscribe_track(output_track_id);
        let mut received_video_frame_count = 0;
        while received_video_frame_count < 5 {
            let message = tokio::time::timeout(Duration::from_secs(2), mixed_rx.recv())
                .await
                .map_err(|e| Error::new(e.to_string()))?;
            if matches!(message, Message::Media(MediaFrame::Video(_))) {
                received_video_frame_count += 1;
            }
        }

        assert!(received_video_frame_count >= 5);

        sender1_task
            .await
            .map_err(|e| Error::new(e.to_string()))??;
        sender2_task
            .await
            .map_err(|e| Error::new(e.to_string()))??;

        mixer_task.abort();
        let _ = mixer_task.await;

        drop(mixed_rx);
        drop(sink_processor);
        drop(pipeline_handle);

        tokio::time::timeout(Duration::from_secs(2), pipeline_task)
            .await
            .map_err(|e| Error::new(e.to_string()))?
            .map_err(|e| Error::new(e.to_string()))?;

        Ok(())
    }

    #[tokio::test]
    async fn spawn_input_receiver_forwards_syn_and_ack_waits_until_event_drop() -> crate::Result<()>
    {
        let pipeline = crate::MediaPipeline::new()?;
        let pipeline_handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let sender_processor = pipeline_handle
            .register_processor(
                crate::ProcessorId::new("syn_sender"),
                crate::ProcessorMetadata::new("test_sender"),
            )
            .await?;
        let receiver_processor = pipeline_handle
            .register_processor(
                crate::ProcessorId::new("syn_receiver"),
                crate::ProcessorMetadata::new("test_receiver"),
            )
            .await?;
        sender_processor.notify_ready();
        receiver_processor.notify_ready();
        assert!(
            pipeline_handle
                .trigger_start()
                .await
                .expect("trigger_start must succeed")
        );

        let track_id = TrackId::new("syn-track");
        let mut tx = sender_processor.publish_track(track_id.clone()).await?;
        let input_rx = receiver_processor.subscribe_track(track_id.clone());

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let mixer_start = tokio::time::Instant::now();
        let receiver_handle =
            spawn_input_receiver(track_id.clone(), input_rx, event_tx, mixer_start);

        let mut first_event = None;
        for _ in 0..40 {
            tx.send_video(dummy_frame(Duration::from_millis(0)));
            if let Ok(Some(event)) =
                tokio::time::timeout(Duration::from_millis(50), event_rx.recv()).await
            {
                first_event = Some(event);
                break;
            }
        }
        let first_event =
            first_event.ok_or_else(|| Error::new("failed to receive first video event"))?;
        assert!(matches!(
            first_event,
            TrackEvent::Video {
                track_id: event_track_id,
                ..
            } if event_track_id == track_id
        ));

        let ack = tx.send_syn();
        tokio::pin!(ack);

        let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .map_err(|e| Error::new(e.to_string()))?
            .ok_or_else(|| Error::new("event channel closed unexpectedly"))?;
        assert!(matches!(&event, TrackEvent::Syn(_)));

        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut ack)
                .await
                .is_err()
        );

        drop(event);
        tokio::time::timeout(Duration::from_secs(2), &mut ack)
            .await
            .map_err(|e| Error::new(e.to_string()))?;

        tx.send_eos();
        drop(receiver_handle);

        drop(receiver_processor);
        drop(sender_processor);
        drop(pipeline_handle);

        tokio::time::timeout(Duration::from_secs(2), pipeline_task)
            .await
            .map_err(|e| Error::new(e.to_string()))?
            .map_err(|e| Error::new(e.to_string()))?;

        Ok(())
    }

    #[test]
    fn input_track_state_handle_eos_clears_pending_and_current_frame() -> crate::Result<()> {
        let input_track = InputTrack {
            track_id: TrackId::new("input-1"),
            x: 0,
            y: 0,
            z: 0,
            width: None,
            height: None,
            scale_x: None,
            scale_y: None,
            crop_top: 0,
            crop_bottom: 0,
            crop_left: 0,
            crop_right: 0,
        };
        let mut state = InputTrackState::new(input_track)?;

        state.handle_video(
            Arc::new(dummy_frame(Duration::from_millis(10))),
            Duration::from_millis(5),
        )?;
        state.handle_video(
            Arc::new(dummy_frame(Duration::from_millis(60))),
            Duration::from_millis(10),
        )?;
        state.advance(Duration::from_millis(5));

        assert!(state.current_frame.is_some());
        assert!(!state.pending_frames.is_empty());

        state.handle_eos();

        assert!(state.eos);
        assert!(state.current_frame.is_none());
        assert!(state.pending_frames.is_empty());

        Ok(())
    }

    #[test]
    fn input_track_state_accepts_i420a_frame() -> crate::Result<()> {
        let input_track = InputTrack {
            track_id: TrackId::new("input-alpha"),
            x: 0,
            y: 0,
            z: 0,
            width: None,
            height: None,
            scale_x: None,
            scale_y: None,
            crop_top: 0,
            crop_bottom: 0,
            crop_left: 0,
            crop_right: 0,
        };
        let mut state = InputTrackState::new(input_track)?;

        let frame = Arc::new(dummy_i420a_frame(Duration::from_millis(10), 200, 128));
        state.handle_video(frame, Duration::from_millis(1))?;
        state.advance(Duration::from_millis(1));

        assert!(state.current_frame.is_some());
        Ok(())
    }

    #[test]
    fn input_track_state_update_input_track_keeps_buffered_frames() -> crate::Result<()> {
        let input_track = InputTrack {
            track_id: TrackId::new("input-update"),
            x: 0,
            y: 0,
            z: 0,
            width: Some(EvenUsize::new(160).expect("infallible")),
            height: Some(EvenUsize::new(120).expect("infallible")),
            scale_x: None,
            scale_y: None,
            crop_top: 0,
            crop_bottom: 0,
            crop_left: 0,
            crop_right: 0,
        };
        let mut state = InputTrackState::new(input_track)?;
        state.current_frame = Some(PendingVideoFrame {
            timestamp: Duration::from_millis(10),
            frame: RawVideoFrame::from_video_frame(Arc::new(dummy_frame(Duration::from_millis(
                10,
            ))))
            .expect("infallible"),
        });
        state.pending_frames.push_back(PendingVideoFrame {
            timestamp: Duration::from_millis(20),
            frame: RawVideoFrame::from_video_frame(Arc::new(dummy_frame(Duration::from_millis(
                20,
            ))))
            .expect("infallible"),
        });

        state.update_input_track(InputTrack {
            track_id: TrackId::new("input-update"),
            x: 100,
            y: 50,
            z: 3,
            width: Some(EvenUsize::new(320).expect("infallible")),
            height: Some(EvenUsize::new(180).expect("infallible")),
            scale_x: None,
            scale_y: None,
            crop_top: 0,
            crop_bottom: 0,
            crop_left: 0,
            crop_right: 0,
        });

        assert_eq!(state.input_track.x, 100);
        assert_eq!(state.input_track.y, 50);
        assert_eq!(state.input_track.z, 3);
        assert_eq!(state.target_width.map(EvenUsize::get), Some(320));
        assert_eq!(state.target_height.map(EvenUsize::get), Some(180));
        assert!(state.current_frame.is_some());
        assert_eq!(state.pending_frames.len(), 1);
        Ok(())
    }

    #[test]
    fn compose_frame_blends_i420a_and_outputs_i420() -> crate::Result<()> {
        let track_id = TrackId::new("alpha-track");
        let input_track = InputTrack {
            track_id: track_id.clone(),
            x: 0,
            y: 0,
            z: 0,
            width: None,
            height: None,
            scale_x: None,
            scale_y: None,
            crop_top: 0,
            crop_bottom: 0,
            crop_left: 0,
            crop_right: 0,
        };
        let mut state = InputTrackState::new(input_track)?;
        state.current_frame = Some(PendingVideoFrame {
            timestamp: Duration::ZERO,
            frame: RawVideoFrame::from_video_frame(Arc::new(dummy_i420a_frame(
                Duration::ZERO,
                200,
                128,
            )))
            .expect("infallible"),
        });

        let draw_order = vec![DrawOrder {
            track_id: track_id.clone(),
            z: 0,
            index: 0,
        }];
        let mut states = HashMap::new();
        states.insert(track_id, state);

        let frame = compose_frame(2, 2, Duration::ZERO, &draw_order, &states)?;

        assert_eq!(frame.format, VideoFormat::I420);
        assert_eq!(frame.data[0], 100);
        assert_eq!(frame.data[1], 100);
        assert_eq!(frame.data[2], 100);
        assert_eq!(frame.data[3], 100);
        assert_eq!(frame.data[4], 164);
        assert_eq!(frame.data[5], 164);
        Ok(())
    }

    fn dummy_frame(timestamp: Duration) -> VideoFrame {
        let mut frame =
            VideoFrame::black(EvenUsize::truncating_new(64), EvenUsize::truncating_new(64));
        frame.timestamp = timestamp;
        frame
    }

    fn dummy_i420a_frame(timestamp: Duration, y: u8, alpha: u8) -> VideoFrame {
        VideoFrame {
            sample_entry: None,
            keyframe: true,
            format: VideoFormat::I420A,
            size: Some(VideoFrameSize {
                width: 2,
                height: 2,
            }),
            timestamp,
            data: vec![y, y, y, y, 200, 200, alpha, alpha, alpha, alpha],
        }
    }

    #[test]
    fn catch_up_output_frame_index_skips_late_frames() {
        let now = Duration::from_millis(120);
        let index = catch_up_output_frame_index(FrameRate::FPS_25, 0, now);
        assert_eq!(index, 3);
    }

    #[test]
    fn catch_up_output_frame_index_keeps_current_when_not_late() {
        let now = Duration::from_millis(39);
        let index = catch_up_output_frame_index(FrameRate::FPS_25, 0, now);
        assert_eq!(index, 0);
    }
}
