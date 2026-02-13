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

#[derive(Debug)]
pub struct VideoRealtimeMixer {
    pub canvas_width: usize,
    pub canvas_height: usize,
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
        let canvas_width: usize = value.to_member("canvasWidth")?.required()?.try_into()?;
        let canvas_height: usize = value.to_member("canvasHeight")?.required()?.try_into()?;

        let frame_rate = value
            .to_member("frameRate")?
            .try_into()?
            .unwrap_or(FrameRate::FPS_30);

        let input_tracks: Vec<InputTrack> =
            value.to_member("inputTracks")?.required()?.try_into()?;
        let output_track_id: TrackId = value.to_member("outputTrackId")?.required()?.try_into()?;

        Ok(Self {
            canvas_width,
            canvas_height,
            frame_rate,
            input_tracks,
            output_track_id,
        })
    }
}

impl VideoRealtimeMixer {
    pub async fn run(self, handle: ProcessorHandle) -> crate::Result<()> {
        if self.canvas_width == 0 || self.canvas_height == 0 {
            return Err(Error::new("canvas width and height must be positive"));
        }

        let frame_interval = frames_to_timestamp(self.frame_rate, 1);
        if frame_interval.is_zero() {
            return Err(Error::new("frameRate is too high"));
        }

        let mut output_tx = handle.publish_track(self.output_track_id).await?;

        let mut seen_track_ids = HashSet::new();
        let mut draw_order = Vec::with_capacity(self.input_tracks.len());
        let mut states = HashMap::with_capacity(self.input_tracks.len());
        for (index, input_track) in self.input_tracks.into_iter().enumerate() {
            if !seen_track_ids.insert(input_track.track_id.clone()) {
                return Err(Error::new(format!(
                    "duplicate input track ID: {}",
                    input_track.track_id
                )));
            }

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

        let mut event_rx = Some(event_rx);

        let mut ticker = tokio::time::interval(frame_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut output_frame_count = 0u64;
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let now = mixer_start.elapsed();
                    for state in states.values_mut() {
                        state.advance(now);
                    }

                    let timestamp = frames_to_timestamp(self.frame_rate, output_frame_count);
                    let duration = frames_to_timestamp(self.frame_rate, output_frame_count + 1)
                        .saturating_sub(timestamp);
                    output_frame_count = output_frame_count.saturating_add(1);

                    let frame = compose_frame(
                        self.canvas_width,
                        self.canvas_height,
                        timestamp,
                        duration,
                        &draw_order,
                        &states,
                    )?;

                    if !output_tx.send_video(frame) {
                        break;
                    }
                }
                event = recv_track_event_or_pending(&mut event_rx) => {
                    let Some(event) = event else {
                        event_rx = None;
                        continue;
                    };
                    handle_track_event(event, &mut states)?;
                }
            }
        }

        Ok(())
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
                Message::Syn(_) => {}
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

        assert_eq!(mixer.canvas_width, 1280);
        assert_eq!(mixer.canvas_height, 720);
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

    #[tokio::test]
    async fn video_realtime_mixer_two_tracks_smoke() -> crate::Result<()> {
        let pipeline = crate::MediaPipeline::new();
        let pipeline_handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let output_track_id = TrackId::new("mixed-output");
        let input_track_id_1 = TrackId::new("input-1");
        let input_track_id_2 = TrackId::new("input-2");

        let mixer = VideoRealtimeMixer {
            canvas_width: 320,
            canvas_height: 240,
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

    fn dummy_frame(timestamp: Duration) -> VideoFrame {
        let mut frame =
            VideoFrame::black(EvenUsize::truncating_new(64), EvenUsize::truncating_new(64));
        frame.timestamp = timestamp;
        frame.duration = Duration::from_millis(40);
        frame
    }
}
