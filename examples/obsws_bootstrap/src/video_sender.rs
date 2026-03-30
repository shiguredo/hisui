use std::sync::Arc;
use std::time::Duration;

use shiguredo_webrtc::{
    AdaptedVideoTrackSource, I420Buffer, PeerConnection, PeerConnectionFactory, RtpSender,
    StringVector, TimestampAligner, VideoFrame,
};

/// 単色 I420 フレームの Y, U, V 値を返す
fn solid_color_yuv(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let r = i32::from(r);
    let g = i32::from(g);
    let b = i32::from(b);
    let y = ((77 * r + 150 * g + 29 * b + 128) >> 8).clamp(0, 255) as u8;
    let u = (((-43 * r - 85 * g + 128 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
    let v = (((128 * r - 107 * g - 21 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
    (y, u, v)
}

/// VideoTrack を作成して PeerConnection に追加する
pub fn create_and_add_video_track(
    factory: &Arc<PeerConnectionFactory>,
    pc: &PeerConnection,
    track_id: &str,
) -> Result<(AdaptedVideoTrackSource, RtpSender), String> {
    let source = AdaptedVideoTrackSource::new();
    let video_track_source = source.cast_to_video_track_source();
    let video_track = factory
        .create_video_track(&video_track_source, track_id)
        .map_err(|e| format!("failed to create video track: {e}"))?;

    let media_track = video_track.cast_to_media_stream_track();
    let stream_ids = StringVector::new(0);
    let sender = pc
        .add_track(&media_track, &stream_ids)
        .map_err(|e| format!("failed to add video track: {e}"))?;

    Ok((source, sender))
}

/// 単色フレームを定期的に送信するタスク（緑: #00FF00）
pub async fn send_frames_task(
    mut source: AdaptedVideoTrackSource,
    width: i32,
    height: i32,
    fps: u32,
    deadline: tokio::time::Instant,
) {
    let (y_val, u_val, v_val) = solid_color_yuv(0, 255, 0); // 緑
    let interval = Duration::from_micros(1_000_000 / u64::from(fps));
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut aligner = TimestampAligner::new();

    loop {
        ticker.tick().await;
        if tokio::time::Instant::now() >= deadline {
            break;
        }

        let mut buffer = I420Buffer::new(width, height);
        buffer.fill_y(y_val);
        buffer.fill_uv(u_val, v_val);

        let timestamp_us = shiguredo_webrtc::time_millis() * 1000;
        let translated = aligner.translate(timestamp_us, timestamp_us);
        let frame = VideoFrame::from_i420(&buffer, translated, 0);
        source.on_frame(&frame);
    }
}
