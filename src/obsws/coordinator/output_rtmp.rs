//! RTMP outbound の output エンジン。
//! Program 出力を指定された RTMP エンドポイントに再配信するための processor 起動・停止を行う。

use super::ObswsCoordinator;
use super::output::{OutputOperationOutcome, terminate_and_wait};
use super::output_registry::{OutputRun, OutputSettings};

impl ObswsCoordinator {
    /// 指定された output_name の rtmp_outbound output を開始する。
    pub(crate) async fn handle_start_rtmp_outbound(
        &mut self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> OutputOperationOutcome {
        use crate::obsws::state::{ObswsRecordTrackRun, ObswsRtmpOutboundRun};

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
        let OutputSettings::RtmpOutbound(rtmp_settings) = &output.settings else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Output is not an rtmp_outbound output",
                ),
            );
        };
        let rtmp_settings = rtmp_settings.clone();

        if output.runtime.active {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_RUNNING,
                    "RTMP outbound is already active",
                ),
            );
        }

        let Some(output_url) = rtmp_settings.output_url else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Missing outputSettings.outputUrl field",
                ),
            );
        };

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
        let run = ObswsRtmpOutboundRun {
            video,
            audio,
            endpoint_processor_id: crate::ProcessorId::new(format!(
                "output:{output_name}:endpoint:{run_id}"
            )),
        };

        if let Some(output) = self.outputs.get_mut(output_name) {
            output.runtime.active = true;
            output.runtime.started_at = Some(std::time::Instant::now());
            output.runtime.run = Some(OutputRun::RtmpOutbound(run.clone()));
        }

        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
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
        let frame_rate = self.state.frame_rate();
        if let Err(e) = start_rtmp_outbound_processors(
            pipeline_handle,
            &output_url,
            rtmp_settings.stream_name.as_deref(),
            &run,
            frame_rate,
        )
        .await
        {
            if let Some(output) = self.outputs.get_mut(output_name) {
                output.runtime.active = false;
                output.runtime.started_at = None;
                output.runtime.run = None;
            }
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

    /// 指定された output_name の rtmp_outbound output を停止する。
    pub(crate) async fn handle_stop_rtmp_outbound(
        &mut self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> OutputOperationOutcome {
        let run = self
            .outputs
            .get(output_name)
            .and_then(|o| o.runtime.run.as_ref())
            .and_then(|r| match r {
                OutputRun::RtmpOutbound(run) => Some(run.clone()),
                _ => None,
            });
        let Some(run) = run else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                    "RTMP outbound is not active",
                ),
            );
        };
        if let Some(pipeline_handle) = self.pipeline_handle.as_ref()
            && let Err(e) = stop_processors_staged_rtmp_outbound(pipeline_handle, &run).await
        {
            tracing::warn!("failed to stop rtmp outbound processors: {}", e.display());
        }
        if let Some(output) = self.outputs.get_mut(output_name) {
            output.runtime.active = false;
            output.runtime.started_at = None;
            output.runtime.run = None;
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_output_response(request_id),
            None,
        )
    }
}

// -----------------------------------------------------------------------
// RtmpOutboundOutputSettings: rtmp_outbound output の種別固有設定
// -----------------------------------------------------------------------

/// RTMP outbound output の設定。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct RtmpOutboundOutputSettings {
    pub(crate) output_url: Option<String>,
    pub(crate) stream_name: Option<String>,
}

impl From<crate::obsws::state::ObswsRtmpOutboundSettings> for RtmpOutboundOutputSettings {
    fn from(s: crate::obsws::state::ObswsRtmpOutboundSettings) -> Self {
        Self {
            output_url: s.output_url,
            stream_name: s.stream_name,
        }
    }
}

impl From<RtmpOutboundOutputSettings> for crate::obsws::state::ObswsRtmpOutboundSettings {
    fn from(s: RtmpOutboundOutputSettings) -> Self {
        Self {
            output_url: s.output_url,
            stream_name: s.stream_name,
        }
    }
}

impl nojson::DisplayJson for RtmpOutboundOutputSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(output_url) = &self.output_url {
                f.member("outputUrl", output_url)?;
            }
            if let Some(stream_name) = &self.stream_name {
                f.member("streamName", stream_name)?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

impl RtmpOutboundOutputSettings {
    /// JSON から設定を更新する（SetOutputSettings 用）。
    /// 各フィールドは「キーが存在し値が non-null」なら更新、「値が null」なら None にクリア、
    /// 「キーが存在しない」なら既存値を維持する。
    pub(crate) fn update_from_json(
        &mut self,
        output_settings: &nojson::RawJsonValue<'_, '_>,
    ) -> Result<(), String> {
        if let Ok(v) = output_settings.to_member("outputUrl")
            && let Some(v) = v.optional()
        {
            if v.kind().is_null() {
                self.output_url = None;
            } else {
                match <String>::try_from(v) {
                    Ok(url) => self.output_url = Some(url),
                    Err(_) => return Err("outputUrl must be a string".to_owned()),
                }
            }
        }
        if let Ok(v) = output_settings.to_member("streamName")
            && let Some(v) = v.optional()
        {
            if v.kind().is_null() {
                self.stream_name = None;
            } else {
                match <String>::try_from(v) {
                    Ok(name) => self.stream_name = Some(name),
                    Err(_) => return Err("streamName must be a string".to_owned()),
                }
            }
        }
        Ok(())
    }

    /// JSON から設定をパースする（HisuiCreateOutput / state file 復元用）。
    pub(crate) fn parse_from_json(
        settings_value: Option<&nojson::RawJsonValue<'_, '_>>,
    ) -> Result<Self, String> {
        use super::output_registry::parse_optional_string_strict;

        let mut settings = Self::default();
        if let Some(v) = settings_value {
            settings.output_url =
                parse_optional_string_strict(v, "outputUrl", "outputUrl must be a string")?;
            settings.stream_name =
                parse_optional_string_strict(v, "streamName", "streamName must be a string")?;
        }
        Ok(settings)
    }
}

/// RTMP outbound 用プロセッサを起動する: エンコーダー → RTMP エンドポイント
async fn start_rtmp_outbound_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    output_url: &str,
    stream_name: Option<&str>,
    run: &crate::obsws::state::ObswsRtmpOutboundRun,
    frame_rate: crate::video::FrameRate,
) -> crate::Result<()> {
    // RTMP outbound は AAC エンコーディングを使用する（RTMP の制約）
    super::output::start_encoder_processors(
        pipeline_handle,
        &run.video,
        &run.audio,
        crate::types::CodecName::Aac,
        frame_rate,
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
    run: &crate::obsws::state::ObswsRtmpOutboundRun,
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
