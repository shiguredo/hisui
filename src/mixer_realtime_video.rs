use std::{
    collections::{HashMap, HashSet, VecDeque},
    num::NonZeroUsize,
    sync::Arc,
    time::Duration,
};

use crate::{
    Error, MediaSample, Message, ProcessorHandle, TrackId,
    types::EvenUsize,
    video::{FrameRate, VideoFormat, VideoFrame},
};

const MAX_NOACKED_COUNT: u64 = 100;

#[derive(Debug)]
pub struct VideoRealtimeMixer {
    pub canvas_width: NonZeroUsize,
    pub canvas_height: NonZeroUsize,
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
        let mut seen_track_ids = HashSet::new();
        for track in &input_tracks {
            if !seen_track_ids.insert(track.track_id.clone()) {
                return Err(value.invalid(format!("duplicate input track ID: {}", track.track_id)));
            }
        }
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

        let output_tx = handle.publish_track(output_track_id).await?;

        let mut draw_order = Vec::with_capacity(input_tracks.len());
        let mut states = HashMap::with_capacity(input_tracks.len());
        for (index, input_track) in input_tracks.iter().enumerate() {
            let state = InputTrackState::new(input_track.clone())?;
            draw_order.push(DrawOrder {
                track_id: input_track.track_id.clone(),
                z: input_track.z,
                index,
            });
            states.insert(input_track.track_id.clone(), state);
        }

        draw_order.sort_by_key(|x| (x.z, x.index));

        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let mixer_start = tokio::time::Instant::now();
        for track in &draw_order {
            let input_rx = handle.subscribe_track(track.track_id.clone());
            spawn_input_receiver(
                track.track_id.clone(),
                input_rx,
                event_tx.clone(),
                mixer_start,
            );
        }
        drop(event_tx);

        VideoRealtimeMixerRunner::new(
            canvas_width.get(),
            canvas_height.get(),
            frame_rate,
            output_tx,
            draw_order,
            states,
            event_rx,
            mixer_start,
        )
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
    pub width: Option<usize>,
    pub height: Option<usize>,
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
        let width: Option<usize> = value.to_member("width")?.try_into()?;
        let height: Option<usize> = value.to_member("height")?.try_into()?;

        Ok(Self {
            track_id,
            x: x.unwrap_or_default(),
            y: y.unwrap_or_default(),
            z: z.unwrap_or_default(),
            width,
            height,
        })
    }
}

#[derive(Debug)]
struct DrawOrder {
    track_id: TrackId,
    z: isize,
    index: usize,
}

#[derive(Debug)]
struct VideoRealtimeMixerRunner {
    canvas_width: usize,
    canvas_height: usize,
    frame_rate: FrameRate,
    output_tx: crate::MessageSender,
    draw_order: Vec<DrawOrder>,
    states: HashMap<TrackId, InputTrackState>,
    event_rx: Option<tokio::sync::mpsc::UnboundedReceiver<TrackEvent>>,
    mixer_start: tokio::time::Instant,
    output_frame_index: u64,
    noacked_sent: u64,
    ack: Option<crate::Ack>,
}

impl VideoRealtimeMixerRunner {
    fn new(
        canvas_width: usize,
        canvas_height: usize,
        frame_rate: FrameRate,
        mut output_tx: crate::MessageSender,
        draw_order: Vec<DrawOrder>,
        states: HashMap<TrackId, InputTrackState>,
        event_rx: tokio::sync::mpsc::UnboundedReceiver<TrackEvent>,
        mixer_start: tokio::time::Instant,
    ) -> Self {
        let ack = Some(output_tx.send_syn());
        Self {
            canvas_width,
            canvas_height,
            frame_rate,
            output_tx,
            draw_order,
            states,
            event_rx: Some(event_rx),
            mixer_start,
            output_frame_index: 0,
            noacked_sent: 0,
            ack,
        }
    }

    async fn run(mut self) -> crate::Result<()> {
        let mut event_rx = self.event_rx.take();
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
        let duration =
            frames_to_timestamp(self.frame_rate, self.output_frame_index.saturating_add(1))
                .saturating_sub(timestamp);
        self.output_frame_index = self.output_frame_index.saturating_add(1);

        let frame = compose_frame(
            self.canvas_width,
            self.canvas_height,
            timestamp,
            duration,
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
}

#[derive(Debug)]
struct PendingVideoFrame {
    timestamp: Duration,
    frame: Arc<VideoFrame>,
}

#[derive(Debug)]
struct InputTrackState {
    input_track: InputTrack,
    target_width: Option<EvenUsize>,
    target_height: Option<EvenUsize>,
    first_input_sample_timestamp: Option<Duration>,
    first_input_elapsed: Option<Duration>,
    pending_frames: VecDeque<PendingVideoFrame>,
    current_frame: Option<PendingVideoFrame>,
    eos: bool,
}

impl InputTrackState {
    fn new(input_track: InputTrack) -> crate::Result<Self> {
        let target_width = input_track.width.map(EvenUsize::truncating_new);
        let target_height = input_track.height.map(EvenUsize::truncating_new);

        if input_track.width.is_some() && target_width.is_some_and(|w| w.get() == 0) {
            return Err(Error::new(format!(
                "input track width must be at least 2: track={} width={:?}",
                input_track.track_id, input_track.width,
            )));
        }
        if input_track.height.is_some() && target_height.is_some_and(|h| h.get() == 0) {
            return Err(Error::new(format!(
                "input track height must be at least 2: track={} height={:?}",
                input_track.track_id, input_track.height,
            )));
        }

        Ok(Self {
            input_track,
            target_width,
            target_height,
            first_input_sample_timestamp: None,
            first_input_elapsed: None,
            pending_frames: VecDeque::new(),
            current_frame: None,
            eos: false,
        })
    }

    fn handle_video(&mut self, frame: Arc<VideoFrame>, received_at: Duration) -> crate::Result<()> {
        if frame.format != VideoFormat::I420 {
            return Err(Error::new(format!(
                "unsupported video format: expected I420, got {}",
                frame.format
            )));
        }

        let first_sample_timestamp = *self
            .first_input_sample_timestamp
            .get_or_insert(frame.timestamp);
        let first_input_elapsed = *self.first_input_elapsed.get_or_insert(received_at);

        let adjusted_timestamp = frame
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

fn spawn_input_receiver(
    track_id: TrackId,
    mut input_rx: crate::MessageReceiver,
    event_tx: tokio::sync::mpsc::UnboundedSender<TrackEvent>,
    mixer_start: tokio::time::Instant,
) {
    tokio::spawn(async move {
        loop {
            match input_rx.recv().await {
                Message::Media(sample) => match sample {
                    MediaSample::Video(frame) => {
                        let _ = event_tx.send(TrackEvent::Video {
                            track_id: track_id.clone(),
                            frame,
                            received_at: mixer_start.elapsed(),
                        });
                    }
                    MediaSample::Audio(_) => {
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
                return Err(Error::new(format!("unknown input track ID: {}", track_id)));
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
            return Err(Error::new(format!("input track {}: {}", track_id, reason)));
        }
    }

    Ok(())
}

fn compose_frame(
    canvas_width: usize,
    canvas_height: usize,
    timestamp: Duration,
    duration: Duration,
    draw_order: &[DrawOrder],
    states: &HashMap<TrackId, InputTrackState>,
) -> crate::Result<VideoFrame> {
    let mut canvas = Canvas::new(canvas_width, canvas_height);

    for draw in draw_order {
        let Some(state) = states.get(&draw.track_id) else {
            continue;
        };
        let Some(current) = state.current_frame.as_ref() else {
            continue;
        };

        let x = state.input_track.x;
        let y = state.input_track.y;
        let target_width = state
            .target_width
            .map(|w| w.get())
            .unwrap_or(current.frame.width);
        let target_height = state
            .target_height
            .map(|h| h.get())
            .unwrap_or(current.frame.height);

        if current.frame.width == target_width && current.frame.height == target_height {
            canvas.draw_frame_clipped(x, y, &current.frame)?;
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

        let resized = current
            .frame
            .resize(
                resize_width,
                resize_height,
                shiguredo_libyuv::FilterMode::Bilinear,
            )
            .map_err(|e| Error::new(e.to_string()))?
            .ok_or_else(|| Error::new("failed to resize input frame"))?;

        canvas.draw_frame_clipped(x, y, &resized)?;
    }

    Ok(VideoFrame {
        source_id: None,
        sample_entry: None,
        keyframe: true,
        format: VideoFormat::I420,
        timestamp,
        duration,
        width: canvas_width,
        height: canvas_height,
        data: canvas.data,
    })
}

#[derive(Debug)]
struct Canvas {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            data: black_i420_data(width, height),
        }
    }

    fn draw_frame_clipped(&mut self, x: isize, y: isize, frame: &VideoFrame) -> crate::Result<()> {
        if frame.format != VideoFormat::I420 {
            return Err(Error::new("unsupported video format: expected I420"));
        }

        let src_y_size = frame.width.saturating_mul(frame.height);
        let src_uv_width = frame.width.div_ceil(2);
        let src_uv_height = frame.height.div_ceil(2);
        let src_uv_size = src_uv_width.saturating_mul(src_uv_height);

        if frame.data.len() < src_y_size.saturating_add(src_uv_size.saturating_mul(2)) {
            return Err(Error::new("invalid I420 frame size"));
        }

        let src_y = &frame.data[..src_y_size];
        let src_u = &frame.data[src_y_size..][..src_uv_size];
        let src_v = &frame.data[src_y_size + src_uv_size..][..src_uv_size];

        let (src_x0, dst_x0, copy_width) = clipped_span(frame.width, self.width, x);
        let (src_y0, dst_y0, copy_height) = clipped_span(frame.height, self.height, y);

        if copy_width == 0 || copy_height == 0 {
            return Ok(());
        }

        for row in 0..copy_height {
            let src_offset = (src_y0 + row) * frame.width + src_x0;
            let dst_offset = (dst_y0 + row) * self.width + dst_x0;
            self.data[dst_offset..][..copy_width]
                .copy_from_slice(&src_y[src_offset..][..copy_width]);
        }

        let dst_y_size = self.width.saturating_mul(self.height);
        let dst_uv_width = self.width.div_ceil(2);
        let dst_uv_height = self.height.div_ceil(2);
        let dst_uv_size = dst_uv_width.saturating_mul(dst_uv_height);

        let src_uv_x0 = src_x0 / 2;
        let src_uv_y0 = src_y0 / 2;
        let dst_uv_x0 = dst_x0 / 2;
        let dst_uv_y0 = dst_y0 / 2;
        let copy_uv_width = copy_width.div_ceil(2);
        let copy_uv_height = copy_height.div_ceil(2);

        for row in 0..copy_uv_height {
            let src_offset = (src_uv_y0 + row) * src_uv_width + src_uv_x0;
            let dst_offset = (dst_uv_y0 + row) * dst_uv_width + dst_uv_x0;

            let dst_u_offset = dst_y_size + dst_offset;
            let dst_v_offset = dst_y_size + dst_uv_size + dst_offset;

            self.data[dst_u_offset..][..copy_uv_width]
                .copy_from_slice(&src_u[src_offset..][..copy_uv_width]);
            self.data[dst_v_offset..][..copy_uv_width]
                .copy_from_slice(&src_v[src_offset..][..copy_uv_width]);
        }

        Ok(())
    }
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

fn black_i420_data(width: usize, height: usize) -> Vec<u8> {
    let y_size = width.saturating_mul(height);
    let uv_size = width.div_ceil(2).saturating_mul(height.div_ceil(2));
    let mut data = vec![0; y_size.saturating_add(uv_size.saturating_mul(2))];
    data[y_size..].fill(128);
    data
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
        )
        .map_err(|e| Error::new(e.to_string()))?;

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
        )
        .map_err(|e| Error::new(e.to_string()))?;

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
        )
        .map_err(|e| Error::new(e.to_string()))?;

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
        )
        .map_err(|e| Error::new(e.to_string()))?;

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

    #[tokio::test]
    async fn video_realtime_mixer_two_tracks_smoke() -> crate::Result<()> {
        let pipeline = crate::MediaPipeline::new();
        let pipeline_handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let output_track_id = TrackId::new("mixed-output");
        let input_track_id_1 = TrackId::new("input-1");
        let input_track_id_2 = TrackId::new("input-2");

        let mixer = VideoRealtimeMixer {
            canvas_width: NonZeroUsize::new(320).expect("infallible"),
            canvas_height: NonZeroUsize::new(240).expect("infallible"),
            frame_rate: FrameRate::FPS_25,
            input_tracks: vec![
                InputTrack {
                    track_id: input_track_id_1.clone(),
                    x: 0,
                    y: 0,
                    z: 0,
                    width: Some(160),
                    height: Some(120),
                },
                InputTrack {
                    track_id: input_track_id_2.clone(),
                    x: 80,
                    y: 60,
                    z: 1,
                    width: Some(160),
                    height: Some(120),
                },
            ],
            output_track_id: output_track_id.clone(),
        };

        let mixer_processor = pipeline_handle
            .register_processor(crate::ProcessorId::new("mixer"))
            .await?;
        let sink_processor = pipeline_handle
            .register_processor(crate::ProcessorId::new("sink"))
            .await?;
        let sender1_processor = pipeline_handle
            .register_processor(crate::ProcessorId::new("sender1"))
            .await?;
        let sender2_processor = pipeline_handle
            .register_processor(crate::ProcessorId::new("sender2"))
            .await?;

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
            if matches!(message, Message::Media(MediaSample::Video(_))) {
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
        let pipeline = crate::MediaPipeline::new();
        let pipeline_handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let sender_processor = pipeline_handle
            .register_processor(crate::ProcessorId::new("syn_sender"))
            .await?;
        let receiver_processor = pipeline_handle
            .register_processor(crate::ProcessorId::new("syn_receiver"))
            .await?;

        let track_id = TrackId::new("syn-track");
        let mut tx = sender_processor.publish_track(track_id.clone()).await?;
        let input_rx = receiver_processor.subscribe_track(track_id.clone());

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let mixer_start = tokio::time::Instant::now();
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

    fn dummy_frame(timestamp: Duration) -> VideoFrame {
        let mut frame =
            VideoFrame::black(EvenUsize::truncating_new(64), EvenUsize::truncating_new(64));
        frame.timestamp = timestamp;
        frame.duration = Duration::from_millis(40);
        frame
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
