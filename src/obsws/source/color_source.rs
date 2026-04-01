use crate::{
    ProcessorHandle, Result, TrackId,
    video::{FrameRate, VideoFormat, VideoFrame, VideoFrameSize, rgb_to_yuv_bt601_int},
};

use super::webrtc_source::parse_hex_color;

const MAX_NOACKED_COUNT: u64 = 100;

/// デフォルトの色（黒）
const DEFAULT_COLOR: &str = "#000000";

#[derive(Debug, Clone)]
pub struct ColorSource {
    pub color: String,
    pub frame_rate: FrameRate,
    pub output_video_track_id: TrackId,
}

impl ColorSource {
    pub async fn run(self, outer_processor: ProcessorHandle) -> Result<()> {
        let (r, g, b) = parse_hex_color(&self.color)
            .ok_or_else(|| crate::Error::new(format!("invalid color format: {}", self.color)))?;
        let (y, u, v) = rgb_to_yuv_bt601_int(r, g, b);

        // ソースサイズは SceneItem の transform で制御されるため、
        // ここでは最小限の 2x2 I420 フレームを生成する。
        // I420 は 4:2:0 サブサンプリングのため偶数サイズが必要。
        let width = 2;
        let height = 2;
        let y_size = width * height;
        let uv_size = (width / 2) * (height / 2);
        let mut data = vec![y; y_size];
        data.resize(y_size + uv_size, u);
        data.resize(y_size + uv_size * 2, v);

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
                data: data.clone(),
                format: VideoFormat::I420,
                keyframe: true,
                size: Some(VideoFrameSize { width, height }),
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

pub(super) fn build_record_source_plan(
    settings: &crate::obsws::input_registry::ObswsColorSourceSettings,
    output_kind: super::ObswsOutputKind,
    run_id: u64,
    source_key: &str,
    frame_rate: FrameRate,
) -> std::result::Result<super::ObswsRecordSourcePlan, super::BuildObswsRecordSourcePlanError> {
    let color = settings
        .color
        .as_deref()
        .unwrap_or(DEFAULT_COLOR)
        .to_owned();

    let source_processor_id = crate::ProcessorId::new(format!(
        "obsws:{}:{run_id}:source:{source_key}:color_source",
        output_kind.as_str()
    ));
    let source_video_track_id = crate::TrackId::new(format!(
        "obsws:{}:{run_id}:source:{source_key}:raw_video",
        output_kind.as_str()
    ));

    let source = ColorSource {
        color,
        frame_rate,
        output_video_track_id: source_video_track_id.clone(),
    };

    Ok(super::ObswsRecordSourcePlan {
        source_processor_ids: vec![source_processor_id.clone()],
        source_video_track_id: Some(source_video_track_id),
        source_audio_track_id: None,
        requests: vec![super::ObswsSourceRequest::CreateColorSource {
            source,
            processor_id: Some(source_processor_id),
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MediaFrame, MediaPipeline, Message, ProcessorId, ProcessorMetadata};

    #[tokio::test]
    async fn color_source_emits_i420_frames() -> crate::Result<()> {
        let pipeline = MediaPipeline::new()?;
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());
        {
            let handle = handle;
            let video_track_id = TrackId::new("color_source_test_video");
            let subscriber = handle
                .register_processor(
                    ProcessorId::new("test_subscriber"),
                    ProcessorMetadata::new("test_subscriber"),
                )
                .await?;
            let mut rx = subscriber.subscribe_track(video_track_id.clone());
            subscriber.notify_ready();
            assert!(
                handle
                    .trigger_start()
                    .await
                    .expect("trigger_start must succeed")
            );

            let source = ColorSource {
                color: "#FF0000".to_owned(),
                frame_rate: FrameRate::FPS_30,
                output_video_track_id: video_track_id.clone(),
            };
            handle
                .spawn_processor(
                    ProcessorId::new("color_source"),
                    ProcessorMetadata::new("color_source"),
                    |h| source.run(h),
                )
                .await?;

            // 最低 1 フレーム受信できることを確認する
            let mut received = 0;
            for _ in 0..5 {
                match rx.recv().await {
                    Message::Media(MediaFrame::Video(frame)) => {
                        assert_eq!(frame.format, VideoFormat::I420);
                        assert_eq!(
                            frame.size,
                            Some(VideoFrameSize {
                                width: 2,
                                height: 2
                            })
                        );
                        received += 1;
                        break;
                    }
                    Message::Eos => break,
                    _ => {}
                }
            }
            assert!(received > 0, "color_source must emit at least one frame");
        }

        pipeline_task.abort();
        Ok(())
    }

    #[test]
    fn build_record_source_plan_uses_default_color_when_none() {
        let settings = crate::obsws::input_registry::ObswsColorSourceSettings { color: None };
        let plan = build_record_source_plan(
            &settings,
            super::super::ObswsOutputKind::Program,
            1,
            "test",
            FrameRate::FPS_30,
        )
        .expect("build_record_source_plan must succeed");
        assert!(plan.source_video_track_id.is_some());
        assert!(plan.source_audio_track_id.is_none());
        assert_eq!(plan.requests.len(), 1);
    }

    #[test]
    fn build_record_source_plan_uses_specified_color() {
        let settings = crate::obsws::input_registry::ObswsColorSourceSettings {
            color: Some("#FF0000".to_owned()),
        };
        let plan = build_record_source_plan(
            &settings,
            super::super::ObswsOutputKind::Program,
            2,
            "red",
            FrameRate::FPS_30,
        )
        .expect("build_record_source_plan must succeed");
        assert!(plan.source_video_track_id.is_some());
        assert_eq!(plan.source_processor_ids.len(), 1);
    }
}
