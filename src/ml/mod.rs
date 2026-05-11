pub mod device;
pub mod yolo;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use crate::MediaFrame;
use crate::video::{RawVideoFrame, VideoFrame};

/// ML モデル（共有用）
pub enum MlModel {
    Detect(yolo::YoloV8),
    Pose(yolo::YoloV8Pose),
}

impl MlModel {
    pub fn forward(&self, xs: &candle_core::Tensor) -> candle_core::Result<candle_core::Tensor> {
        use candle_core::Module;
        match self {
            MlModel::Detect(m) => m.forward(xs),
            MlModel::Pose(m) => m.forward(xs),
        }
    }
}

/// ML 推論プロセッサ
///
/// 入力トラックの I420 ビデオフレームに YOLO 推論を適用し、結果を描画して出力トラックに送る。
/// カメラ入力・RTMP 入力など、I420 フレームを出力する任意のソースの後段に接続可能。
pub struct MlProcessor {
    pub input_track_id: crate::TrackId,
    pub output_track_id: crate::TrackId,
    pub model: Arc<MlModel>,
    pub model_size: usize,
    pub is_pose: bool,
    pub confidence: f32,
    pub nms: f32,
    pub device: candle_core::Device,
    pub running: Arc<AtomicBool>,
}

impl MlProcessor {
    pub async fn run(self, handle: crate::ProcessorHandle) -> crate::Result<()> {
        let mut input_rx = handle.subscribe_track(self.input_track_id);
        let output_track_id = self.output_track_id;
        let model = self.model;
        let model_size = self.model_size;
        let is_pose = self.is_pose;
        let confidence = self.confidence;
        let nms = self.nms;
        let device = self.device;
        let running = self.running;

        let mut output_tx = handle.publish_track(output_track_id).await?;
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        let mut frame_count: u64 = 0;
        let mut last_report = std::time::Instant::now();

        while running.load(Ordering::Relaxed) {
            let frame = match input_rx.recv().await {
                crate::Message::Media(MediaFrame::Video(f)) => f,
                crate::Message::Eos => {
                    output_tx.send_eos();
                    break;
                }
                _ => continue,
            };

            let output_frame = process_frame(
                &frame, &model, model_size, is_pose, confidence, nms, &device,
            )?;
            if !output_tx.send_video(output_frame) {
                break;
            }

            frame_count += 1;
            let elapsed = last_report.elapsed();
            if elapsed.as_secs() >= 1 {
                tracing::info!(
                    "ml frame={frame_count} fps={:.1}",
                    frame_count as f64 / elapsed.as_secs_f64(),
                );
                last_report = std::time::Instant::now();
            }
        }

        output_tx.send_eos();
        Ok(())
    }
}

/// 1 フレームの ML 処理を実行する
fn process_frame(
    frame: &Arc<VideoFrame>,
    model: &MlModel,
    model_size: usize,
    is_pose: bool,
    confidence: f32,
    nms: f32,
    device: &candle_core::Device,
) -> crate::Result<VideoFrame> {
    let raw = RawVideoFrame::from_i420_video_frame(Arc::clone(frame))?;
    let size = raw.size();
    let (y_plane, u_plane, v_plane) = raw.as_i420_planes()?;

    let (input_tensor, input_dims) = yolo::preprocess_i420(
        y_plane,
        u_plane,
        v_plane,
        size.width,
        size.height,
        model_size,
        device,
    )
    .map_err(|e| crate::Error::new(format!("preprocess error: {e}")))?;

    let output = model
        .forward(&input_tensor)
        .map_err(|e| crate::Error::new(format!("inference error: {e}")))?;

    // 検出があった場合のみプレーンデータをコピーして描画する
    if is_pose {
        let dets = yolo::postprocess_pose(&output, input_dims, confidence, nms)
            .map_err(|e| crate::Error::new(format!("postprocess error: {e}")))?;
        if !dets.is_empty() {
            let mut yc = y_plane.to_vec();
            let mut uc = u_plane.to_vec();
            let mut vc = v_plane.to_vec();
            yolo::draw_pose_on_i420(&mut yc, &mut uc, &mut vc, size.width, size.height, &dets);
            return build_video_frame_from_planes(
                &yc,
                &uc,
                &vc,
                size.width,
                size.height,
                frame.timestamp,
            );
        }
    } else {
        let dets = yolo::postprocess(&output, input_dims, confidence, nms)
            .map_err(|e| crate::Error::new(format!("postprocess error: {e}")))?;
        if !dets.is_empty() {
            let mut yc = y_plane.to_vec();
            let mut uc = u_plane.to_vec();
            let mut vc = v_plane.to_vec();
            yolo::draw_detections_on_i420(
                &mut yc,
                &mut uc,
                &mut vc,
                size.width,
                size.height,
                &dets,
            );
            return build_video_frame_from_planes(
                &yc,
                &uc,
                &vc,
                size.width,
                size.height,
                frame.timestamp,
            );
        }
    }

    // 検出なし: 元のフレームデータをそのまま出力
    build_video_frame_from_planes(
        y_plane,
        u_plane,
        v_plane,
        size.width,
        size.height,
        frame.timestamp,
    )
}

/// 3 つの I420 プレーンから VideoFrame を構築する
fn build_video_frame_from_planes(
    y_plane: &[u8],
    u_plane: &[u8],
    v_plane: &[u8],
    width: usize,
    height: usize,
    timestamp: std::time::Duration,
) -> crate::Result<VideoFrame> {
    let w = width;
    let h = height;
    let uv_width = w.div_ceil(2);
    let uv_height = h.div_ceil(2);

    let expected_y_size = w * h;
    let expected_uv_size = uv_width * uv_height;

    if y_plane.len() < expected_y_size {
        return Err(crate::Error::new(format!(
            "Y plane too small: expected {expected_y_size}, got {}",
            y_plane.len()
        )));
    }
    if u_plane.len() < expected_uv_size {
        return Err(crate::Error::new(format!(
            "U plane too small: expected {expected_uv_size}, got {}",
            u_plane.len()
        )));
    }
    if v_plane.len() < expected_uv_size {
        return Err(crate::Error::new(format!(
            "V plane too small: expected {expected_uv_size}, got {}",
            v_plane.len()
        )));
    }

    let mut data = Vec::with_capacity(expected_y_size + expected_uv_size * 2);
    data.extend_from_slice(&y_plane[..expected_y_size]);
    data.extend_from_slice(&u_plane[..expected_uv_size]);
    data.extend_from_slice(&v_plane[..expected_uv_size]);

    Ok(VideoFrame {
        data,
        format: crate::video::VideoFormat::I420,
        keyframe: false,
        size: Some(crate::video::VideoFrameSize::new(w, h)?),
        timestamp,
        sample_entry: None,
    })
}
