use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use shiguredo_websocket::CloseCode;
use tokio::sync::RwLock;

use crate::obsws_auth::ObswsAuthentication;
use crate::obsws_input_registry::{
    ActivateRecordError, ActivateStreamError, ObswsInputRegistry, ObswsInputSettings,
    ObswsRecordRun, ObswsStreamRun,
};
use crate::obsws_message::{ClientMessage, ObswsSessionStats};
use crate::obsws_protocol::{
    OBSWS_CLOSE_ALREADY_IDENTIFIED, OBSWS_CLOSE_AUTHENTICATION_FAILED, OBSWS_CLOSE_NOT_IDENTIFIED,
    OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION, REQUEST_STATUS_INVALID_REQUEST_FIELD,
    REQUEST_STATUS_MISSING_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_TYPE,
    REQUEST_STATUS_OUTPUT_NOT_RUNNING, REQUEST_STATUS_OUTPUT_RUNNING,
    REQUEST_STATUS_STREAM_NOT_RUNNING, REQUEST_STATUS_STREAM_RUNNING,
};

pub enum SessionAction {
    SendText {
        text: String,
        message_name: &'static str,
    },
    Close {
        code: CloseCode,
        reason: &'static str,
        close_error_context: &'static str,
    },
    Terminate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObswsSessionState {
    AwaitingIdentify,
    Identified,
}

pub struct ObswsSession {
    state: ObswsSessionState,
    auth: Option<ObswsAuthentication>,
    input_registry: Arc<RwLock<ObswsInputRegistry>>,
    pipeline_handle: Option<crate::MediaPipelineHandle>,
    stats: ObswsSessionStats,
}

impl ObswsSession {
    pub fn new(
        auth: Option<ObswsAuthentication>,
        input_registry: Arc<RwLock<ObswsInputRegistry>>,
        pipeline_handle: Option<crate::MediaPipelineHandle>,
    ) -> Self {
        Self {
            state: ObswsSessionState::AwaitingIdentify,
            auth,
            input_registry,
            pipeline_handle,
            stats: ObswsSessionStats::default(),
        }
    }

    pub fn stats_mut(&mut self) -> &mut ObswsSessionStats {
        &mut self.stats
    }

    pub fn on_connected(&self) -> SessionAction {
        SessionAction::SendText {
            text: crate::obsws_message::build_hello_message(self.auth.as_ref()),
            message_name: "hello message",
        }
    }

    pub async fn on_text_message(&mut self, text: &str) -> crate::Result<SessionAction> {
        self.stats.incoming_messages = self.stats.incoming_messages.saturating_add(1);

        let message = crate::obsws_message::parse_client_message(text)?;
        let action = match message {
            ClientMessage::Identify(identify) => self.handle_identify(identify),
            ClientMessage::Request(request) => self.handle_request(request).await,
        };
        Ok(action)
    }

    pub fn on_close_event(&self) -> SessionAction {
        SessionAction::Terminate
    }

    pub fn on_error_event(&self) -> SessionAction {
        SessionAction::Terminate
    }

    fn build_internal_error_response(
        request_type: &str,
        request_id: &str,
        message: &str,
    ) -> String {
        // [NOTE]
        // 現在 hisui が実装している obsws requestStatus code では、
        // サーバー内部エラー専用のコードを定義していない。
        // そのため互換性を保ったまま 400 を内部エラーにも流用している。
        crate::obsws_response_builder::build_request_response_error(
            request_type,
            request_id,
            REQUEST_STATUS_INVALID_REQUEST_FIELD,
            message,
        )
    }

    fn handle_identify(
        &mut self,
        identify: crate::obsws_message::IdentifyMessage,
    ) -> SessionAction {
        if self.state == ObswsSessionState::Identified {
            return SessionAction::Close {
                code: OBSWS_CLOSE_ALREADY_IDENTIFIED,
                reason: "already identified",
                close_error_context: "failed to close websocket for duplicated identify",
            };
        }

        if !crate::obsws_message::is_supported_rpc_version(identify.rpc_version) {
            return SessionAction::Close {
                code: OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION,
                reason: "unsupported rpc version",
                close_error_context: "failed to close websocket for unsupported rpc version",
            };
        }

        if let Some(auth) = self.auth.as_ref()
            && !auth.is_valid_response(identify.authentication.as_deref())
        {
            return SessionAction::Close {
                code: OBSWS_CLOSE_AUTHENTICATION_FAILED,
                reason: "authentication failed",
                close_error_context: "failed to close websocket for authentication failure",
            };
        }

        self.state = ObswsSessionState::Identified;
        SessionAction::SendText {
            text: crate::obsws_message::build_identified_message(identify.rpc_version),
            message_name: "identified message",
        }
    }

    async fn handle_request(
        &mut self,
        request: crate::obsws_message::RequestMessage,
    ) -> SessionAction {
        if self.state != ObswsSessionState::Identified {
            return SessionAction::Close {
                code: OBSWS_CLOSE_NOT_IDENTIFIED,
                reason: "identify is required",
                close_error_context: "failed to close websocket for unidentified request",
            };
        }

        let request_id = request.request_id.clone().unwrap_or_default();
        let request_type = request.request_type.clone().unwrap_or_default();
        if request_id.is_empty() {
            return SessionAction::SendText {
                text: crate::obsws_response_builder::build_request_response_error(
                    &request_type,
                    &request_id,
                    REQUEST_STATUS_MISSING_REQUEST_FIELD,
                    "Missing required requestId field",
                ),
                message_name: "request response message",
            };
        }
        if request_type.is_empty() {
            return SessionAction::SendText {
                text: crate::obsws_response_builder::build_request_response_error(
                    &request_type,
                    &request_id,
                    REQUEST_STATUS_MISSING_REQUEST_TYPE,
                    "Missing required requestType field",
                ),
                message_name: "request response message",
            };
        }

        if request_type == "StartStream" {
            return SessionAction::SendText {
                text: self.handle_start_stream(&request_id).await,
                message_name: "request response message",
            };
        }
        if request_type == "StopStream" {
            return SessionAction::SendText {
                text: self.handle_stop_stream(&request_id).await,
                message_name: "request response message",
            };
        }
        if request_type == "StartRecord" {
            return SessionAction::SendText {
                text: self.handle_start_record(&request_id).await,
                message_name: "request response message",
            };
        }
        if request_type == "StopRecord" {
            return SessionAction::SendText {
                text: self.handle_stop_record(&request_id).await,
                message_name: "request response message",
            };
        }

        let mut input_registry = self.input_registry.write().await;
        let response =
            crate::obsws_message::handle_request_message(request, &self.stats, &mut input_registry);
        SessionAction::SendText {
            text: response.message,
            message_name: "request response message",
        }
    }

    async fn handle_start_stream(&self, request_id: &str) -> String {
        let (output_url, stream_name, image_path, run) = {
            let mut input_registry = self.input_registry.write().await;
            let stream_service_settings = input_registry.stream_service_settings().clone();
            if stream_service_settings.stream_service_type != "rtmp_custom" {
                return crate::obsws_response_builder::build_request_response_error(
                    "StartStream",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Unsupported streamServiceType field",
                );
            }
            let Some(output_url) = stream_service_settings.server else {
                return crate::obsws_response_builder::build_request_response_error(
                    "StartStream",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Missing streamServiceSettings.server field",
                );
            };

            let scene_inputs = input_registry.list_current_program_scene_inputs();
            if scene_inputs.len() != 1 {
                return crate::obsws_response_builder::build_request_response_error(
                    "StartStream",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Exactly one enabled input is required in the current program scene",
                );
            }
            let input = &scene_inputs[0];
            let ObswsInputSettings::ImageSource(settings) = &input.input.settings else {
                return crate::obsws_response_builder::build_request_response_error(
                    "StartStream",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Only image_source is supported for StartStream",
                );
            };
            let Some(image_path) = settings.file.clone() else {
                return crate::obsws_response_builder::build_request_response_error(
                    "StartStream",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "inputSettings.file is required for image_source",
                );
            };

            let run_id = input_registry.next_stream_run_id();
            let source_processor_id = format!("obsws:stream:{run_id}:png_source");
            let encoder_processor_id = format!("obsws:stream:{run_id}:video_encoder");
            let endpoint_processor_id = format!("obsws:stream:{run_id}:rtmp_outbound");
            let source_track_id = format!("obsws:stream:{run_id}:raw_video");
            let encoded_track_id = format!("obsws:stream:{run_id}:encoded_video");
            let run = ObswsStreamRun {
                source_processor_id: source_processor_id.clone(),
                encoder_processor_id: encoder_processor_id.clone(),
                endpoint_processor_id: endpoint_processor_id.clone(),
                source_track_id: source_track_id.clone(),
                encoded_track_id: encoded_track_id.clone(),
            };
            if let Err(ActivateStreamError::AlreadyActive) =
                input_registry.activate_stream(run.clone())
            {
                return crate::obsws_response_builder::build_request_response_error(
                    "StartStream",
                    request_id,
                    REQUEST_STATUS_STREAM_RUNNING,
                    "Stream is already active",
                );
            }

            (output_url, stream_service_settings.key, image_path, run)
        };

        let start_result = self
            .start_stream_processors(&image_path, &output_url, stream_name.as_deref(), &run)
            .await;

        if let Err(e) = start_result {
            let _ = self.input_registry.write().await.deactivate_stream();
            if let Err(cleanup_error) = self.stop_stream_processors(&run).await {
                tracing::warn!(
                    "failed to cleanup stream processors after start failure: {}",
                    cleanup_error.display()
                );
            }
            return Self::build_internal_error_response(
                "StartStream",
                request_id,
                &format!("Failed to start stream: {}", e.display()),
            );
        }

        crate::obsws_response_builder::build_start_stream_response(request_id)
    }

    async fn handle_stop_stream(&self, request_id: &str) -> String {
        let run = {
            let mut input_registry = self.input_registry.write().await;
            if !input_registry.is_stream_active() {
                return crate::obsws_response_builder::build_request_response_error(
                    "StopStream",
                    request_id,
                    REQUEST_STATUS_STREAM_NOT_RUNNING,
                    "Stream is not active",
                );
            }
            input_registry
                .deactivate_stream()
                .expect("infallible: active stream must have run state")
        };
        if let Err(e) = self.stop_stream_processors(&run).await {
            return Self::build_internal_error_response(
                "StopStream",
                request_id,
                &format!("Failed to stop stream: {}", e.display()),
            );
        }
        crate::obsws_response_builder::build_stop_stream_response(request_id)
    }

    async fn handle_start_record(&self, request_id: &str) -> String {
        let (image_path, output_path, run) = {
            let mut input_registry = self.input_registry.write().await;
            let scene_inputs = input_registry.list_current_program_scene_inputs();
            if scene_inputs.len() != 1 {
                return crate::obsws_response_builder::build_request_response_error(
                    "StartRecord",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Exactly one enabled input is required in the current program scene",
                );
            }
            let input = &scene_inputs[0];
            let ObswsInputSettings::ImageSource(settings) = &input.input.settings else {
                return crate::obsws_response_builder::build_request_response_error(
                    "StartRecord",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Only image_source is supported for StartRecord",
                );
            };
            let Some(image_path) = settings.file.clone() else {
                return crate::obsws_response_builder::build_request_response_error(
                    "StartRecord",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "inputSettings.file is required for image_source",
                );
            };
            let run_id = input_registry.next_record_run_id();
            let source_processor_id = format!("obsws:record:{run_id}:png_source");
            let encoder_processor_id = format!("obsws:record:{run_id}:video_encoder");
            let writer_processor_id = format!("obsws:record:{run_id}:mp4_writer");
            let source_track_id = format!("obsws:record:{run_id}:raw_video");
            let encoded_track_id = format!("obsws:record:{run_id}:encoded_video");
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_millis();
            let output_path = input_registry
                .record_directory()
                .join(format!("obsws-record-{timestamp}.mp4"));
            let run = ObswsRecordRun {
                source_processor_id,
                encoder_processor_id,
                writer_processor_id,
                source_track_id,
                encoded_track_id,
                output_path: output_path.clone(),
            };
            if let Err(ActivateRecordError::AlreadyActive) =
                input_registry.activate_record(run.clone())
            {
                return crate::obsws_response_builder::build_request_response_error(
                    "StartRecord",
                    request_id,
                    REQUEST_STATUS_OUTPUT_RUNNING,
                    "Record is already active",
                );
            }
            (image_path, output_path, run)
        };

        if let Some(parent) = output_path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            let _ = self.input_registry.write().await.deactivate_record();
            return Self::build_internal_error_response(
                "StartRecord",
                request_id,
                &format!("Failed to create record directory: {e}"),
            );
        }

        let start_result = self
            .start_record_processors(&image_path, &output_path, &run)
            .await;
        if let Err(e) = start_result {
            let _ = self.input_registry.write().await.deactivate_record();
            if let Err(cleanup_error) = self.stop_record_processors(&run).await {
                tracing::warn!(
                    "failed to cleanup record processors after start failure: {}",
                    cleanup_error.display()
                );
            }
            return Self::build_internal_error_response(
                "StartRecord",
                request_id,
                &format!("Failed to start record: {}", e.display()),
            );
        }

        crate::obsws_response_builder::build_start_record_response(request_id)
    }

    async fn handle_stop_record(&self, request_id: &str) -> String {
        let run = {
            let mut input_registry = self.input_registry.write().await;
            if !input_registry.is_record_active() {
                return crate::obsws_response_builder::build_request_response_error(
                    "StopRecord",
                    request_id,
                    REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                    "Record is not active",
                );
            }
            input_registry
                .deactivate_record()
                .expect("infallible: active record must have run state")
        };
        if let Err(e) = self.stop_record_processors(&run).await {
            return Self::build_internal_error_response(
                "StopRecord",
                request_id,
                &format!("Failed to stop record: {}", e.display()),
            );
        }
        crate::obsws_response_builder::build_stop_record_response(
            request_id,
            &run.output_path.display().to_string(),
        )
    }

    async fn start_stream_processors(
        &self,
        image_path: &str,
        output_url: &str,
        stream_name: Option<&str>,
        run: &ObswsStreamRun,
    ) -> crate::Result<()> {
        // [NOTE]
        // ここで送る内部 JSON-RPC は常に 1 件ずつ送信して即時 await しているため、
        // 相関に id は使っておらず固定値を意図的に使用している。
        // 将来並列送信へ拡張する場合は id をユニーク化すること。
        let video_encoder_request = nojson::object(|f| {
            f.member("jsonrpc", "2.0")?;
            f.member("id", 1)?;
            f.member("method", "createVideoEncoder")?;
            f.member(
                "params",
                nojson::object(|f| {
                    f.member("inputTrackId", &run.source_track_id)?;
                    f.member("outputTrackId", &run.encoded_track_id)?;
                    f.member("codec", "H264")?;
                    f.member("bitrateBps", 2_000_000)?;
                    f.member("frameRate", 30)?;
                    f.member("processorId", &run.encoder_processor_id)
                }),
            )
        })
        .to_string();
        self.send_pipeline_rpc_request("createVideoEncoder", &video_encoder_request)
            .await?;

        let rtmp_request = nojson::object(|f| {
            f.member("jsonrpc", "2.0")?;
            f.member("id", 1)?;
            f.member("method", "createRtmpOutboundEndpoint")?;
            f.member(
                "params",
                nojson::object(|f| {
                    f.member("outputUrl", output_url)?;
                    if let Some(stream_name) = stream_name {
                        f.member("streamName", stream_name)?;
                    }
                    f.member("inputVideoTrackId", &run.encoded_track_id)?;
                    f.member("processorId", &run.endpoint_processor_id)
                }),
            )
        })
        .to_string();
        self.send_pipeline_rpc_request("createRtmpOutboundEndpoint", &rtmp_request)
            .await?;

        let png_request = nojson::object(|f| {
            f.member("jsonrpc", "2.0")?;
            f.member("id", 1)?;
            f.member("method", "createPngFileSource")?;
            f.member(
                "params",
                nojson::object(|f| {
                    f.member("path", image_path)?;
                    f.member("frameRate", 30)?;
                    f.member("outputVideoTrackId", &run.source_track_id)?;
                    f.member("processorId", &run.source_processor_id)
                }),
            )
        })
        .to_string();
        self.send_pipeline_rpc_request("createPngFileSource", &png_request)
            .await
    }

    async fn start_record_processors(
        &self,
        image_path: &str,
        output_path: &std::path::Path,
        run: &ObswsRecordRun,
    ) -> crate::Result<()> {
        // [NOTE]
        // ここで送る内部 JSON-RPC は常に 1 件ずつ送信して即時 await しているため、
        // 相関に id は使っておらず固定値を意図的に使用している。
        // 将来並列送信へ拡張する場合は id をユニーク化すること。
        let video_encoder_request = nojson::object(|f| {
            f.member("jsonrpc", "2.0")?;
            f.member("id", 1)?;
            f.member("method", "createVideoEncoder")?;
            f.member(
                "params",
                nojson::object(|f| {
                    f.member("inputTrackId", &run.source_track_id)?;
                    f.member("outputTrackId", &run.encoded_track_id)?;
                    f.member("codec", "H264")?;
                    f.member("bitrateBps", 2_000_000)?;
                    f.member("frameRate", 30)?;
                    f.member("processorId", &run.encoder_processor_id)
                }),
            )
        })
        .to_string();
        self.send_pipeline_rpc_request("createVideoEncoder", &video_encoder_request)
            .await?;

        let writer_request = nojson::object(|f| {
            f.member("jsonrpc", "2.0")?;
            f.member("id", 1)?;
            f.member("method", "createMp4Writer")?;
            f.member(
                "params",
                nojson::object(|f| {
                    f.member("outputPath", output_path.display().to_string())?;
                    f.member("inputVideoTrackId", &run.encoded_track_id)?;
                    f.member("processorId", &run.writer_processor_id)
                }),
            )
        })
        .to_string();
        self.send_pipeline_rpc_request("createMp4Writer", &writer_request)
            .await?;

        let png_request = nojson::object(|f| {
            f.member("jsonrpc", "2.0")?;
            f.member("id", 1)?;
            f.member("method", "createPngFileSource")?;
            f.member(
                "params",
                nojson::object(|f| {
                    f.member("path", image_path)?;
                    f.member("frameRate", 30)?;
                    f.member("outputVideoTrackId", &run.source_track_id)?;
                    f.member("processorId", &run.source_processor_id)
                }),
            )
        })
        .to_string();
        self.send_pipeline_rpc_request("createPngFileSource", &png_request)
            .await
    }

    async fn send_pipeline_rpc_request(
        &self,
        method: &str,
        request_text: &str,
    ) -> crate::Result<()> {
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            return Err(crate::Error::new(
                "BUG: obsws pipeline handle is not initialized",
            ));
        };
        let Some(response_json) = pipeline_handle.rpc(request_text.as_bytes()).await else {
            return Err(crate::Error::new(format!(
                "failed to run {method}: response is missing",
            )));
        };

        if let Some(error_value) = response_json.value().to_member("error")?.get() {
            let message = error_value
                .to_member("message")
                .ok()
                .and_then(|v| v.get())
                .and_then(|v| v.try_into().ok())
                .unwrap_or_else(|| "unknown rpc error".to_owned());
            return Err(crate::Error::new(format!(
                "failed to run {method}: {message}"
            )));
        }

        Ok(())
    }

    async fn stop_stream_processors(&self, run: &ObswsStreamRun) -> crate::Result<()> {
        self.stop_processors(&[
            crate::ProcessorId::new(run.endpoint_processor_id.clone()),
            crate::ProcessorId::new(run.encoder_processor_id.clone()),
            crate::ProcessorId::new(run.source_processor_id.clone()),
        ])
        .await
    }

    async fn stop_record_processors(&self, run: &ObswsRecordRun) -> crate::Result<()> {
        self.stop_processors(&[
            crate::ProcessorId::new(run.writer_processor_id.clone()),
            crate::ProcessorId::new(run.encoder_processor_id.clone()),
            crate::ProcessorId::new(run.source_processor_id.clone()),
        ])
        .await
    }

    async fn stop_processors(&self, processor_ids: &[crate::ProcessorId]) -> crate::Result<()> {
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            return Err(crate::Error::new(
                "BUG: obsws pipeline handle is not initialized",
            ));
        };

        let mut terminate_error = None;
        for processor_id in processor_ids {
            if pipeline_handle
                .terminate_processor(processor_id.clone())
                .await
                .is_err()
                && terminate_error.is_none()
            {
                terminate_error = Some(crate::Error::new(
                    "failed to terminate processor: pipeline has terminated",
                ));
            }
        }

        self.wait_processors_stopped(pipeline_handle, processor_ids, Duration::from_secs(2))
            .await?;

        if let Some(e) = terminate_error {
            return Err(e);
        }

        Ok(())
    }

    async fn wait_processors_stopped(
        &self,
        pipeline_handle: &crate::MediaPipelineHandle,
        processor_ids: &[crate::ProcessorId],
        timeout: Duration,
    ) -> crate::Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let live_processors = pipeline_handle.list_processors().await.map_err(|_| {
                crate::Error::new("failed to list processors: pipeline has terminated")
            })?;
            if processor_ids
                .iter()
                .all(|processor_id| !live_processors.iter().any(|id| id == processor_id))
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                let pending = processor_ids
                    .iter()
                    .filter(|processor_id| live_processors.iter().any(|id| id == *processor_id))
                    .map(|processor_id| processor_id.get().to_owned())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(crate::Error::new(format!(
                    "processors did not terminate in time: {pending}"
                )));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obsws_auth::build_authentication_response;
    use crate::obsws_message::RequestMessage;
    use crate::obsws_protocol::{
        OBSWS_CLOSE_ALREADY_IDENTIFIED, OBSWS_CLOSE_AUTHENTICATION_FAILED,
        OBSWS_CLOSE_NOT_IDENTIFIED, OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION,
        REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_OUTPUT_NOT_RUNNING,
    };
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn input_registry() -> Arc<RwLock<ObswsInputRegistry>> {
        Arc::new(RwLock::new(ObswsInputRegistry::new_for_test()))
    }

    #[test]
    fn on_connected_returns_hello_message_action() {
        let session = ObswsSession::new(None, input_registry(), None);
        let action = session.on_connected();
        let SessionAction::SendText { text, message_name } = action else {
            panic!("must be SendText");
        };
        assert_eq!(message_name, "hello message");
        assert!(text.contains("\"op\":0"));
    }

    #[tokio::test]
    async fn on_request_before_identify_returns_close_action() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let action = session
            .handle_request(RequestMessage {
                request_id: Some("req-1".to_owned()),
                request_type: Some("GetVersion".to_owned()),
                request_data: None,
            })
            .await;
        let SessionAction::Close { code, reason, .. } = action else {
            panic!("must be Close");
        };
        assert_eq!(code, OBSWS_CLOSE_NOT_IDENTIFIED);
        assert_eq!(reason, "identify is required");
    }

    #[tokio::test]
    async fn duplicate_identify_returns_already_identified_close() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let first = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
            .await;
        assert!(first.is_ok());

        let second = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
            .await;
        let action = second.expect("second identify must return action");
        let SessionAction::Close { code, reason, .. } = action else {
            panic!("must be Close");
        };
        assert_eq!(code, OBSWS_CLOSE_ALREADY_IDENTIFIED);
        assert_eq!(reason, "already identified");
    }

    #[tokio::test]
    async fn unsupported_rpc_version_returns_close_action() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":2}}"#)
            .await
            .expect("identify must be parsed");
        let SessionAction::Close { code, reason, .. } = action else {
            panic!("must be Close");
        };
        assert_eq!(code, OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION);
        assert_eq!(reason, "unsupported rpc version");
    }

    #[tokio::test]
    async fn invalid_authentication_returns_close_action() {
        let auth = ObswsAuthentication {
            salt: "test-salt".to_owned(),
            challenge: "test-challenge".to_owned(),
            expected_response: build_authentication_response(
                "test-password",
                "test-salt",
                "test-challenge",
            ),
        };
        let mut session = ObswsSession::new(Some(auth), input_registry(), None);
        let action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"authentication":"invalid"}}"#)
            .await
            .expect("identify must be parsed");
        let SessionAction::Close { code, reason, .. } = action else {
            panic!("must be Close");
        };
        assert_eq!(code, OBSWS_CLOSE_AUTHENTICATION_FAILED);
        assert_eq!(reason, "authentication failed");
    }

    #[tokio::test]
    async fn stop_record_when_inactive_returns_error_response() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));

        let action = session
            .handle_request(RequestMessage {
                request_id: Some("req-stop-record".to_owned()),
                request_type: Some("StopRecord".to_owned()),
                request_data: None,
            })
            .await;
        let SessionAction::SendText { text, .. } = action else {
            panic!("must be SendText");
        };
        let json = nojson::RawJson::parse(&text).expect("response must be valid json");
        let status = json
            .value()
            .to_member("d")
            .expect("d access must succeed")
            .required()
            .expect("d must exist")
            .to_member("requestStatus")
            .expect("requestStatus access must succeed")
            .required()
            .expect("requestStatus must exist");
        let result: bool = status
            .to_member("result")
            .expect("result access must succeed")
            .required()
            .expect("result must exist")
            .try_into()
            .expect("result must be bool");
        let code: i64 = status
            .to_member("code")
            .expect("code access must succeed")
            .required()
            .expect("code must exist")
            .try_into()
            .expect("code must be i64");
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_OUTPUT_NOT_RUNNING);
    }

    #[tokio::test]
    async fn start_record_without_image_input_returns_error_response() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));

        let action = session
            .handle_request(RequestMessage {
                request_id: Some("req-start-record".to_owned()),
                request_type: Some("StartRecord".to_owned()),
                request_data: None,
            })
            .await;
        let SessionAction::SendText { text, .. } = action else {
            panic!("must be SendText");
        };
        let json = nojson::RawJson::parse(&text).expect("response must be valid json");
        let status = json
            .value()
            .to_member("d")
            .expect("d access must succeed")
            .required()
            .expect("d must exist")
            .to_member("requestStatus")
            .expect("requestStatus access must succeed")
            .required()
            .expect("requestStatus must exist");
        let result: bool = status
            .to_member("result")
            .expect("result access must succeed")
            .required()
            .expect("result must exist")
            .try_into()
            .expect("result must be bool");
        let code: i64 = status
            .to_member("code")
            .expect("code access must succeed")
            .required()
            .expect("code must exist")
            .try_into()
            .expect("code must be i64");
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
    }
}
