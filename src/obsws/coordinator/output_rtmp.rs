//! RTMP outbound の output エンジン。
//! Program 出力を指定された RTMP エンドポイントに再配信するための processor 起動・停止を行う。

use super::ObswsCoordinator;
use super::output::{OutputOperationOutcome, terminate_and_wait};

impl ObswsCoordinator {
    pub(crate) async fn handle_start_rtmp_outbound(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        use crate::obsws::input_registry::{
            ActivateRtmpOutboundError, ObswsRecordTrackRun, ObswsRtmpOutboundRun,
        };
        let rtmp_outbound_settings = self.input_registry.rtmp_outbound_settings().clone();
        let Some(output_url) = rtmp_outbound_settings.output_url else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Missing outputSettings.outputUrl field",
                ),
            );
        };
        let run_id = match self.input_registry.next_rtmp_outbound_run_id() {
            Ok(run_id) => run_id,
            Err(_) => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        "RTMP outbound run ID overflow",
                    ),
                );
            }
        };
        let video = ObswsRecordTrackRun::new(
            "rtmp_outbound",
            run_id,
            "video",
            &self.program_output.video_track_id,
        );
        let audio = ObswsRecordTrackRun::new(
            "rtmp_outbound",
            run_id,
            "audio",
            &self.program_output.audio_track_id,
        );
        let run = ObswsRtmpOutboundRun {
            video,
            audio,
            endpoint_processor_id: crate::ProcessorId::new(format!(
                "obsws:rtmp_outbound:{run_id}:rtmp_outbound_endpoint"
            )),
        };
        if let Err(ActivateRtmpOutboundError::AlreadyActive) =
            self.input_registry.activate_rtmp_outbound(run.clone())
        {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_RUNNING,
                    "RTMP outbound is already active",
                ),
            );
        }
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            self.input_registry.deactivate_rtmp_outbound();
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    "Pipeline is not initialized",
                ),
            );
        };
        let frame_rate = self.input_registry.frame_rate();
        if let Err(e) = start_rtmp_outbound_processors(
            pipeline_handle,
            &output_url,
            rtmp_outbound_settings.stream_name.as_deref(),
            &run,
            frame_rate,
        )
        .await
        {
            self.input_registry.deactivate_rtmp_outbound();
            let _ = stop_processors_staged_rtmp_outbound(pipeline_handle, &run).await;
            let error_comment = format!("Failed to start rtmp_outbound: {}", e.display());
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &error_comment,
                ),
            );
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_start_output_response(request_id),
            None,
        )
    }

    pub(crate) async fn handle_stop_rtmp_outbound(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        let run = match self.input_registry.rtmp_outbound_run() {
            Some(run) => run.clone(),
            None => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                        "RTMP outbound is not active",
                    ),
                );
            }
        };
        if let Some(pipeline_handle) = self.pipeline_handle.as_ref()
            && let Err(e) = stop_processors_staged_rtmp_outbound(pipeline_handle, &run).await
        {
            tracing::warn!("failed to stop rtmp outbound processors: {}", e.display());
        }
        self.input_registry.deactivate_rtmp_outbound();
        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_output_response(request_id),
            None,
        )
    }
}

/// RTMP outbound 用プロセッサを起動する: エンコーダー → RTMP エンドポイント
/// program mixer の出力トラックを直接エンコーダーに入力するため、ミキサーとソースの起動は不要。
async fn start_rtmp_outbound_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    output_url: &str,
    stream_name: Option<&str>,
    run: &crate::obsws::input_registry::ObswsRtmpOutboundRun,
    frame_rate: crate::video::FrameRate,
) -> crate::Result<()> {
    crate::encoder::create_video_processor(
        pipeline_handle,
        run.video.source_track_id.clone(),
        run.video.encoded_track_id.clone(),
        crate::types::CodecName::H264,
        std::num::NonZeroUsize::new(2_000_000).expect("non-zero constant"),
        frame_rate,
        Some(run.video.encoder_processor_id.clone()),
    )
    .await?;
    // RTMP outbound は AAC エンコーディングを使用する（RTMP の制約）
    crate::encoder::create_audio_processor(
        pipeline_handle,
        run.audio.source_track_id.clone(),
        run.audio.encoded_track_id.clone(),
        crate::types::CodecName::Aac,
        std::num::NonZeroUsize::new(128_000).expect("non-zero constant"),
        Some(run.audio.encoder_processor_id.clone()),
    )
    .await?;
    let endpoint = crate::rtmp::outbound_endpoint::RtmpOutboundEndpoint {
        output_url: output_url.to_owned(),
        stream_name: stream_name.map(|s| s.to_owned()),
        input_audio_track_id: Some(run.audio.encoded_track_id.clone()),
        input_video_track_id: Some(run.video.encoded_track_id.clone()),
        options: Default::default(),
    };
    crate::rtmp::outbound_endpoint::create_processor(
        pipeline_handle,
        endpoint,
        Some(run.endpoint_processor_id.clone()),
    )
    .await?;
    Ok(())
}

/// RTMP outbound 用プロセッサを段階的に停止する: エンコーダー → エンドポイント
async fn stop_processors_staged_rtmp_outbound(
    pipeline_handle: &crate::MediaPipelineHandle,
    run: &crate::obsws::input_registry::ObswsRtmpOutboundRun,
) -> crate::Result<()> {
    terminate_and_wait(
        pipeline_handle,
        &[
            run.video.encoder_processor_id.clone(),
            run.audio.encoder_processor_id.clone(),
        ],
    )
    .await?;
    terminate_and_wait(
        pipeline_handle,
        std::slice::from_ref(&run.endpoint_processor_id),
    )
    .await?;
    Ok(())
}
