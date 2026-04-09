//! Stream (RTMP 配信) の output エンジン。
//! Program 出力を RTMP でライブ配信するための processor 起動・停止を行う。

use super::ObswsCoordinator;
use super::output::{OutputOperationOutcome, terminate_and_wait};
use super::output_dynamic::{OutputRun, OutputSettings};

impl ObswsCoordinator {
    /// 指定された output_name の stream output を開始する。
    /// outputs BTreeMap から設定・ランタイム状態を読み書きする。
    pub(crate) async fn handle_start_stream(
        &mut self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> OutputOperationOutcome {
        use crate::obsws::input_registry::{ObswsRecordTrackRun, ObswsStreamRun};

        // output の存在チェックと設定取得
        let Some(output) = self.outputs.get(output_name) else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Output not found",
                ),
            );
        };
        let OutputSettings::Stream(stream_settings) = &output.settings else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Output is not a stream output",
                ),
            );
        };
        let stream_settings = stream_settings.clone();

        // 稼働中チェック
        if output.runtime.active {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_STREAM_RUNNING,
                    "Stream is already active",
                ),
            );
        }

        // rtmp_custom 以外は非対応
        if stream_settings.stream_service_type != "rtmp_custom" {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Unsupported streamServiceType field",
                ),
            );
        }
        let Some(output_url) = stream_settings.server else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Missing streamServiceSettings.server field",
                ),
            );
        };

        // run_id の発行
        let run_id = self.next_output_run_id;
        self.next_output_run_id = self.next_output_run_id.wrapping_add(1);

        let video = ObswsRecordTrackRun::new(
            output_name,
            run_id,
            "video",
            &self.program_output.video_track_id,
        );
        let audio = ObswsRecordTrackRun::new(
            output_name,
            run_id,
            "audio",
            &self.program_output.audio_track_id,
        );
        let run = ObswsStreamRun {
            video,
            audio,
            publisher_processor_id: crate::ProcessorId::new(format!(
                "output:{output_name}:rtmp_publisher:{run_id}"
            )),
        };

        // ランタイム状態を active にする
        if let Some(output) = self.outputs.get_mut(output_name) {
            output.runtime.active = true;
            output.runtime.started_at = Some(std::time::Instant::now());
            output.runtime.run = Some(OutputRun::Stream(run.clone()));
        }

        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            // ロールバック
            if let Some(output) = self.outputs.get_mut(output_name) {
                output.runtime.active = false;
                output.runtime.started_at = None;
                output.runtime.run = None;
            }
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
            stream_settings.key.as_deref(),
            &run,
            frame_rate,
        )
        .await
        {
            // ロールバック
            if let Some(output) = self.outputs.get_mut(output_name) {
                output.runtime.active = false;
                output.runtime.started_at = None;
                output.runtime.run = None;
            }
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

    /// 指定された output_name の stream output を停止する。
    pub(crate) async fn handle_stop_stream(
        &mut self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> OutputOperationOutcome {
        // output から run を取得
        let run = self
            .outputs
            .get(output_name)
            .and_then(|o| o.runtime.run.as_ref())
            .and_then(|r| match r {
                OutputRun::Stream(run) => Some(run.clone()),
                _ => None,
            });
        let Some(run) = run else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_STREAM_NOT_RUNNING,
                    "Stream is not active",
                ),
            );
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
        // ランタイム状態をリセット
        if let Some(output) = self.outputs.get_mut(output_name) {
            output.runtime.active = false;
            output.runtime.started_at = None;
            output.runtime.run = None;
        }
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
