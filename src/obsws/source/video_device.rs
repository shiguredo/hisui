use std::time::Duration;

use crate::{Error, ProcessorHandle, Result, TrackId, VideoFrame};

#[derive(Debug, Clone)]
pub struct VideoDeviceSource {
    pub output_video_track_id: TrackId,
    pub device_id: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub fps: Option<i32>,
}

impl VideoDeviceSource {
    pub async fn run(self, handle: ProcessorHandle) -> Result<()> {
        let mut output_video_sender = handle
            .publish_track(self.output_video_track_id.clone())
            .await
            .map_err(|e| {
                Error::new(format!(
                    "failed to publish output video track {}: {e}",
                    self.output_video_track_id
                ))
            })?;
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        let default_config = shiguredo_video_device::VideoCaptureConfig::default();
        let config = shiguredo_video_device::VideoCaptureConfig {
            device_id: self.device_id.clone(),
            width: self.width.unwrap_or(default_config.width),
            height: self.height.unwrap_or(default_config.height),
            fps: self.fps.unwrap_or(default_config.fps),
            pixel_format: Some(shiguredo_video_device::PixelFormat::I420),
        };

        let (frame_tx, mut frame_rx) =
            tokio::sync::mpsc::unbounded_channel::<shiguredo_video_device::VideoFrameOwned>();
        let mut capture = shiguredo_video_device::VideoCapture::new(config, move |frame| {
            let _ = frame_tx.send(frame.to_owned());
        })
        .map_err(|e| Error::new(format!("failed to create video capture session: {e}")))?;
        capture
            .start()
            .map_err(|e| Error::new(format!("failed to start video capture session: {e}")))?;

        while let Some(captured_frame) = frame_rx.recv().await {
            let frame = convert_captured_frame_to_i420(&captured_frame)?;
            // TODO: send_syn() でペース調整に対応する
            if !output_video_sender.send_video(frame) {
                break;
            }
        }

        capture.stop();
        output_video_sender.send_eos();

        Ok(())
    }
}

fn convert_captured_frame_to_i420(
    frame: &shiguredo_video_device::VideoFrameOwned,
) -> Result<VideoFrame> {
    let width = usize::try_from(frame.width)
        .map_err(|_| Error::new(format!("invalid frame width: {}", frame.width)))?;
    let height = usize::try_from(frame.height)
        .map_err(|_| Error::new(format!("invalid frame height: {}", frame.height)))?;
    if width == 0 || height == 0 {
        return Err(Error::new(format!(
            "invalid frame size: {}x{}",
            frame.width, frame.height
        )));
    }

    let timestamp = if frame.timestamp_us <= 0 {
        Duration::ZERO
    } else {
        Duration::from_micros(frame.timestamp_us as u64)
    };

    match frame.pixel_format {
        shiguredo_video_device::PixelFormat::Nv12 => {
            let y_stride = usize::try_from(frame.stride)
                .map_err(|_| Error::new(format!("invalid Y stride: {}", frame.stride)))?;
            let uv_stride = usize::try_from(frame.stride_uv)
                .map_err(|_| Error::new(format!("invalid UV stride: {}", frame.stride_uv)))?;
            let uv_data = frame
                .uv_data
                .as_deref()
                .ok_or_else(|| Error::new("missing UV plane for NV12 frame"))?;

            let y_size = width * height;
            let uv_width = width.div_ceil(2);
            let uv_height = height.div_ceil(2);
            let uv_size = uv_width * uv_height;

            let mut i420_data = vec![0u8; y_size + uv_size * 2];
            let (y_plane, rest) = i420_data.split_at_mut(y_size);
            let (u_plane, v_plane) = rest.split_at_mut(uv_size);

            let src = shiguredo_libyuv::Nv12Image {
                y: &frame.data,
                y_stride,
                uv: uv_data,
                uv_stride,
            };
            let mut dst = shiguredo_libyuv::I420ImageMut {
                y: y_plane,
                y_stride: width,
                u: u_plane,
                u_stride: uv_width,
                v: v_plane,
                v_stride: uv_width,
            };
            shiguredo_libyuv::nv12_to_i420(
                &src,
                &mut dst,
                shiguredo_libyuv::ImageSize::new(width, height),
            )
            .map_err(|e| Error::new(format!("failed to convert NV12 to I420: {e}")))?;

            let input_frame = VideoFrame {
                data: Vec::new(),
                format: crate::video::VideoFormat::I420,
                keyframe: true,
                size: Some(crate::video::VideoFrameSize { width, height }),
                timestamp,
                sample_entry: None,
            };
            Ok(VideoFrame::new_i420(
                input_frame,
                width,
                height,
                y_plane,
                u_plane,
                v_plane,
                width,
                uv_width,
                uv_width,
            ))
        }
        shiguredo_video_device::PixelFormat::I420 => {
            let y_size = width * height;
            let uv_width = width.div_ceil(2);
            let uv_height = height.div_ceil(2);
            let uv_size = uv_width * uv_height;
            let expected_size = y_size + uv_size * 2;

            if frame.data.len() < expected_size {
                return Err(Error::new(format!(
                    "insufficient I420 data: expected at least {}, got {}",
                    expected_size,
                    frame.data.len()
                )));
            }

            let y_plane = &frame.data[..y_size];
            let u_plane = &frame.data[y_size..(y_size + uv_size)];
            let v_plane = &frame.data[(y_size + uv_size)..(y_size + uv_size * 2)];
            let input_frame = VideoFrame {
                data: Vec::new(),
                format: crate::video::VideoFormat::I420,
                keyframe: true,
                size: Some(crate::video::VideoFrameSize { width, height }),
                timestamp,
                sample_entry: None,
            };
            Ok(VideoFrame::new_i420(
                input_frame,
                width,
                height,
                y_plane,
                u_plane,
                v_plane,
                width,
                uv_width,
                uv_width,
            ))
        }
        shiguredo_video_device::PixelFormat::Yuy2 => {
            Err(Error::new("unsupported pixel format: YUY2"))
        }
        shiguredo_video_device::PixelFormat::Unknown(raw) => Err(Error::new(format!(
            "unsupported pixel format: unknown ({raw})"
        ))),
    }
}

pub(super) fn build_record_source_plan(
    settings: &crate::obsws::input_registry::ObswsVideoCaptureDeviceSettings,
    output_kind: super::ObswsOutputKind,
    run_id: u64,
    source_key: &str,
) -> std::result::Result<super::ObswsRecordSourcePlan, super::BuildObswsRecordSourcePlanError> {
    let kind = output_kind.as_str();
    let source_processor_id = crate::ProcessorId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_key}:video_device_source"
    ));
    let raw_video_track_id = crate::TrackId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_key}:raw_video"
    ));

    let source = VideoDeviceSource {
        output_video_track_id: raw_video_track_id.clone(),
        device_id: settings.device_id.clone(),
        width: None,
        height: None,
        fps: None,
    };

    Ok(super::ObswsRecordSourcePlan {
        source_processor_ids: vec![source_processor_id.clone()],
        source_video_track_id: Some(raw_video_track_id),
        source_audio_track_id: None,
        requests: vec![super::ObswsSourceRequest::CreateVideoDeviceSource {
            source,
            processor_id: Some(source_processor_id),
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obsws::input_registry::ObswsVideoCaptureDeviceSettings;
    use crate::obsws::source::{ObswsOutputKind, ObswsSourceRequest};

    #[test]
    fn build_record_source_plan_with_device_id() {
        let plan = build_record_source_plan(
            &ObswsVideoCaptureDeviceSettings {
                device_id: Some("camera0".to_owned()),
            },
            ObswsOutputKind::Program,
            1,
            "0",
        )
        .expect("video_capture_device source plan must succeed");

        assert_eq!(plan.source_processor_ids.len(), 1);
        assert_eq!(
            plan.source_processor_ids[0].get(),
            "obsws:record:1:source:0:video_device_source"
        );

        assert_eq!(plan.requests.len(), 1);

        assert_eq!(
            plan.source_video_track_id.as_ref().map(|t| t.get()),
            Some("obsws:record:1:source:0:raw_video")
        );
        assert!(plan.source_audio_track_id.is_none());

        match &plan.requests[0] {
            ObswsSourceRequest::CreateVideoDeviceSource {
                source,
                processor_id,
            } => {
                assert_eq!(
                    source.output_video_track_id.get(),
                    "obsws:record:1:source:0:raw_video"
                );
                assert_eq!(source.device_id.as_deref(), Some("camera0"));
                assert_eq!(
                    processor_id.as_ref().map(|p| p.get()),
                    Some("obsws:record:1:source:0:video_device_source")
                );
            }
            _ => panic!("expected CreateVideoDeviceSource"),
        }
    }

    #[test]
    fn build_record_source_plan_without_device_id() {
        let plan = build_record_source_plan(
            &ObswsVideoCaptureDeviceSettings { device_id: None },
            ObswsOutputKind::Program,
            2,
            "1",
        )
        .expect("video_capture_device source plan without device_id must succeed");

        match &plan.requests[0] {
            ObswsSourceRequest::CreateVideoDeviceSource { source, .. } => {
                assert_eq!(source.device_id, None);
            }
            _ => panic!("expected CreateVideoDeviceSource"),
        }
    }

    #[test]
    fn convert_captured_frame_to_i420_accepts_i420_input() {
        let width = 4usize;
        let height = 2usize;
        let y_size = width * height;
        let uv_width = width.div_ceil(2);
        let uv_height = height.div_ceil(2);
        let uv_size = uv_width * uv_height;
        let expected_size = y_size + uv_size * 2;
        let data = (0u8..(expected_size as u8)).collect::<Vec<_>>();

        let captured = shiguredo_video_device::VideoFrameOwned {
            data: data.clone(),
            uv_data: None,
            width: width as i32,
            height: height as i32,
            stride: width as i32,
            stride_uv: uv_width as i32,
            pixel_format: shiguredo_video_device::PixelFormat::I420,
            timestamp_us: 1_000_000,
            pixel_buffer: None,
        };

        let frame = convert_captured_frame_to_i420(&captured).expect("convert");

        assert_eq!(frame.format, crate::video::VideoFormat::I420);
        let size = frame.size().expect("infallible");
        assert_eq!(size.width, width);
        assert_eq!(size.height, height);
        assert_eq!(frame.data, data);
        assert_eq!(frame.timestamp, Duration::from_secs(1));
    }

    #[test]
    fn convert_captured_frame_to_i420_rejects_short_i420_input() {
        let width = 4usize;
        let height = 2usize;
        let y_size = width * height;
        let uv_width = width.div_ceil(2);
        let uv_height = height.div_ceil(2);
        let uv_size = uv_width * uv_height;
        let expected_size = y_size + uv_size * 2;

        let captured = shiguredo_video_device::VideoFrameOwned {
            data: vec![0; expected_size - 1],
            uv_data: None,
            width: width as i32,
            height: height as i32,
            stride: width as i32,
            stride_uv: uv_width as i32,
            pixel_format: shiguredo_video_device::PixelFormat::I420,
            timestamp_us: 1_000_000,
            pixel_buffer: None,
        };

        let error = convert_captured_frame_to_i420(&captured).expect_err("must fail");

        assert!(error.reason.contains("insufficient I420 data"));
    }
}
