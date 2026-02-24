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

impl nojson::DisplayJson for VideoDeviceSource {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("outputVideoTrackId", &self.output_video_track_id)?;
            if let Some(device_id) = &self.device_id {
                f.member("deviceId", device_id)?;
            }
            if let Some(width) = self.width {
                f.member("width", width)?;
            }
            if let Some(height) = self.height {
                f.member("height", height)?;
            }
            if let Some(fps) = self.fps {
                f.member("fps", fps)?;
            }
            Ok(())
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for VideoDeviceSource {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let output_video_track_id: TrackId = value
            .to_member("outputVideoTrackId")?
            .required()?
            .try_into()?;
        let device_id: Option<String> = value.to_member("deviceId")?.try_into()?;
        let width: Option<i32> = value.to_member("width")?.try_into()?;
        let height: Option<i32> = value.to_member("height")?.try_into()?;
        let fps: Option<i32> = value.to_member("fps")?.try_into()?;

        if let Some(device_id) = device_id.as_ref()
            && device_id.trim().is_empty()
        {
            return Err(value
                .to_member("deviceId")?
                .required()?
                .invalid("deviceId must not be empty"));
        }

        if width.is_some() != height.is_some() {
            return Err(value.invalid("width and height must be specified together"));
        }

        if let Some(width) = width
            && width <= 0
        {
            return Err(value
                .to_member("width")?
                .required()?
                .invalid("width must be greater than 0"));
        }

        if let Some(height) = height
            && height <= 0
        {
            return Err(value
                .to_member("height")?
                .required()?
                .invalid("height must be greater than 0"));
        }

        if let Some(fps) = fps
            && fps <= 0
        {
            return Err(value
                .to_member("fps")?
                .required()?
                .invalid("fps must be greater than 0"));
        }

        Ok(Self {
            output_video_track_id,
            device_id,
            width,
            height,
            fps,
        })
    }
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
        };

        let default_duration = frame_duration_from_fps(config.fps);
        let (frame_tx, mut frame_rx) =
            tokio::sync::mpsc::unbounded_channel::<shiguredo_video_device::VideoFrameOwned>();
        let mut capture = shiguredo_video_device::VideoCapture::new(config, move |frame| {
            let _ = frame_tx.send(frame.to_owned());
        })
        .map_err(|e| Error::new(format!("failed to create video capture session: {e}")))?;
        capture
            .start()
            .map_err(|e| Error::new(format!("failed to start video capture session: {e}")))?;

        let mut last_timestamp = None;
        while let Some(captured_frame) = frame_rx.recv().await {
            let frame = convert_captured_frame_to_i420(
                &captured_frame,
                default_duration,
                &mut last_timestamp,
            )?;
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

fn frame_duration_from_fps(fps: i32) -> Duration {
    let fps = u64::try_from(fps).unwrap_or(1);
    Duration::from_micros((1_000_000 / fps).max(1))
}

fn convert_captured_frame_to_i420(
    frame: &shiguredo_video_device::VideoFrameOwned,
    default_duration: Duration,
    last_timestamp: &mut Option<Duration>,
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
    let duration = match *last_timestamp {
        Some(prev) if timestamp > prev => timestamp.saturating_sub(prev),
        _ => default_duration,
    };
    *last_timestamp = Some(timestamp);

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

            let src = shiguredo_libyuv::Nv12Planes {
                y: &frame.data,
                y_stride,
                uv: uv_data,
                uv_stride,
            };
            let mut dst = shiguredo_libyuv::I420PlanesMut {
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
                width,
                height,
                timestamp,
                duration,
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
                width,
                height,
                timestamp,
                duration,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_device_source_requires_output_video_track_id() {
        let json = r#"{}"#;
        let result: crate::Result<VideoDeviceSource> = crate::json::parse_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn video_device_source_accepts_default_device() {
        let json = r#"{
            "outputVideoTrackId": "video-main"
        }"#;
        let source: VideoDeviceSource = crate::json::parse_str(json).expect("parse");

        assert_eq!(source.output_video_track_id.get(), "video-main");
        assert!(source.device_id.is_none());
        assert!(source.width.is_none());
        assert!(source.height.is_none());
        assert!(source.fps.is_none());
    }

    #[test]
    fn video_device_source_rejects_only_width() {
        let json = r#"{
            "outputVideoTrackId": "video-main",
            "width": 640
        }"#;
        let result: crate::Result<VideoDeviceSource> = crate::json::parse_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn video_device_source_rejects_zero_fps() {
        let json = r#"{
            "outputVideoTrackId": "video-main",
            "fps": 0
        }"#;
        let result: crate::Result<VideoDeviceSource> = crate::json::parse_str(json);
        assert!(result.is_err());
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
        };

        let default_duration = Duration::from_millis(33);
        let mut last_timestamp = None;
        let frame =
            convert_captured_frame_to_i420(&captured, default_duration, &mut last_timestamp)
                .expect("convert");

        assert_eq!(frame.format, crate::video::VideoFormat::I420);
        assert_eq!(frame.width, width);
        assert_eq!(frame.height, height);
        assert_eq!(frame.data, data);
        assert_eq!(frame.timestamp, Duration::from_secs(1));
        assert_eq!(frame.duration, default_duration);
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
        };

        let default_duration = Duration::from_millis(33);
        let mut last_timestamp = None;
        let error =
            convert_captured_frame_to_i420(&captured, default_duration, &mut last_timestamp)
                .expect_err("must fail");

        assert!(error.reason.contains("insufficient I420 data"));
    }
}
