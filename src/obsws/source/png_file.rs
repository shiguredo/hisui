use std::path::{Path, PathBuf};

use crate::{
    Error, ProcessorHandle, Result, TrackId,
    video::{FrameRate, VideoFormat, VideoFrame, VideoFrameSize, rgb_to_yuv_bt601_int},
};

const MAX_NOACKED_COUNT: u64 = 100;

#[derive(Debug, Clone)]
pub struct PngFileSource {
    pub path: PathBuf,
    pub frame_rate: FrameRate,
    pub output_video_track_id: TrackId,
}

#[derive(Debug)]
struct DecodedPngI420A {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

impl PngFileSource {
    pub async fn run(self, outer_processor: ProcessorHandle) -> Result<()> {
        let decoded = decode_png_to_i420a(&self.path)?;
        let mut tx = outer_processor
            .publish_track(self.output_video_track_id.clone())
            .await?;
        outer_processor.notify_ready();
        outer_processor.wait_subscribers_ready().await?;

        let mut frame_index = 0u64;
        let mut noacked_sent = 0u64;
        let start = tokio::time::Instant::now();
        let mut ack = tx.send_syn();
        loop {
            let timestamp = super::frames_to_timestamp(self.frame_rate, frame_index);
            tokio::time::sleep_until(start + timestamp).await;

            if noacked_sent > MAX_NOACKED_COUNT {
                ack.await;
                ack = tx.send_syn();
                noacked_sent = 0;
            }

            let frame = VideoFrame {
                data: decoded.data.clone(),
                format: VideoFormat::I420A,
                keyframe: true,
                size: Some(VideoFrameSize {
                    width: decoded.width,
                    height: decoded.height,
                }),
                timestamp,
                sample_entry: None,
            };

            if !tx.send_video(frame) {
                break;
            }
            noacked_sent = noacked_sent.saturating_add(1);
            frame_index = frame_index.saturating_add(1);
        }

        Ok(())
    }
}

fn decode_png_to_i420a(path: &Path) -> Result<DecodedPngI420A> {
    let png_bytes = std::fs::read(path).map_err(|e| {
        Error::new(format!(
            "failed to open input PNG file {}: {e}",
            path.display()
        ))
    })?;
    let (spec, pixels) = nopng::decode_image(&png_bytes).map_err(|e| {
        Error::new(format!(
            "failed to decode PNG image from {}: {e}",
            path.display()
        ))
    })?;

    let src_width = spec.width as usize;
    let src_height = spec.height as usize;
    let width = src_width - (src_width % 2);
    let height = src_height - (src_height % 2);
    if src_width != width || src_height != height {
        tracing::warn!(
            "odd PNG dimensions were truncated: {}x{} -> {}x{}",
            src_width,
            src_height,
            width,
            height
        );
    }
    if width == 0 || height == 0 {
        return Err(Error::new(format!(
            "PNG dimensions are too small after truncation: width={src_width} height={src_height}"
        )));
    }

    let (data, converted_width, converted_height) = match spec.pixel_format {
        nopng::PixelFormat::Rgb8 => {
            rgba_like_to_i420a(&pixels, src_width, src_height, width, height, 3)
        }
        nopng::PixelFormat::Rgba8 => {
            rgba_like_to_i420a(&pixels, src_width, src_height, width, height, 4)
        }
        nopng::PixelFormat::Gray8 => {
            grayscale_like_to_i420a(&pixels, src_width, src_height, width, height, 1)
        }
        nopng::PixelFormat::GrayAlpha8 => {
            grayscale_like_to_i420a(&pixels, src_width, src_height, width, height, 2)
        }
        ref other => {
            // サポート外のフォーマットは Rgba8 に変換してからデコードする
            let rgba = nopng::reformat_pixels(
                &spec.pixel_format,
                &pixels,
                &nopng::PixelFormat::Rgba8,
            )
            .map_err(|e| {
                Error::new(format!(
                    "unsupported PNG pixel format {other:?}, and failed to convert to RGBA8: {e}"
                ))
            })?;
            rgba_like_to_i420a(&rgba, src_width, src_height, width, height, 4)
        }
    }?;

    Ok(DecodedPngI420A {
        width: converted_width,
        height: converted_height,
        data,
    })
}

fn rgba_like_to_i420a(
    src: &[u8],
    src_width: usize,
    src_height: usize,
    width: usize,
    height: usize,
    channels: usize,
) -> Result<(Vec<u8>, usize, usize)> {
    let expected = src_width
        .checked_mul(src_height)
        .and_then(|v| v.checked_mul(channels))
        .ok_or_else(|| Error::new("PNG image size is too large"))?;
    if src.len() < expected {
        return Err(Error::new(format!(
            "insufficient PNG image data: expected at least {expected} bytes, got {}",
            src.len()
        )));
    }

    let y_size = width * height;
    let uv_width = width / 2;
    let uv_height = height / 2;
    let uv_size = uv_width * uv_height;
    let mut y_plane = vec![0u8; y_size];
    let mut u_plane = vec![0u8; uv_size];
    let mut v_plane = vec![0u8; uv_size];
    let mut a_plane = vec![0u8; y_size];

    for block_y in (0..height).step_by(2) {
        for block_x in (0..width).step_by(2) {
            let mut u_sum = 0u32;
            let mut v_sum = 0u32;

            for dy in 0..2 {
                for dx in 0..2 {
                    let x = block_x + dx;
                    let y = block_y + dy;
                    let pixel_index = (y * src_width + x) * channels;
                    let r = src[pixel_index];
                    let g = src[pixel_index + 1];
                    let b = src[pixel_index + 2];
                    let a = if channels == 4 {
                        src[pixel_index + 3]
                    } else {
                        u8::MAX
                    };

                    let (y_val, u_val, v_val) = rgb_to_yuv_bt601_int(r, g, b);
                    y_plane[y * width + x] = y_val;
                    a_plane[y * width + x] = a;
                    u_sum += u32::from(u_val);
                    v_sum += u32::from(v_val);
                }
            }

            let uv_index = (block_y / 2) * uv_width + (block_x / 2);
            u_plane[uv_index] = (u_sum / 4) as u8;
            v_plane[uv_index] = (v_sum / 4) as u8;
        }
    }

    let mut data = Vec::with_capacity(y_size * 2 + uv_size * 2);
    data.extend_from_slice(&y_plane);
    data.extend_from_slice(&u_plane);
    data.extend_from_slice(&v_plane);
    data.extend_from_slice(&a_plane);
    Ok((data, width, height))
}

fn grayscale_like_to_i420a(
    src: &[u8],
    src_width: usize,
    src_height: usize,
    width: usize,
    height: usize,
    channels: usize,
) -> Result<(Vec<u8>, usize, usize)> {
    let expected = src_width
        .checked_mul(src_height)
        .and_then(|v| v.checked_mul(channels))
        .ok_or_else(|| Error::new("PNG image size is too large"))?;
    if src.len() < expected {
        return Err(Error::new(format!(
            "insufficient PNG image data: expected at least {expected} bytes, got {}",
            src.len()
        )));
    }

    let y_size = width * height;
    let uv_width = width / 2;
    let uv_height = height / 2;
    let uv_size = uv_width * uv_height;
    let mut y_plane = vec![0u8; y_size];
    let u_plane = vec![128u8; uv_size];
    let v_plane = vec![128u8; uv_size];
    let mut a_plane = vec![0u8; y_size];

    for block_y in (0..height).step_by(2) {
        for block_x in (0..width).step_by(2) {
            for dy in 0..2 {
                for dx in 0..2 {
                    let x = block_x + dx;
                    let y = block_y + dy;
                    let pixel_index = (y * src_width + x) * channels;
                    y_plane[y * width + x] = src[pixel_index];
                    let alpha = if channels == 2 {
                        src[pixel_index + 1]
                    } else {
                        u8::MAX
                    };
                    a_plane[y * width + x] = alpha;
                }
            }
        }
    }

    let mut data = Vec::with_capacity(y_size * 2 + uv_size * 2);
    data.extend_from_slice(&y_plane);
    data.extend_from_slice(&u_plane);
    data.extend_from_slice(&v_plane);
    data.extend_from_slice(&a_plane);
    Ok((data, width, height))
}

/// source processor を起動できる設定が揃っているかを返す
pub(super) fn is_source_startable(
    settings: &crate::obsws::state::ObswsImageSourceSettings,
) -> bool {
    settings.file.is_some()
}

pub(super) fn build_record_source_plan(
    settings: &crate::obsws::state::ObswsImageSourceSettings,
    source_key: &str,
    frame_rate: crate::video::FrameRate,
) -> std::result::Result<super::ObswsRecordSourcePlan, super::BuildObswsRecordSourcePlanError> {
    let Some(path) = settings.file.as_deref() else {
        return Err(super::BuildObswsRecordSourcePlanError::InvalidInput(
            "inputSettings.file is required".to_owned(),
        ));
    };

    let source_processor_id = crate::ProcessorId::new(format!("input:png_source:{source_key}"));
    let source_video_track_id = crate::TrackId::new(format!("input:raw_video:{source_key}"));

    let source = PngFileSource {
        path: std::path::PathBuf::from(path),
        frame_rate,
        output_video_track_id: source_video_track_id.clone(),
    };

    Ok(super::ObswsRecordSourcePlan {
        source_processor_ids: vec![source_processor_id.clone()],
        source_video_track_id: Some(source_video_track_id),
        source_audio_track_id: None,
        requests: vec![super::ObswsSourceRequest::CreatePngFileSource {
            source,
            processor_id: Some(source_processor_id),
        }],
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{Error, MediaPipeline, Message, ProcessorId, ProcessorMetadata};

    #[test]
    fn decode_png_to_i420a_truncates_odd_size() -> crate::Result<()> {
        let data = [
            255, 0, 0, 0, 255, 0, 0, 0, 255, //
            255, 255, 0, 0, 255, 255, 255, 0, 255, //
            10, 20, 30, 40, 50, 60, 70, 80, 90, //
        ];
        let png_file = create_test_png_file(3, 3, nopng::PixelFormat::Rgb8, &data)?;
        let decoded = decode_png_to_i420a(png_file.path())?;

        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 2);
        assert_eq!(decoded.data.len(), 10);
        Ok(())
    }

    #[tokio::test]
    async fn png_file_source_run_sends_i420a_frames() -> crate::Result<()> {
        let png_file = create_test_png_file(2, 2, nopng::PixelFormat::Rgba8, &[255; 16])?;
        let pipeline = MediaPipeline::new()?;
        let pipeline_handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let output_track_id = TrackId::new("png-video");
        let subscriber = pipeline_handle
            .register_processor(
                ProcessorId::new("subscriber"),
                ProcessorMetadata::new("test_subscriber"),
            )
            .await?;
        let mut rx = subscriber.subscribe_track(output_track_id.clone());
        subscriber.notify_ready();
        assert!(
            pipeline_handle
                .trigger_start()
                .await
                .expect("trigger_start must succeed")
        );

        let source = PngFileSource {
            path: png_file.path().to_path_buf(),
            frame_rate: FrameRate::FPS_30,
            output_video_track_id: output_track_id,
        };
        let source_processor_id = ProcessorId::new("png_source");
        pipeline_handle
            .spawn_processor(
                source_processor_id.clone(),
                ProcessorMetadata::new("png_file_source"),
                |handle| source.run(handle),
            )
            .await?;

        let mut video_count = 0usize;
        let mut last_timestamp = Duration::ZERO;
        while video_count < 3 {
            let message = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .map_err(|e| Error::new(e.to_string()))?;
            if let Message::Media(crate::MediaFrame::Video(frame)) = message {
                assert_eq!(frame.format, VideoFormat::I420A);
                if video_count > 0 {
                    assert!(frame.timestamp >= last_timestamp);
                }
                last_timestamp = frame.timestamp;
                video_count += 1;
            }
        }

        drop(rx);
        drop(subscriber);
        let terminated = pipeline_handle
            .terminate_processor(source_processor_id)
            .await
            .map_err(|e| Error::new(e.to_string()))?;
        assert!(terminated, "png source processor must be terminated");
        drop(pipeline_handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .map_err(|e| Error::new(e.to_string()))?
            .map_err(|e| Error::new(e.to_string()))?;
        Ok(())
    }

    fn create_test_png_file(
        width: u32,
        height: u32,
        pixel_format: nopng::PixelFormat,
        data: &[u8],
    ) -> crate::Result<tempfile::NamedTempFile> {
        let spec = nopng::ImageSpec::new(width, height, pixel_format);
        let png_bytes = nopng::encode_image(&spec, data).map_err(|e| Error::new(e.to_string()))?;
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), &png_bytes)?;
        Ok(file)
    }
}
