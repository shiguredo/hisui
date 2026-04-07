//! Stream (RTMP 配信) の output エンジン。
//! Program 出力を RTMP でライブ配信するための processor 起動・停止を行う。

use super::ObswsCoordinator;
use super::output::{OutputOperationOutcome, terminate_and_wait};

impl ObswsCoordinator {
    pub(crate) async fn handle_start_stream(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        use crate::obsws::input_registry::{
            ActivateStreamError, ObswsRecordTrackRun, ObswsStreamRun,
        };
        let stream_service_settings = self.input_registry.stream_service_settings().clone();
        // `stream` は OBS 互換の主配信 Output として扱い、現状は RTMP 起動経路に限定する。
        // `sora` は stream service の差し替えではなく別 Output として扱うため、
        // ここでは `rtmp_custom` 以外を受け付けない。
        if stream_service_settings.stream_service_type != "rtmp_custom" {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Unsupported streamServiceType field",
                ),
            );
        }
        let Some(output_url) = stream_service_settings.server else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Missing streamServiceSettings.server field",
                ),
            );
        };
        let run_id = match self.input_registry.next_stream_run_id() {
            Ok(run_id) => run_id,
            Err(_) => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        "Stream run ID overflow",
                    ),
                );
            }
        };
        let video = ObswsRecordTrackRun::new(
            "stream",
            run_id,
            "video",
            &self.program_output.video_track_id,
        );
        let audio = ObswsRecordTrackRun::new(
            "stream",
            run_id,
            "audio",
            &self.program_output.audio_track_id,
        );
        let run = ObswsStreamRun {
            video,
            audio,
            publisher_processor_id: crate::ProcessorId::new(format!(
                "output:stream:rtmp_publisher:{run_id}"
            )),
        };
        if let Err(ActivateStreamError::AlreadyActive) =
            self.input_registry.activate_stream(run.clone())
        {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_STREAM_RUNNING,
                    "Stream is already active",
                ),
            );
        }
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            self.input_registry.deactivate_stream();
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
        if let Err(e) = start_stream_processors(
            pipeline_handle,
            &output_url,
            stream_service_settings.key.as_deref(),
            &run,
            frame_rate,
        )
        .await
        {
            self.input_registry.deactivate_stream();
            let _ = stop_processors_staged_stream(pipeline_handle, &run).await;
            let error_comment = format!("Failed to start stream: {}", e.display());
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
            crate::obsws::response::build_start_stream_response(request_id),
            None,
        )
    }

    pub(crate) async fn handle_stop_stream(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        let run = match self.input_registry.stream_run() {
            Some(run) => run.clone(),
            None => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_STREAM_NOT_RUNNING,
                        "Stream is not active",
                    ),
                );
            }
        };
        if let Some(pipeline_handle) = self.pipeline_handle.as_ref()
            && let Err(e) = stop_processors_staged_stream(pipeline_handle, &run).await
        {
            let error_comment = format!("Failed to stop stream: {}", e.display());
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &error_comment,
                ),
            );
        }
        self.input_registry.deactivate_stream();
        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_stream_response(request_id),
            None,
        )
    }
}

/// ストリーム用プロセッサを起動する: エンコーダー → パブリッシャー
/// program mixer の出力トラックを直接エンコーダーに入力するため、ミキサーとソースの起動は不要。
async fn start_stream_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    output_url: &str,
    stream_key: Option<&str>,
    run: &crate::obsws::input_registry::ObswsStreamRun,
    frame_rate: crate::video::FrameRate,
) -> crate::Result<()> {
    super::output::start_encoder_processors(
        pipeline_handle,
        &run.video,
        &run.audio,
        crate::types::CodecName::Aac,
        frame_rate,
    )
    .await?;
    // RTMP パブリッシャーを起動する
    let publisher = crate::rtmp::publisher::RtmpPublisher {
        output_url: output_url.to_owned(),
        stream_name: stream_key.map(|s| s.to_owned()),
        input_audio_track_id: Some(run.audio.encoded_track_id.clone()),
        input_video_track_id: Some(run.video.encoded_track_id.clone()),
        options: Default::default(),
    };
    crate::rtmp::publisher::create_processor(
        pipeline_handle,
        publisher,
        Some(run.publisher_processor_id.clone()),
    )
    .await?;
    Ok(())
}

/// ストリーム用プロセッサを段階的に停止する: エンコーダー → パブリッシャー
async fn stop_processors_staged_stream(
    pipeline_handle: &crate::MediaPipelineHandle,
    run: &crate::obsws::input_registry::ObswsStreamRun,
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
        std::slice::from_ref(&run.publisher_processor_id),
    )
    .await?;
    Ok(())
}
